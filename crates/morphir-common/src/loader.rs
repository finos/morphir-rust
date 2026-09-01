use crate::remote::{RemoteSource, RemoteSourceResolver, ResolveOptions};
use crate::vfs::{OsVfs, Vfs};
use anyhow::{Context, Result};
use indexmap::IndexMap;
use morphir_core::ir::{classic, v4};
use morphir_core::naming::PackageName;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum LoadedDistribution {
    V4(v4::IRFile),
    Classic(classic::Distribution),
}

/// Load distribution from a source string (local path or remote source).
///
/// This is a convenience function that:
/// 1. Parses the source string into a RemoteSource
/// 2. Resolves it to a local path (downloading if necessary)
/// 3. Loads the distribution from the local path
///
/// # Arguments
/// * `source` - A source string (local path, URL, or shorthand like `github:owner/repo`)
///
/// # Examples
/// ```ignore
/// // Local file
/// let dist = load_distribution_from_source("./morphir-ir.json")?;
///
/// // Remote URL
/// let dist = load_distribution_from_source("https://example.com/morphir-ir.json")?;
///
/// // GitHub shorthand
/// let dist = load_distribution_from_source("github:finos/morphir-examples/examples/basic")?;
/// ```
pub fn load_distribution_from_source(source: &str) -> Result<LoadedDistribution> {
    load_distribution_from_source_with_options(source, &ResolveOptions::new())
}

/// Load distribution from a source string with custom resolve options.
pub fn load_distribution_from_source_with_options(
    source: &str,
    options: &ResolveOptions,
) -> Result<LoadedDistribution> {
    let remote_source =
        RemoteSource::parse(source).map_err(|e| anyhow::anyhow!("Invalid source: {}", e))?;

    let local_path = if remote_source.is_local() {
        std::path::PathBuf::from(source)
    } else {
        let mut resolver = RemoteSourceResolver::with_defaults()
            .map_err(|e| anyhow::anyhow!("Failed to create source resolver: {}", e))?;

        resolver
            .resolve(&remote_source, options)
            .map_err(|e| anyhow::anyhow!("Failed to resolve source: {}", e))?
    };

    let vfs = OsVfs;
    load_distribution(&vfs, &local_path)
}

pub fn load_distribution(vfs: &impl Vfs, path: &Path) -> Result<LoadedDistribution> {
    if vfs.is_dir(path) {
        return load_v4_from_dir(vfs, path);
    }

    let content = vfs.read_to_string(path)?;

    if let Ok(ir_file) = serde_json::from_str::<v4::IRFile>(&content) {
        // Check if it's a V4 format based on format_version
        let is_v4 = match &ir_file.format_version {
            v4::FormatVersion::Integer(n) => *n >= 4,
            v4::FormatVersion::String(s) => s.starts_with("4"),
        };

        if is_v4 {
            return Ok(LoadedDistribution::V4(ir_file));
        }
    }

    let classic_dist = deserialize_classic(&content)
        .context("Failed to parse distribution as either V4 or Classic IR")?;

    Ok(LoadedDistribution::Classic(classic_dist))
}

fn deserialize_classic(content: &str) -> serde_json::Result<classic::Distribution> {
    match serde_json::from_str(content) {
        Ok(distribution) => Ok(distribution),
        Err(original_error) => {
            let Ok(mut value) = serde_json::from_str::<serde_json::Value>(content) else {
                return Err(original_error);
            };
            if value.get("formatVersion") != Some(&serde_json::Value::from("3.0.0")) {
                return Err(original_error);
            }
            value["formatVersion"] = serde_json::Value::from(3);
            serde_json::from_value(value)
        }
    }
}

fn load_v4_from_dir(vfs: &impl Vfs, path: &Path) -> Result<LoadedDistribution> {
    // Read morphir.json from the directory root to get package name
    let morphir_json_path = path.join("morphir.json");
    let package_name = if vfs.exists(&morphir_json_path) {
        let content = vfs.read_to_string(&morphir_json_path)?;
        let config: serde_json::Value =
            serde_json::from_str(&content).context("Failed to parse morphir.json")?;
        config
            .get("name")
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown-package".to_string())
    } else {
        "unknown-package".to_string()
    };

    // Scan for module JSON files in src/ directory
    let src_path = path.join("src");
    let mut modules: IndexMap<String, v4::AccessControlled<v4::ModuleDefinition>> = IndexMap::new();

    if vfs.is_dir(&src_path) {
        let json_files = collect_descendant_files(vfs, &src_path)?;

        for file_path in json_files.into_iter().filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        }) {
            // Skip Package.json files (metadata only)
            if file_path
                .file_name()
                .map(|n| n == "Package.json")
                .unwrap_or(false)
            {
                continue;
            }

            let Ok(relative) = file_path.strip_prefix(&src_path) else {
                continue;
            };
            let module_name = relative
                .with_extension("")
                .components()
                .map(|component| component.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");

            let content = vfs.read_to_string(&file_path).with_context(|| {
                format!("Failed to read module document {}", file_path.display())
            })?;
            let module_def = serde_json::from_str(&content).with_context(|| {
                format!("Failed to parse module document {}", file_path.display())
            })?;
            modules.insert(module_name, module_def);
        }
    }

    let ir_file = v4::IRFile {
        format_version: v4::FormatVersion::default(),
        distribution: v4::Distribution::Library(v4::LibraryContent {
            package_name: PackageName::parse(&package_name),
            dependencies: IndexMap::new(),
            def: v4::PackageDefinition { modules },
        }),
    };

    Ok(LoadedDistribution::V4(ir_file))
}

fn collect_descendant_files(vfs: &impl Vfs, root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let root_identity = vfs.canonicalize(root)?;
    let mut pending = vec![root.to_path_buf()];
    let mut visited = BTreeSet::new();
    let mut files = BTreeSet::new();
    while let Some(directory) = pending.pop() {
        let directory_identity = vfs.canonicalize(&directory)?;
        if !directory_identity.starts_with(&root_identity) || !visited.insert(directory_identity) {
            continue;
        }
        let mut entries = vfs.list_dir(&directory)?;
        entries.sort();
        entries.dedup();
        for entry in entries.into_iter().rev() {
            if vfs.is_symlink(&entry)? {
                continue;
            }
            let entry_identity = vfs.canonicalize(&entry)?;
            if !entry_identity.starts_with(&root_identity) {
                continue;
            }
            let metadata = vfs.metadata(&entry)?;
            if metadata.is_dir {
                pending.push(entry);
            } else if metadata.is_file {
                files.insert(entry);
            }
        }
    }
    Ok(files.into_iter().collect())
}

/// Load IR from a path and return as JSON value
/// This is a convenience function for commands that need IR as JSON
pub fn load_ir(path: &Path) -> Result<serde_json::Value> {
    let vfs = OsVfs;
    let distribution = load_distribution(&vfs, path)?;

    match distribution {
        LoadedDistribution::V4(ir_file) => {
            serde_json::to_value(&ir_file).context("Failed to serialize V4 IR to JSON")
        }
        LoadedDistribution::Classic(classic_dist) => {
            serde_json::to_value(&classic_dist).context("Failed to serialize Classic IR to JSON")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LoadedDistribution, load_distribution};
    use crate::vfs::{FileMetadata, MemoryVfs, OsVfs, Vfs};
    use indexmap::IndexMap;
    use morphir_core::ir::v4::{
        Access, AccessControlled, Distribution, Documented, Literal, ModuleDefinition, Type,
        TypeAttributes, TypeDefinition, Value, ValueAttributes, ValueDefinition,
    };
    use std::io;
    use std::path::{Path, PathBuf};

    struct FailingListVfs;

    impl Vfs for FailingListVfs {
        fn read_to_string(&self, _path: &Path) -> io::Result<String> {
            unreachable!("the fixture has no readable files")
        }

        fn write_from_string(&self, _path: &Path, _content: &str) -> io::Result<()> {
            unreachable!("the fixture is read-only")
        }

        fn exists(&self, _path: &Path) -> bool {
            false
        }

        fn is_dir(&self, path: &Path) -> bool {
            path == Path::new("/document-tree") || path == Path::new("/document-tree/src")
        }

        fn list_dir(&self, _path: &Path) -> io::Result<Vec<PathBuf>> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "sentinel directory listing failure",
            ))
        }

        fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
            unreachable!("the fixture is read-only")
        }

        fn glob(&self, _pattern: &str) -> io::Result<Vec<PathBuf>> {
            unreachable!("the loader does not use glob discovery")
        }

        fn remove(&self, _path: &Path) -> io::Result<()> {
            unreachable!("the fixture is read-only")
        }

        fn copy(&self, _from: &Path, _to: &Path) -> io::Result<()> {
            unreachable!("the fixture is read-only")
        }

        fn metadata(&self, _path: &Path) -> io::Result<FileMetadata> {
            unreachable!("the current traversal does not query metadata")
        }
    }

    fn empty_module() -> AccessControlled<ModuleDefinition> {
        AccessControlled {
            access: Access::Public,
            value: ModuleDefinition {
                types: IndexMap::new(),
                values: IndexMap::new(),
                doc: None,
            },
        }
    }

    fn module_with_type_alias() -> AccessControlled<ModuleDefinition> {
        AccessControlled {
            access: Access::Public,
            value: ModuleDefinition {
                types: IndexMap::from([(
                    "world-alias".into(),
                    AccessControlled {
                        access: Access::Public,
                        value: Documented::new(
                            None,
                            TypeDefinition::TypeAliasDefinition {
                                type_params: vec![],
                                type_expr: Type::unit(TypeAttributes::default()),
                            },
                        ),
                    },
                )]),
                values: IndexMap::new(),
                doc: None,
            },
        }
    }

    fn module_with_hello_value() -> AccessControlled<ModuleDefinition> {
        AccessControlled {
            access: Access::Public,
            value: ModuleDefinition {
                types: IndexMap::new(),
                values: IndexMap::from([(
                    "hello".into(),
                    AccessControlled {
                        access: Access::Public,
                        value: Documented::new(
                            None,
                            ValueDefinition::new(
                                vec![],
                                Type::unit(TypeAttributes::default()),
                                Value::literal(
                                    ValueAttributes::default(),
                                    Literal::String("world".into()),
                                ),
                            ),
                        ),
                    },
                )]),
                doc: None,
            },
        }
    }

    #[test]
    fn absolute_v4_document_tree_loads_modules_independent_of_the_process_directory() {
        let vfs = MemoryVfs::new();
        let root = Path::new("/workspace/document-tree");
        vfs.write_from_string(&root.join("morphir.json"), r#"{"name":"example/hello"}"#)
            .unwrap();
        let zeta = module_with_type_alias();
        vfs.write_from_string(
            &root.join("src/zeta.json"),
            &serde_json::to_string(&zeta).unwrap(),
        )
        .unwrap();
        let greeting = module_with_hello_value();
        vfs.write_from_string(
            &root.join("src/example/greeting.json"),
            &serde_json::to_string(&greeting).unwrap(),
        )
        .unwrap();

        let LoadedDistribution::V4(ir) = load_distribution(&vfs, root).unwrap() else {
            panic!("expected a v4 distribution");
        };
        let Distribution::Library(library) = ir.distribution else {
            panic!("expected a v4 library");
        };

        assert_eq!(
            library
                .def
                .modules
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["example/greeting", "zeta"]
        );
        assert_eq!(library.def.modules["example/greeting"], greeting);
        assert_eq!(library.def.modules["zeta"], zeta);
    }

    #[test]
    fn v4_document_tree_propagates_directory_listing_failures() {
        let error = load_distribution(&FailingListVfs, Path::new("/document-tree")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("sentinel directory listing failure"),
            "unexpected loader error: {error:#}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn v4_document_tree_does_not_follow_directory_symlinks_outside_the_source_root() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("document-tree");
        let src = root.join("src");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("morphir.json"), r#"{"name":"example/hello"}"#).unwrap();
        std::fs::write(
            src.join("local.json"),
            serde_json::to_string(&empty_module()).unwrap(),
        )
        .unwrap();
        std::fs::write(
            outside.join("external.json"),
            serde_json::to_string(&module_with_hello_value()).unwrap(),
        )
        .unwrap();
        symlink(&outside, src.join("outside")).unwrap();

        let LoadedDistribution::V4(ir) = load_distribution(&OsVfs, &root).unwrap() else {
            panic!("expected a v4 distribution");
        };
        let Distribution::Library(library) = ir.distribution else {
            panic!("expected a v4 library");
        };

        assert_eq!(
            library
                .def
                .modules
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["local"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn v4_document_tree_does_not_follow_a_directory_symlink_cycle() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("document-tree");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(root.join("morphir.json"), r#"{"name":"example/hello"}"#).unwrap();
        std::fs::write(
            src.join("local.json"),
            serde_json::to_string(&empty_module()).unwrap(),
        )
        .unwrap();
        symlink(&src, src.join("cycle")).unwrap();

        let LoadedDistribution::V4(ir) = load_distribution(&OsVfs, &root).unwrap() else {
            panic!("expected a v4 distribution");
        };
        let Distribution::Library(library) = ir.distribution else {
            panic!("expected a v4 library");
        };

        assert_eq!(
            library
                .def
                .modules
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["local"]
        );
    }
}
