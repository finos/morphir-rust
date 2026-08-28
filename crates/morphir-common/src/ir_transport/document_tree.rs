use std::io::Write;

use anyhow::{Context, Result, bail};
use indexmap::IndexMap;
use morphir_core::ir::v4::{
    Access, AccessControlled, ApplicationContent, Dependencies, Distribution, Documentation,
    Documented, EntryPoints, FormatVersion, IRFile, LibraryContent, ModuleDefinition,
    ModuleSpecification, PackageDefinition, PackageSpecification, SpecsContent, TypeDefinition,
    TypeSpecification, ValueDefinition, ValueSpecification,
};
use morphir_core::naming::PackageName;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use vfs::VfsPath;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum DistributionKind {
    Library,
    Specs,
    Application,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DistributionManifest {
    format_version: FormatVersion,
    distribution: DistributionKind,
    package: PackageName,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    dependencies: Dependencies,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    entry_points: EntryPoints,
    layout: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModuleManifest {
    format_version: FormatVersion,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    access: Option<Access>,
    #[serde(skip_serializing_if = "Option::is_none")]
    doc: Option<Documentation>,
    #[serde(default)]
    types: Vec<String>,
    #[serde(default)]
    values: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DefinitionPayload<T> {
    access: Access,
    #[serde(flatten)]
    value: T,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DefinitionFile<T> {
    format_version: FormatVersion,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    doc: Option<Documentation>,
    def: DefinitionPayload<T>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpecificationFile<T> {
    format_version: FormatVersion,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    doc: Option<Documentation>,
    spec: T,
}

fn write_json(path: &VfsPath, value: &impl Serialize) -> Result<()> {
    let mut writer = path
        .create_file()
        .with_context(|| format!("failed to create {}", path.as_str()))?;
    serde_json::to_writer_pretty(&mut writer, value)
        .with_context(|| format!("failed to serialize {}", path.as_str()))?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

fn read_json<T: DeserializeOwned>(path: &VfsPath) -> Result<T> {
    let reader = path
        .open_file()
        .with_context(|| format!("failed to open {}", path.as_str()))?;
    serde_json::from_reader(reader).with_context(|| format!("failed to parse {}", path.as_str()))
}

fn package_root(root: &VfsPath, package: &PackageName) -> Result<VfsPath> {
    root.join(&format!("pkg/{package}"))
        .context("invalid package path")
}

fn module_root(root: &VfsPath, package: &PackageName, module: &str) -> Result<VfsPath> {
    package_root(root, package)?
        .join(module)
        .context("invalid module path")
}

fn write_definition_module(
    root: &VfsPath,
    package: &PackageName,
    path: &str,
    module: &AccessControlled<ModuleDefinition>,
) -> Result<()> {
    let directory = module_root(root, package, path)?;
    directory.create_dir_all()?;

    for (name, definition) in &module.value.types {
        write_json(
            &directory.join(&format!("{name}.type.json"))?,
            &DefinitionFile {
                format_version: FormatVersion::Integer(4),
                name: name.clone(),
                doc: definition.value.doc.clone(),
                def: DefinitionPayload {
                    access: definition.access.clone(),
                    value: definition.value.value.clone(),
                },
            },
        )?;
    }
    for (name, definition) in &module.value.values {
        write_json(
            &directory.join(&format!("{name}.value.json"))?,
            &DefinitionFile {
                format_version: FormatVersion::Integer(4),
                name: name.clone(),
                doc: definition.value.doc.clone(),
                def: DefinitionPayload {
                    access: definition.access.clone(),
                    value: definition.value.value.clone(),
                },
            },
        )?;
    }

    write_json(
        &directory.join("module.json")?,
        &ModuleManifest {
            format_version: FormatVersion::Integer(4),
            path: path.to_owned(),
            access: Some(module.access.clone()),
            doc: module.value.doc.clone(),
            types: module.value.types.keys().cloned().collect(),
            values: module.value.values.keys().cloned().collect(),
        },
    )
}

fn write_specification_module(
    root: &VfsPath,
    package: &PackageName,
    path: &str,
    module: &ModuleSpecification,
) -> Result<()> {
    let directory = module_root(root, package, path)?;
    directory.create_dir_all()?;

    for (name, specification) in &module.types {
        write_json(
            &directory.join(&format!("{name}.type.json"))?,
            &SpecificationFile {
                format_version: FormatVersion::Integer(4),
                name: name.clone(),
                doc: specification.doc.clone(),
                spec: specification.value.clone(),
            },
        )?;
    }
    for (name, specification) in &module.values {
        write_json(
            &directory.join(&format!("{name}.value.json"))?,
            &SpecificationFile {
                format_version: FormatVersion::Integer(4),
                name: name.clone(),
                doc: specification.doc.clone(),
                spec: specification.value.clone(),
            },
        )?;
    }

    write_json(
        &directory.join("module.json")?,
        &ModuleManifest {
            format_version: FormatVersion::Integer(4),
            path: path.to_owned(),
            access: None,
            doc: module.doc.clone(),
            types: module.types.keys().cloned().collect(),
            values: module.values.keys().cloned().collect(),
        },
    )
}

/// Write a concrete v4 distribution as a granular document tree.
///
/// The distribution manifest is published last, so an incomplete tree is never
/// advertised as complete on backends without an atomic directory move.
pub fn write_document_tree(root: &VfsPath, ir: &IRFile) -> Result<()> {
    root.create_dir_all()?;
    let manifest = match &ir.distribution {
        Distribution::Library(content) => {
            for (path, module) in &content.def.modules {
                write_definition_module(root, &content.package_name, path, module)?;
            }
            DistributionManifest {
                format_version: ir.format_version.clone(),
                distribution: DistributionKind::Library,
                package: content.package_name.clone(),
                dependencies: content.dependencies.clone(),
                entry_points: IndexMap::new(),
                layout: "VfsMode".to_owned(),
            }
        }
        Distribution::Specs(content) => {
            for (path, module) in &content.spec.modules {
                write_specification_module(root, &content.package_name, path, module)?;
            }
            DistributionManifest {
                format_version: ir.format_version.clone(),
                distribution: DistributionKind::Specs,
                package: content.package_name.clone(),
                dependencies: content.dependencies.clone(),
                entry_points: IndexMap::new(),
                layout: "VfsMode".to_owned(),
            }
        }
        Distribution::Application(content) => {
            for (path, module) in &content.def.modules {
                write_definition_module(root, &content.package_name, path, module)?;
            }
            DistributionManifest {
                format_version: ir.format_version.clone(),
                distribution: DistributionKind::Application,
                package: content.package_name.clone(),
                dependencies: content.dependencies.clone(),
                entry_points: content.entry_points.clone(),
                layout: "VfsMode".to_owned(),
            }
        }
    };
    write_json(&root.join("manifest.json")?, &manifest)
}

fn module_manifests(root: &VfsPath) -> Result<Vec<(VfsPath, ModuleManifest)>> {
    root.walk_dir()?
        .filter_map(|entry| match entry {
            Ok(path) if path.filename() == "module.json" => Some(
                read_json(&path)
                    .map(|manifest| (path.parent(), manifest))
                    .with_context(|| format!("invalid module manifest {}", path.as_str())),
            ),
            Ok(_) => None,
            Err(error) => Some(Err(error.into())),
        })
        .collect()
}

fn checked_name(expected: &str, actual: &str, path: &VfsPath) -> Result<()> {
    if expected != actual {
        bail!(
            "definition name '{}' in {} does not match manifest name '{}'",
            actual,
            path.as_str(),
            expected
        );
    }
    Ok(())
}

fn read_definition_module(
    directory: &VfsPath,
    manifest: ModuleManifest,
) -> Result<(String, AccessControlled<ModuleDefinition>)> {
    let mut types = IndexMap::new();
    for name in &manifest.types {
        let path = directory.join(&format!("{name}.type.json"))?;
        let file: DefinitionFile<TypeDefinition> = read_json(&path)?;
        checked_name(name, &file.name, &path)?;
        types.insert(
            name.clone(),
            AccessControlled {
                access: file.def.access,
                value: Documented::new(file.doc, file.def.value),
            },
        );
    }
    let mut values = IndexMap::new();
    for name in &manifest.values {
        let path = directory.join(&format!("{name}.value.json"))?;
        let file: DefinitionFile<ValueDefinition> = read_json(&path)?;
        checked_name(name, &file.name, &path)?;
        values.insert(
            name.clone(),
            AccessControlled {
                access: file.def.access,
                value: Documented::new(file.doc, file.def.value),
            },
        );
    }
    Ok((
        manifest.path,
        AccessControlled {
            access: manifest.access.unwrap_or(Access::Public),
            value: ModuleDefinition {
                types,
                values,
                doc: manifest.doc,
            },
        },
    ))
}

fn read_specification_module(
    directory: &VfsPath,
    manifest: ModuleManifest,
) -> Result<(String, ModuleSpecification)> {
    let mut types = IndexMap::new();
    for name in &manifest.types {
        let path = directory.join(&format!("{name}.type.json"))?;
        let file: SpecificationFile<TypeSpecification> = read_json(&path)?;
        checked_name(name, &file.name, &path)?;
        types.insert(name.clone(), Documented::new(file.doc, file.spec));
    }
    let mut values = IndexMap::new();
    for name in &manifest.values {
        let path = directory.join(&format!("{name}.value.json"))?;
        let file: SpecificationFile<ValueSpecification> = read_json(&path)?;
        checked_name(name, &file.name, &path)?;
        values.insert(name.clone(), Documented::new(file.doc, file.spec));
    }
    Ok((
        manifest.path,
        ModuleSpecification {
            types,
            values,
            doc: manifest.doc,
        },
    ))
}

/// Read a granular v4 document tree into the concrete v4 IR model.
pub fn read_document_tree(root: &VfsPath) -> Result<IRFile> {
    let manifest: DistributionManifest = read_json(&root.join("manifest.json")?)?;
    let modules = module_manifests(root)?;
    let distribution = match manifest.distribution {
        DistributionKind::Library => Distribution::Library(LibraryContent {
            package_name: manifest.package,
            dependencies: manifest.dependencies,
            def: PackageDefinition {
                modules: modules
                    .into_iter()
                    .map(|(directory, module)| read_definition_module(&directory, module))
                    .collect::<Result<_>>()?,
            },
        }),
        DistributionKind::Specs => Distribution::Specs(SpecsContent {
            package_name: manifest.package,
            dependencies: manifest.dependencies,
            spec: PackageSpecification {
                modules: modules
                    .into_iter()
                    .map(|(directory, module)| read_specification_module(&directory, module))
                    .collect::<Result<_>>()?,
            },
        }),
        DistributionKind::Application => Distribution::Application(ApplicationContent {
            package_name: manifest.package,
            dependencies: manifest.dependencies,
            def: PackageDefinition {
                modules: modules
                    .into_iter()
                    .map(|(directory, module)| read_definition_module(&directory, module))
                    .collect::<Result<_>>()?,
            },
            entry_points: manifest.entry_points,
        }),
    };
    Ok(IRFile {
        format_version: manifest.format_version,
        distribution,
    })
}
