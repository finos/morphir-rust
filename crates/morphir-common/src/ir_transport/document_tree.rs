//! Profile-neutral v4 document-tree transport.

use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};

use indexmap::IndexMap;
use morphir_core::ir::v4::{
    Access, AccessControlled, Dependencies, Documentation, Documented, EntryPoints, FormatVersion,
    IRFile, ModuleDefinition, ModuleSpecification, TypeDefinition, TypeSpecification,
    ValueDefinition, ValueSpecification,
};
use morphir_core::naming::PackageName;
use morphir_core::traversal::{
    CursorSegment, DependencyEvent, DistributionHeader, IrCursor, ModuleEvent, SemanticEvent,
    SemanticEventKind,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use vfs::VfsPath;

use super::semantic::{self, SemanticFile};
use super::yaml;
use super::{
    CodecOptions, EventSink, EventSource, FormatId, IR_RECURSION_STACK_BYTES, IrVersion, Layout,
    Stage, TransportDiagnostic,
};

/// Serialization-independent identity of one document-tree file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LogicalDocument {
    /// Distribution manifest, published last.
    Manifest,
    /// Module manifest.
    Module {
        package: PackageName,
        module: String,
    },
    /// Type definition or specification.
    Type {
        package: PackageName,
        module: String,
        name: String,
    },
    /// Value definition or specification.
    Value {
        package: PackageName,
        module: String,
        name: String,
    },
}

impl LogicalDocument {
    /// Map this identity to its canonical path relative to a tree root.
    pub fn relative_path(&self, format: &FormatId) -> Result<String, TransportDiagnostic> {
        let profile = TreeProfile::new(format.clone())?;
        let extension = profile.extension();
        Ok(match self {
            Self::Manifest => format!("manifest.{extension}"),
            Self::Module { package, module } => {
                format!("pkg/{package}/{module}/module.{extension}")
            }
            Self::Type {
                package,
                module,
                name,
            } => format!("pkg/{package}/{module}/{name}.type.{extension}"),
            Self::Value {
                package,
                module,
                name,
            } => format!("pkg/{package}/{module}/{name}.value.{extension}"),
        })
    }

    fn path(&self, root: &VfsPath, profile: &TreeProfile) -> Result<VfsPath, TransportDiagnostic> {
        let relative = self.relative_path(&profile.format)?;
        root.join(relative).map_err(|error| {
            tree_error(
                "morphir::ir::document_tree::invalid_path",
                Stage::Publication,
                error.to_string(),
                "use valid Morphir package, module, and definition names",
            )
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum DistributionKind {
    Library,
    Specs,
    Application,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
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

#[derive(Clone)]
struct TreeProfile {
    format: FormatId,
}

impl TreeProfile {
    fn new(format: FormatId) -> Result<Self, TransportDiagnostic> {
        if format != FormatId::json() && format != FormatId::yaml() {
            return Err(tree_error(
                "morphir::ir::document_tree::unsupported_format",
                Stage::Detection,
                format!("document trees do not have a '{format}' profile"),
                "select json or yaml, or register a document-tree profile",
            ));
        }
        Ok(Self { format })
    }

    fn extension(&self) -> &'static str {
        if self.format == FormatId::json() {
            "json"
        } else {
            "yaml"
        }
    }

    fn read<T: DeserializeOwned>(&self, path: &VfsPath) -> Result<T, TransportDiagnostic> {
        let mut reader = path
            .open_file()
            .map_err(|error| io_error("open", path, Stage::Syntax, error))?;
        if self.format == FormatId::json() {
            stacker::grow(IR_RECURSION_STACK_BYTES, || {
                serde_json::from_reader(&mut reader)
            })
            .map_err(|error| {
                tree_error(
                    "morphir::ir::json::invalid_syntax",
                    Stage::Syntax,
                    format!("failed to parse {}: {error}", path.as_str()),
                    "correct the JSON document-tree file",
                )
            })
        } else {
            let mut input = Vec::new();
            reader
                .read_to_end(&mut input)
                .map_err(|error| io_error("read", path, Stage::Syntax, error))?;
            yaml::decode_document(&input)
        }
    }

    fn write<T: Serialize>(&self, path: &VfsPath, value: &T) -> Result<(), TransportDiagnostic> {
        let bytes = if self.format == FormatId::json() {
            let mut output = Vec::new();
            stacker::grow(IR_RECURSION_STACK_BYTES, || {
                serde_json::to_writer_pretty(&mut output, value)
            })
            .map_err(|error| {
                tree_error(
                    "morphir::ir::json::encode_failed",
                    Stage::Encoding,
                    format!("failed to encode {}: {error}", path.as_str()),
                    "verify that the logical document is representable as JSON",
                )
            })?;
            output.push(b'\n');
            output
        } else {
            yaml::encode_document(value)?
        };
        let parent = path.parent();
        parent
            .create_dir_all()
            .map_err(|error| io_error("create", &parent, Stage::Publication, error))?;
        let mut writer = path
            .create_file()
            .map_err(|error| io_error("create", path, Stage::Publication, error))?;
        writer
            .write_all(&bytes)
            .map_err(|error| io_error("write", path, Stage::Publication, error))?;
        writer
            .flush()
            .map_err(|error| io_error("flush", path, Stage::Publication, error))
    }
}

fn tree_error(
    code: &'static str,
    stage: Stage,
    message: impl Into<String>,
    guidance: &'static str,
) -> TransportDiagnostic {
    TransportDiagnostic::error(code, stage, IrCursor::root(), message).with_guidance(guidance)
}

fn io_error(
    operation: &'static str,
    path: &VfsPath,
    stage: Stage,
    error: impl std::fmt::Display,
) -> TransportDiagnostic {
    tree_error(
        "morphir::ir::document_tree::io_failed",
        stage,
        format!("failed to {operation} {}: {error}", path.as_str()),
        "verify the VFS path, permissions, and publication capability",
    )
}

fn validate_options(options: &CodecOptions) -> Result<TreeProfile, TransportDiagnostic> {
    if options.version() != IrVersion::V4 {
        return Err(tree_error(
            "morphir::ir::document_tree::version_unsupported",
            Stage::Detection,
            "the granular document-tree layout is defined for IR v4",
            "migrate the event stream to v4 before selecting document-tree output",
        ));
    }
    if options.layout() != Layout::DocumentTree {
        return Err(tree_error(
            "morphir::ir::document_tree::layout_mismatch",
            Stage::Detection,
            "document-tree transport received single-file codec options",
            "select the document-tree layout",
        ));
    }
    TreeProfile::new(options.format().clone())
}

fn package_root(root: &VfsPath, package: &PackageName) -> Result<VfsPath, TransportDiagnostic> {
    root.join(format!("pkg/{package}")).map_err(|error| {
        tree_error(
            "morphir::ir::document_tree::invalid_package_path",
            Stage::Detection,
            error.to_string(),
            "use a valid v4 package name",
        )
    })
}

fn checked_name(expected: &str, actual: &str, path: &VfsPath) -> Result<(), TransportDiagnostic> {
    if expected != actual {
        return Err(tree_error(
            "morphir::ir::document_tree::name_mismatch",
            Stage::Normalization,
            format!(
                "definition name '{actual}' in {} does not match manifest name '{expected}'",
                path.as_str()
            ),
            "make the embedded definition name match the module manifest",
        ));
    }
    Ok(())
}

fn write_definition_module(
    root: &VfsPath,
    profile: &TreeProfile,
    package: &PackageName,
    path: &str,
    module: &AccessControlled<ModuleDefinition>,
) -> Result<(), TransportDiagnostic> {
    for (name, definition) in &module.value.types {
        let document = LogicalDocument::Type {
            package: package.clone(),
            module: path.to_owned(),
            name: name.clone(),
        };
        profile.write(
            &document.path(root, profile)?,
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
        let document = LogicalDocument::Value {
            package: package.clone(),
            module: path.to_owned(),
            name: name.clone(),
        };
        profile.write(
            &document.path(root, profile)?,
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
    let document = LogicalDocument::Module {
        package: package.clone(),
        module: path.to_owned(),
    };
    profile.write(
        &document.path(root, profile)?,
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
    profile: &TreeProfile,
    package: &PackageName,
    path: &str,
    module: &ModuleSpecification,
) -> Result<(), TransportDiagnostic> {
    for (name, specification) in &module.types {
        let document = LogicalDocument::Type {
            package: package.clone(),
            module: path.to_owned(),
            name: name.clone(),
        };
        profile.write(
            &document.path(root, profile)?,
            &SpecificationFile {
                format_version: FormatVersion::Integer(4),
                name: name.clone(),
                doc: specification.doc.clone(),
                spec: specification.value.clone(),
            },
        )?;
    }
    for (name, specification) in &module.values {
        let document = LogicalDocument::Value {
            package: package.clone(),
            module: path.to_owned(),
            name: name.clone(),
        };
        profile.write(
            &document.path(root, profile)?,
            &SpecificationFile {
                format_version: FormatVersion::Integer(4),
                name: name.clone(),
                doc: specification.doc.clone(),
                spec: specification.value.clone(),
            },
        )?;
    }
    let document = LogicalDocument::Module {
        package: package.clone(),
        module: path.to_owned(),
    };
    profile.write(
        &document.path(root, profile)?,
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

fn read_definition_module(
    root: &VfsPath,
    profile: &TreeProfile,
    package: &PackageName,
    manifest: ModuleManifest,
) -> Result<(String, AccessControlled<ModuleDefinition>), TransportDiagnostic> {
    let mut types = IndexMap::new();
    for name in &manifest.types {
        let document = LogicalDocument::Type {
            package: package.clone(),
            module: manifest.path.clone(),
            name: name.clone(),
        };
        let path = document.path(root, profile)?;
        let file: DefinitionFile<TypeDefinition> = profile.read(&path)?;
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
        let document = LogicalDocument::Value {
            package: package.clone(),
            module: manifest.path.clone(),
            name: name.clone(),
        };
        let path = document.path(root, profile)?;
        let file: DefinitionFile<ValueDefinition> = profile.read(&path)?;
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
    root: &VfsPath,
    profile: &TreeProfile,
    package: &PackageName,
    manifest: ModuleManifest,
) -> Result<(String, ModuleSpecification), TransportDiagnostic> {
    let mut types = IndexMap::new();
    for name in &manifest.types {
        let document = LogicalDocument::Type {
            package: package.clone(),
            module: manifest.path.clone(),
            name: name.clone(),
        };
        let path = document.path(root, profile)?;
        let file: SpecificationFile<TypeSpecification> = profile.read(&path)?;
        checked_name(name, &file.name, &path)?;
        types.insert(name.clone(), Documented::new(file.doc, file.spec));
    }
    let mut values = IndexMap::new();
    for name in &manifest.values {
        let document = LogicalDocument::Value {
            package: package.clone(),
            module: manifest.path.clone(),
            name: name.clone(),
        };
        let path = document.path(root, profile)?;
        let file: SpecificationFile<ValueSpecification> = profile.read(&path)?;
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

/// Detect the homogeneous serialization profile of a document tree.
pub fn discover_document_tree_format(root: &VfsPath) -> Result<FormatId, TransportDiagnostic> {
    let candidates = [
        ("manifest.json", FormatId::json()),
        ("manifest.yaml", FormatId::yaml()),
    ];
    let mut found = Vec::new();
    for (name, format) in candidates {
        let path = root.join(name).map_err(|error| {
            tree_error(
                "morphir::ir::detection::invalid_manifest_path",
                Stage::Detection,
                error.to_string(),
                "use a valid document-tree root",
            )
        })?;
        if path
            .is_file()
            .map_err(|error| io_error("inspect", &path, Stage::Detection, error))?
        {
            found.push((name, format));
        }
    }
    match found.as_slice() {
        [(_, format)] => Ok(format.clone()),
        [] => Err(tree_error(
            "morphir::ir::detection::missing_manifest",
            Stage::Detection,
            "the document tree has no supported manifest",
            "add manifest.yaml or manifest.json, or select single-file input",
        )),
        _ => Err(tree_error(
            "morphir::ir::detection::ambiguous_manifest",
            Stage::Detection,
            format!(
                "the document tree contains multiple manifests: {}",
                found
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            "remove all but one supported manifest or select a separate tree",
        )),
    }
}

fn module_manifest_paths(
    root: &VfsPath,
    profile: &TreeProfile,
    package: &PackageName,
) -> Result<VecDeque<VfsPath>, TransportDiagnostic> {
    let package_root = package_root(root, package)?;
    if !package_root
        .exists()
        .map_err(|error| io_error("inspect", &package_root, Stage::Detection, error))?
    {
        return Ok(VecDeque::new());
    }
    let file_name = format!("module.{}", profile.extension());
    let mut paths = package_root
        .walk_dir()
        .map_err(|error| io_error("walk", &package_root, Stage::Detection, error))?
        .filter_map(|entry| match entry {
            Ok(path) if path.filename() == file_name => Some(Ok(path)),
            Ok(_) => None,
            Err(error) => Some(Err(io_error(
                "walk",
                &package_root,
                Stage::Detection,
                error,
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort_by(|left, right| left.as_str().cmp(right.as_str()));
    Ok(paths.into())
}

/// Push-based document-tree encoder that writes one module at a time.
pub struct DocumentTreeSink {
    root: VfsPath,
    profile: TreeProfile,
    manifest: Option<DistributionManifest>,
    modules: HashSet<String>,
    modules_started: bool,
    ended: bool,
}

impl DocumentTreeSink {
    /// Create an encoder for a staging tree.
    pub fn new(root: VfsPath, options: CodecOptions) -> Result<Self, TransportDiagnostic> {
        let profile = validate_options(&options)?;
        root.create_dir_all()
            .map_err(|error| io_error("create", &root, Stage::Publication, error))?;
        Ok(Self {
            root,
            profile,
            manifest: None,
            modules: HashSet::new(),
            modules_started: false,
            ended: false,
        })
    }

    fn begin(
        &mut self,
        header: DistributionHeader,
        cursor: &IrCursor,
    ) -> Result<(), TransportDiagnostic> {
        if self.manifest.is_some() {
            return Err(event_error(
                "duplicate_begin",
                cursor,
                "duplicate tree header",
            ));
        }
        let (format_version, distribution, package, entry_points) = match header {
            DistributionHeader::V4Library {
                format_version,
                package,
            } => (
                format_version,
                DistributionKind::Library,
                package,
                IndexMap::new(),
            ),
            DistributionHeader::V4Specs {
                format_version,
                package,
            } => (
                format_version,
                DistributionKind::Specs,
                package,
                IndexMap::new(),
            ),
            DistributionHeader::V4Application {
                format_version,
                package,
                entry_points,
            } => (
                format_version,
                DistributionKind::Application,
                package,
                entry_points,
            ),
            _ => {
                return Err(event_error(
                    "version_mismatch",
                    cursor,
                    "the v4 document-tree sink received a Classic v3 header",
                ));
            }
        };
        let package_storage = self.root.join("pkg").map_err(|error| {
            tree_error(
                "morphir::ir::document_tree::invalid_path",
                Stage::Publication,
                error.to_string(),
                "use a valid document-tree root",
            )
        })?;
        if package_storage
            .exists()
            .map_err(|error| io_error("inspect", &package_storage, Stage::Publication, error))?
        {
            package_storage
                .remove_dir_all()
                .map_err(|error| io_error("remove", &package_storage, Stage::Publication, error))?;
        }
        for name in ["manifest.json", "manifest.yaml", "manifest.yml"] {
            let path = self.root.join(name).map_err(|error| {
                tree_error(
                    "morphir::ir::document_tree::invalid_path",
                    Stage::Publication,
                    error.to_string(),
                    "use a valid document-tree root",
                )
            })?;
            if path
                .exists()
                .map_err(|error| io_error("inspect", &path, Stage::Publication, error))?
            {
                path.remove_file()
                    .map_err(|error| io_error("remove", &path, Stage::Publication, error))?;
            }
        }
        self.manifest = Some(DistributionManifest {
            format_version,
            distribution,
            package,
            dependencies: IndexMap::new(),
            entry_points,
            layout: "VfsMode".to_owned(),
        });
        Ok(())
    }

    fn dependency(
        &mut self,
        dependency: DependencyEvent,
        cursor: &IrCursor,
    ) -> Result<(), TransportDiagnostic> {
        if self.modules_started {
            return Err(event_error(
                "dependency_after_module",
                cursor,
                "a dependency appeared after the first module",
            ));
        }
        let DependencyEvent::V4 {
            package,
            specification,
        } = dependency
        else {
            return Err(event_error(
                "version_mismatch",
                cursor,
                "the v4 document-tree sink received a Classic v3 dependency",
            ));
        };
        let manifest = self.manifest.as_mut().ok_or_else(|| {
            event_error(
                "missing_begin",
                cursor,
                "a dependency appeared before the header",
            )
        })?;
        if manifest
            .dependencies
            .insert(package, specification)
            .is_some()
        {
            return Err(event_error(
                "duplicate_dependency",
                cursor,
                "the tree contains a duplicate dependency",
            ));
        }
        Ok(())
    }

    fn module(
        &mut self,
        module: ModuleEvent,
        cursor: &IrCursor,
    ) -> Result<(), TransportDiagnostic> {
        let manifest = self.manifest.as_ref().ok_or_else(|| {
            event_error(
                "missing_begin",
                cursor,
                "a module appeared before the header",
            )
        })?;
        let distribution = manifest.distribution;
        let package = manifest.package.clone();
        let path = match &module {
            ModuleEvent::V4Definition { path, .. } | ModuleEvent::V4Specification { path, .. } => {
                path
            }
            ModuleEvent::ClassicV3(_) => {
                return Err(event_error(
                    "version_mismatch",
                    cursor,
                    "the v4 document-tree sink received a Classic v3 module",
                ));
            }
        };
        if !self.modules.insert(path.clone()) {
            return Err(event_error(
                "duplicate_module",
                cursor,
                "the tree contains a duplicate module path",
            ));
        }
        self.modules_started = true;
        match (distribution, module) {
            (
                DistributionKind::Library | DistributionKind::Application,
                ModuleEvent::V4Definition { path, module },
            ) => write_definition_module(&self.root, &self.profile, &package, &path, &module),
            (DistributionKind::Specs, ModuleEvent::V4Specification { path, module }) => {
                write_specification_module(&self.root, &self.profile, &package, &path, &module)
            }
            _ => Err(event_error(
                "module_kind_mismatch",
                cursor,
                "the module event does not match the distribution kind",
            )),
        }
    }

    fn end(&mut self, cursor: &IrCursor) -> Result<(), TransportDiagnostic> {
        if self.ended {
            return Err(event_error(
                "duplicate_end",
                cursor,
                "duplicate tree end event",
            ));
        }
        let manifest = self.manifest.as_ref().ok_or_else(|| {
            event_error("missing_begin", cursor, "the tree ended before its header")
        })?;
        let path = LogicalDocument::Manifest.path(&self.root, &self.profile)?;
        self.profile.write(&path, manifest)?;
        self.ended = true;
        Ok(())
    }
}

impl EventSink for DocumentTreeSink {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        if self.ended {
            return Err(event_error(
                "event_after_end",
                event.cursor(),
                "an event appeared after the tree end",
            ));
        }
        let (cursor, kind) = event.into_parts();
        match kind {
            SemanticEventKind::Begin(header) => self.begin(header, &cursor),
            SemanticEventKind::Dependency(dependency) => self.dependency(dependency, &cursor),
            SemanticEventKind::Module(module) => self.module(module, &cursor),
            SemanticEventKind::End => self.end(&cursor),
        }
    }

    fn finish(&mut self) -> Result<(), TransportDiagnostic> {
        if self.ended {
            Ok(())
        } else {
            Err(event_error(
                "missing_end",
                &IrCursor::root(),
                "the event source ended before the tree manifest could be published",
            ))
        }
    }
}

fn event_error(
    suffix: &'static str,
    cursor: &IrCursor,
    message: &'static str,
) -> TransportDiagnostic {
    TransportDiagnostic::error(
        format!("morphir::ir::document_tree::{suffix}"),
        Stage::Encoding,
        cursor.clone(),
        message,
    )
    .with_guidance("verify the semantic event order and selected v4 tree profile")
}

enum SourceState {
    Header,
    Dependencies,
    Modules,
    End,
    Done,
}

/// Pull-based document-tree source that retains at most one module payload.
pub struct DocumentTreeSource {
    root: VfsPath,
    profile: TreeProfile,
    manifest: DistributionManifest,
    dependencies: VecDeque<(String, morphir_core::ir::v4::PackageSpecification)>,
    modules: VecDeque<VfsPath>,
    state: SourceState,
}

impl DocumentTreeSource {
    /// Open a homogeneous v4 document tree.
    pub fn open(root: VfsPath, options: CodecOptions) -> Result<Self, TransportDiagnostic> {
        let profile = validate_options(&options)?;
        let detected = discover_document_tree_format(&root)?;
        if detected != *options.format() {
            return Err(tree_error(
                "morphir::ir::detection::format_mismatch",
                Stage::Detection,
                format!(
                    "the tree manifest selects '{detected}', not '{}'",
                    options.format()
                ),
                "select the detected input format or rename and convert the complete tree",
            ));
        }
        let manifest_path = LogicalDocument::Manifest.path(&root, &profile)?;
        let mut manifest: DistributionManifest = profile.read(&manifest_path)?;
        let dependencies = std::mem::take(&mut manifest.dependencies)
            .into_iter()
            .collect::<VecDeque<_>>();
        let modules = module_manifest_paths(&root, &profile, &manifest.package)?;
        Ok(Self {
            root,
            profile,
            manifest,
            dependencies,
            modules,
            state: SourceState::Header,
        })
    }

    fn header(&self) -> DistributionHeader {
        match self.manifest.distribution {
            DistributionKind::Library => DistributionHeader::V4Library {
                format_version: self.manifest.format_version.clone(),
                package: self.manifest.package.clone(),
            },
            DistributionKind::Specs => DistributionHeader::V4Specs {
                format_version: self.manifest.format_version.clone(),
                package: self.manifest.package.clone(),
            },
            DistributionKind::Application => DistributionHeader::V4Application {
                format_version: self.manifest.format_version.clone(),
                package: self.manifest.package.clone(),
                entry_points: self.manifest.entry_points.clone(),
            },
        }
    }

    fn read_module(&self, path: &VfsPath) -> Result<ModuleEvent, TransportDiagnostic> {
        let manifest: ModuleManifest = self.profile.read(path)?;
        let expected = LogicalDocument::Module {
            package: self.manifest.package.clone(),
            module: manifest.path.clone(),
        }
        .path(&self.root, &self.profile)?;
        if expected.as_str() != path.as_str() {
            return Err(tree_error(
                "morphir::ir::document_tree::module_path_mismatch",
                Stage::Normalization,
                format!(
                    "module manifest {} declares path '{}'",
                    path.as_str(),
                    manifest.path
                ),
                "move the module manifest to its canonical logical path or correct its embedded path",
            ));
        }
        match self.manifest.distribution {
            DistributionKind::Library | DistributionKind::Application => {
                let (path, module) = read_definition_module(
                    &self.root,
                    &self.profile,
                    &self.manifest.package,
                    manifest,
                )?;
                Ok(ModuleEvent::V4Definition { path, module })
            }
            DistributionKind::Specs => {
                let (path, module) = read_specification_module(
                    &self.root,
                    &self.profile,
                    &self.manifest.package,
                    manifest,
                )?;
                Ok(ModuleEvent::V4Specification { path, module })
            }
        }
    }
}

impl EventSource for DocumentTreeSource {
    fn next_event(&mut self) -> Result<Option<SemanticEvent>, TransportDiagnostic> {
        let distribution_cursor = IrCursor::root().child(CursorSegment::Distribution);
        loop {
            match self.state {
                SourceState::Header => {
                    self.state = SourceState::Dependencies;
                    return Ok(Some(SemanticEvent::new(
                        distribution_cursor,
                        SemanticEventKind::Begin(self.header()),
                    )));
                }
                SourceState::Dependencies => {
                    if let Some((package, specification)) = self.dependencies.pop_front() {
                        let cursor =
                            distribution_cursor.child(CursorSegment::Dependency(package.clone()));
                        return Ok(Some(SemanticEvent::new(
                            cursor,
                            SemanticEventKind::Dependency(DependencyEvent::V4 {
                                package,
                                specification,
                            }),
                        )));
                    }
                    self.state = SourceState::Modules;
                }
                SourceState::Modules => {
                    if let Some(path) = self.modules.pop_front() {
                        let module = self.read_module(&path)?;
                        let module_path = match &module {
                            ModuleEvent::V4Definition { path, .. }
                            | ModuleEvent::V4Specification { path, .. } => path.clone(),
                            ModuleEvent::ClassicV3(_) => unreachable!(),
                        };
                        let cursor = distribution_cursor.child(CursorSegment::Module(module_path));
                        return Ok(Some(SemanticEvent::new(
                            cursor,
                            SemanticEventKind::Module(module),
                        )));
                    }
                    self.state = SourceState::End;
                }
                SourceState::End => {
                    self.state = SourceState::Done;
                    return Ok(Some(SemanticEvent::new(
                        distribution_cursor,
                        SemanticEventKind::End,
                    )));
                }
                SourceState::Done => return Ok(None),
            }
        }
    }
}

/// Write a concrete v4 value to a document tree using explicit codec options.
pub fn write_document_tree_with_options(
    root: &VfsPath,
    ir: &IRFile,
    options: &CodecOptions,
) -> Result<(), TransportDiagnostic> {
    let mut sink = DocumentTreeSink::new(root.clone(), options.clone())?;
    semantic::emit_v4(ir.clone(), &mut sink)
}

/// Read a concrete v4 value from a document tree using explicit codec options.
pub fn read_document_tree_with_options(
    root: &VfsPath,
    options: &CodecOptions,
) -> Result<IRFile, TransportDiagnostic> {
    let mut source = DocumentTreeSource::open(root.clone(), options.clone())?;
    match semantic::collect(&mut source, IrVersion::V4)? {
        SemanticFile::V4(file) => Ok(file),
        SemanticFile::ClassicV3(_) => unreachable!(),
    }
}

/// Write a concrete v4 distribution using the legacy JSON tree default.
pub fn write_document_tree(root: &VfsPath, ir: &IRFile) -> Result<(), TransportDiagnostic> {
    write_document_tree_with_options(
        root,
        ir,
        &CodecOptions::new(IrVersion::V4, Layout::DocumentTree, FormatId::json()),
    )
}

/// Discover and read a concrete v4 JSON or YAML document tree.
pub fn read_document_tree(root: &VfsPath) -> Result<IRFile, TransportDiagnostic> {
    let format = discover_document_tree_format(root)?;
    read_document_tree_with_options(
        root,
        &CodecOptions::new(IrVersion::V4, Layout::DocumentTree, format),
    )
}
