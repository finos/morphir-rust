//! The v4 document-tree transport: a filesystem adapter over `morphir_core::ir::layout`.
//!
//! The layout itself — the logical path grammar, the escaped stems, the path budget, the four
//! tree-file models and the order a tree is written and read in — lives in the kit, over a plain
//! map of logical path to text. Nothing of it is repeated here. What this module adds is the two
//! things a map cannot do: put a file somewhere, and find one again.
//!
//! [`DocumentTreeSink`] keeps its event-order state machine, because the streaming guarantee is
//! the reason it exists: a module's files are written the moment its event arrives, through the
//! kit's per-module writers, and only the distribution manifest waits for the end of the stream.
//! [`DocumentTreeSource`] is the other way round — a tree is a directory, and a directory answers
//! no questions until it has been walked — so it reads the whole tree, hands it to
//! [`layout::read_tree`], and replays the events the equivalent single document would have
//! produced.
//!
//! The tree this writes is not the tree releases up to 0.4.0-alpha.7 wrote. There is no
//! compatibility shim: an older tree is refused, with the guidance that says so.
//!
//! The Ion tree has the same paths, but its files hold the single-file Ion elements rather than
//! the kit's JSON value tree, so the Ion codec lays it out and reads it back. It is defined for v3
//! and v4. Its sink holds the whole distribution, because the Ion writer takes the datagram apart.

use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};

use morphir_core::ir::layout::{
    self, ManifestHeader, Profile, Root, Tree, TreePolicy, from_physical, to_physical,
};
use morphir_core::ir::v4::tree_files::DistributionKind;
use morphir_core::ir::v4::{EntryPoints, FormatVersion, IRFile};
use morphir_core::ir::{Diagnostic as CoreDiagnostic, DiagnosticCode};
use morphir_core::naming::PackageName;
use morphir_core::traversal::{
    CursorSegment, DependencyEvent, DistributionHeader, IrCursor, ModuleEvent, SemanticEvent,
    SemanticEventKind,
};
use vfs::VfsPath;

use super::diagnostic::{core_code_name, core_message, core_source_span, core_stage};
use super::ion;
use super::semantic;
use super::{
    CodecOptions, EventSink, EventSource, FormatId, IrVersion, Layout, Stage, TransportDiagnostic,
};

/// The guidance a tree written before the layout moved into the kit earns.
///
/// `pathBudget` is the one required manifest member no older tree has, so its absence is the
/// reliable signal that a tree predates the change rather than being merely malformed.
const MIGRATE_GUIDANCE: &str =
    "this tree predates 0.4.0-beta.1; regenerate it with morphir migrate";

/// The manifest file names a tree root may carry, and the spelling each selects.
///
/// `.yml` is read but never written, as the kit's `from_physical` treats it.
const MANIFEST_NAMES: [(&str, Spelling); 4] = [
    ("manifest.json", Spelling::Kit(Profile::Json)),
    ("manifest.yaml", Spelling::Kit(Profile::Yaml)),
    ("manifest.yml", Spelling::Kit(Profile::Yaml)),
    ("manifest.ion", Spelling::Ion),
];

/// How a tree's files are spelled: one of the kit's profiles, or the Ion elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spelling {
    Kit(Profile),
    Ion,
}

impl Spelling {
    fn name(self) -> &'static str {
        match self {
            Spelling::Kit(profile) => profile.name(),
            Spelling::Ion => "ion",
        }
    }

    /// The physical path a logical one takes in this spelling.
    fn physical(self, logical: &str) -> String {
        match self {
            Spelling::Kit(profile) => to_physical(logical, profile),
            Spelling::Ion => format!("{logical}{}", ion::TREE_EXTENSION),
        }
    }

    /// The logical path of a physical name this spelling reads, or `None` when the tree ignores
    /// the file. An Ion tree also sees the kit's extensions, so a JSON file in it is refused
    /// rather than skipped.
    fn logical(self, physical: &str) -> Option<String> {
        match self {
            Spelling::Kit(_) => from_physical(physical),
            Spelling::Ion => from_physical(physical).or_else(|| {
                physical
                    .strip_suffix(ion::TREE_EXTENSION)
                    .map(str::to_owned)
            }),
        }
    }
}

/// What a tree operation needs to know: the spelling and the path budget.
#[derive(Debug, Clone, Copy)]
struct Policy {
    spelling: Spelling,
    path_budget: u32,
}

impl Policy {
    /// The kit's policy, when the tree is spelled in a kit profile.
    fn kit(self) -> Option<TreePolicy> {
        match self.spelling {
            Spelling::Kit(profile) => Some(TreePolicy {
                profile,
                path_budget: self.path_budget,
            }),
            Spelling::Ion => None,
        }
    }
}

// =============================================================================
// Diagnostics
// =============================================================================

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

/// One of the kit's diagnostics, as this transport spells it.
///
/// The code is the kit's own under `morphir::ir::document_tree::`, the stage and the message are
/// mapped exactly as the YAML codec maps them — the kit's cursor is a logical path and a JSON
/// pointer, which has no semantic spelling, so it travels in the message.
fn core_error(diagnostic: CoreDiagnostic) -> TransportDiagnostic {
    let transport = TransportDiagnostic::error(
        format!(
            "morphir::ir::document_tree::{}",
            core_code_name(diagnostic.code)
        ),
        core_stage(diagnostic.stage),
        IrCursor::root(),
        core_message(&diagnostic),
    )
    .with_guidance(guidance_for(&diagnostic));
    match core_source_span(&diagnostic) {
        Some(span) => transport.with_source_span(span),
        None => transport,
    }
}

fn guidance_for(diagnostic: &CoreDiagnostic) -> &'static str {
    if is_missing_path_budget(diagnostic) {
        return MIGRATE_GUIDANCE;
    }
    match diagnostic.code {
        DiagnosticCode::InvalidDistributionShape => {
            "correct the document tree's shape for the selected v4 tree profile"
        }
        _ => "correct the document-tree file for the selected concrete IR version",
    }
}

/// A distribution manifest with no `pathBudget`: the shape every tree written before the layout
/// moved into the kit has, and the one an older tree is recognized by.
fn is_missing_path_budget(diagnostic: &CoreDiagnostic) -> bool {
    diagnostic.code == DiagnosticCode::MissingMember
        && diagnostic
            .cursor
            .starts_with(&format!("{}#", layout::MANIFEST))
        && diagnostic.message.ends_with("pathBudget")
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

// =============================================================================
// Options and the profile boundary
// =============================================================================

/// The spelling a format identifier selects.
fn spelling_of(format: &FormatId) -> Result<Spelling, TransportDiagnostic> {
    if *format == FormatId::json() {
        Ok(Spelling::Kit(Profile::Json))
    } else if *format == FormatId::yaml() {
        Ok(Spelling::Kit(Profile::Yaml))
    } else if *format == FormatId::ion() {
        Ok(Spelling::Ion)
    } else {
        Err(tree_error(
            "morphir::ir::document_tree::unsupported_format",
            Stage::Detection,
            format!("document trees do not have a '{format}' profile"),
            "select json, yaml, or ion, or register a document-tree profile",
        ))
    }
}

/// The format identifier a spelling is selected by.
fn format_of(spelling: Spelling) -> FormatId {
    match spelling {
        Spelling::Kit(Profile::Json) => FormatId::json(),
        Spelling::Kit(Profile::Yaml) => FormatId::yaml(),
        Spelling::Ion => FormatId::ion(),
    }
}

fn validate_options(options: &CodecOptions) -> Result<Policy, TransportDiagnostic> {
    let spelling = spelling_of(options.format())?;
    if options.version() != IrVersion::V4 && spelling != Spelling::Ion {
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
    Ok(Policy {
        spelling,
        path_budget: options.path_budget(),
    })
}

/// Whether a physical name in the tree is spelled in `spelling`.
///
/// The kit recognizes three extensions and writes two; `.yml` is the YAML profile's read-only
/// spelling, so it agrees with a YAML tree rather than being the mismatch the reference's
/// directory adapter calls it. A tree whose manifest is `manifest.yml` is otherwise unreadable.
fn agrees_with(physical: &str, spelling: Spelling) -> bool {
    match spelling {
        Spelling::Kit(Profile::Json) => physical.ends_with(".json"),
        Spelling::Kit(Profile::Yaml) => physical.ends_with(".yaml") || physical.ends_with(".yml"),
        Spelling::Ion => physical.ends_with(ion::TREE_EXTENSION),
    }
}

/// The physical path a logical one takes under a tree root.
fn physical_path(
    root: &VfsPath,
    logical: &str,
    spelling: Spelling,
) -> Result<VfsPath, TransportDiagnostic> {
    root.join(spelling.physical(logical)).map_err(|error| {
        tree_error(
            "morphir::ir::document_tree::invalid_path",
            Stage::Publication,
            error.to_string(),
            "use valid Morphir package, module, and definition names",
        )
    })
}

/// Writes one file of a tree, creating the directories it sits under.
fn publish(
    root: &VfsPath,
    spelling: Spelling,
    (logical, text): (String, String),
) -> Result<(), TransportDiagnostic> {
    let path = physical_path(root, &logical, spelling)?;
    let parent = path.parent();
    parent
        .create_dir_all()
        .map_err(|error| io_error("create", &parent, Stage::Publication, error))?;
    let mut writer = path
        .create_file()
        .map_err(|error| io_error("create", &path, Stage::Publication, error))?;
    writer
        .write_all(text.as_bytes())
        .map_err(|error| io_error("write", &path, Stage::Publication, error))?;
    writer
        .flush()
        .map_err(|error| io_error("flush", &path, Stage::Publication, error))
}

/// A dependency's package name, out of the key the event carries it under.
fn dependency_package(key: &str) -> Result<PackageName, TransportDiagnostic> {
    PackageName::from_canonical_string(key).map_err(|message| {
        tree_error(
            "morphir::ir::document_tree::invalid_package_path",
            Stage::Encoding,
            message,
            "use a valid v4 package name",
        )
    })
}

// =============================================================================
// Discovery
// =============================================================================

/// Detect the homogeneous serialization profile of a document tree.
pub fn discover_document_tree_format(root: &VfsPath) -> Result<FormatId, TransportDiagnostic> {
    let mut found = Vec::new();
    for (name, spelling) in MANIFEST_NAMES {
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
            found.push((name, spelling));
        }
    }
    match found.as_slice() {
        [(_, spelling)] => Ok(format_of(*spelling)),
        [] => Err(tree_error(
            "morphir::ir::detection::missing_manifest",
            Stage::Detection,
            "the document tree has no supported manifest",
            "add manifest.yaml, manifest.json, or manifest.ion, or select single-file input",
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

// =============================================================================
// The walk
// =============================================================================

/// Reads every file of the tree under `root` into the map the kit's reader takes.
///
/// Two kinds of entry are passed over. A dot-directory or dot-file — `.git`, `.morphir` — is never
/// part of a tree, and is skipped without being descended into, so a tree inside a working copy
/// reads as the tree rather than as the working copy. A file whose extension the kit does not
/// recognize is not a tree file at all and is ignored, as the reference's directory adapter
/// ignores it.
///
/// Links are skipped too, on a root that can tell: [`morphir_common::vfs::physical_root`][pr]
/// builds a `ContainedPhysicalFS`, whose `read_dir` never yields a symlink or a junction, so a
/// linked file and a linked directory are both simply absent from the walk and nothing outside the
/// OS root can be read through one. On any other backend the walk sees whatever that backend
/// reports; `MemoryFS`, the only other one used here, has no links to report.
///
/// [`MAX_TREE_DEPTH`] is a backstop, not a rule about trees: no backend in this crate can present a
/// cycle, but a walk driven by someone else's `FileSystem` should end rather than recurse forever.
/// It is a fixed depth rather than anything derived from the caller's path budget, which would
/// refuse a legitimate deep tree read back under a smaller budget than it was written with.
///
/// [pr]: crate::vfs::physical_root
fn read_tree_files(root: &VfsPath, spelling: Spelling) -> Result<Tree, TransportDiagnostic> {
    let mut files = Tree::new();
    let mut physical = std::collections::HashMap::new();
    read_directory(root, "", 0, spelling, &mut files, &mut physical)?;
    Ok(files)
}

/// The deepest a tree walk descends. A logical path is `deps/<package segments>/@/<module
/// segments>/<leaf>`, so a real tree is a handful of levels; this is orders of magnitude clear of
/// anything a distribution can spell.
const MAX_TREE_DEPTH: usize = 256;

fn read_directory(
    directory: &VfsPath,
    relative: &str,
    depth: usize,
    spelling: Spelling,
    files: &mut Tree,
    physical: &mut std::collections::HashMap<String, String>,
) -> Result<(), TransportDiagnostic> {
    if depth > MAX_TREE_DEPTH {
        return Err(tree_error(
            "morphir::ir::document_tree::invalid_path",
            Stage::Detection,
            format!(
                "the directory '{relative}' nests deeper than {MAX_TREE_DEPTH} levels; no document \
                 tree does, and a filesystem cycle does"
            ),
            "remove the directory cycle under the tree root",
        ));
    }
    let entries = directory
        .read_dir()
        .map_err(|error| io_error("list", directory, Stage::Detection, error))?;
    for entry in entries {
        let name = entry.filename();
        if name.starts_with('.') {
            continue;
        }
        let child = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        if entry
            .is_dir()
            .map_err(|error| io_error("inspect", &entry, Stage::Detection, error))?
        {
            read_directory(&entry, &child, depth + 1, spelling, files, physical)?;
            continue;
        }
        let Some(logical) = spelling.logical(&child) else {
            continue;
        };
        if !agrees_with(&child, spelling) {
            return Err(core_error(CoreDiagnostic::new(
                DiagnosticCode::InvalidDistributionShape,
                morphir_core::ir::DiagnosticStage::Semantic,
                logical.clone(),
                format!("{child} is not a {} file", spelling.name()),
            )));
        }
        if let Some(previous) = physical.insert(logical.clone(), child.clone()) {
            return Err(core_error(CoreDiagnostic::new(
                DiagnosticCode::InvalidDistributionShape,
                morphir_core::ir::DiagnosticStage::Semantic,
                logical,
                format!("both {previous} and {child} map to the same tree file; keep only one"),
            )));
        }
        files.insert(logical, read_text(&entry)?);
    }
    Ok(())
}

fn read_text(path: &VfsPath) -> Result<String, TransportDiagnostic> {
    let mut reader = path
        .open_file()
        .map_err(|error| io_error("open", path, Stage::Syntax, error))?;
    let mut text = String::new();
    reader
        .read_to_string(&mut text)
        .map_err(|error| io_error("read", path, Stage::Syntax, error))?;
    Ok(text)
}

// =============================================================================
// The sink
// =============================================================================

/// What the distribution header said, held until `end` has the dependency names to go with it.
struct SinkHeader {
    format_version: FormatVersion,
    distribution: DistributionKind,
    package: PackageName,
    entry_points: EntryPoints,
}

/// Push-based document-tree encoder.
///
/// A tree in a kit profile is written one module at a time. An Ion tree is written when the
/// stream ends.
pub struct DocumentTreeSink {
    inner: SinkSpelling,
}

enum SinkSpelling {
    Kit(Box<KitSink>),
    Ion(IonSink),
}

impl DocumentTreeSink {
    /// Create an encoder for a staging tree.
    pub fn new(root: VfsPath, options: CodecOptions) -> Result<Self, TransportDiagnostic> {
        let policy = validate_options(&options)?;
        root.create_dir_all()
            .map_err(|error| io_error("create", &root, Stage::Publication, error))?;
        let inner = match policy.kit() {
            Some(policy) => SinkSpelling::Kit(Box::new(KitSink::new(root, policy))),
            None => SinkSpelling::Ion(IonSink {
                root,
                version: options.version(),
                path_budget: policy.path_budget,
                events: VecDeque::new(),
                ended: false,
            }),
        };
        Ok(Self { inner })
    }
}

impl EventSink for DocumentTreeSink {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        match &mut self.inner {
            SinkSpelling::Kit(sink) => sink.accept(event),
            SinkSpelling::Ion(sink) => sink.accept(event),
        }
    }

    fn finish(&mut self) -> Result<(), TransportDiagnostic> {
        match &mut self.inner {
            SinkSpelling::Kit(sink) => sink.finish(),
            SinkSpelling::Ion(sink) => sink.finish(),
        }
    }
}

/// Empties the tree of everything a previous write left: both package roots, and whichever
/// manifest spelling is there. A module that is no longer in the distribution, and a dependency
/// that is no longer listed, both disappear this way — there is no other staleness story, because
/// a streaming writer never sees the whole tree it is replacing.
///
/// The removal never follows a link. `VfsPath::remove_dir_all` is plain recursion over `read_dir`
/// and `metadata`, so on a backend that resolves links it would delete a link's target rather than
/// the link; [`morphir_common::vfs::physical_root`][pr] therefore builds a `ContainedPhysicalFS`,
/// which hides linked children from `read_dir` and removes a link *as a link* when one stands in
/// the way. A symlink or junction at `pkg/`, at `deps/`, or anywhere beneath them is unlinked;
/// nothing outside the OS root is touched. On a backend with no links to begin with — `MemoryFS` —
/// there is nothing to contain.
///
/// [pr]: crate::vfs::physical_root
fn prune(tree: &VfsPath) -> Result<(), TransportDiagnostic> {
    for root in [Root::Pkg, Root::Deps] {
        let path = tree.join(root.as_str()).map_err(|error| {
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
            path.remove_dir_all()
                .map_err(|error| io_error("remove", &path, Stage::Publication, error))?;
        }
    }
    for (name, _) in MANIFEST_NAMES {
        let path = tree.join(name).map_err(|error| {
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
    Ok(())
}

/// The Ion tree encoder. The Ion writer lays out a whole distribution, so the events wait for the
/// end of the stream.
struct IonSink {
    root: VfsPath,
    version: IrVersion,
    path_budget: u32,
    events: VecDeque<SemanticEvent>,
    ended: bool,
}

impl EventSink for IonSink {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        if self.ended {
            return Err(event_error(
                "event_after_end",
                event.cursor(),
                "an event appeared after the tree end",
            ));
        }
        let end = matches!(event.kind(), SemanticEventKind::End);
        self.events.push_back(event);
        if !end {
            return Ok(());
        }
        self.ended = true;
        let mut source = QueueSource(std::mem::take(&mut self.events));
        let files = ion::write_tree(&mut source, self.version, self.path_budget)?;
        prune(&self.root)?;
        files
            .into_iter()
            .try_for_each(|file| publish(&self.root, Spelling::Ion, file))
    }

    fn finish(&mut self) -> Result<(), TransportDiagnostic> {
        if self.ended {
            Ok(())
        } else {
            Err(event_error(
                "missing_end",
                &IrCursor::root(),
                "the event source ended before the tree could be published",
            ))
        }
    }
}

/// Replays buffered events.
struct QueueSource(VecDeque<SemanticEvent>);

impl EventSource for QueueSource {
    fn next_event(&mut self) -> Result<Option<SemanticEvent>, TransportDiagnostic> {
        Ok(self.0.pop_front())
    }
}

/// The kit-profile encoder, which writes one module at a time.
struct KitSink {
    root: VfsPath,
    policy: TreePolicy,
    header: Option<SinkHeader>,
    /// The dependency packages, in the order their events arrived; the manifest lists them.
    dependencies: Vec<PackageName>,
    dependency_keys: HashSet<String>,
    modules: HashSet<String>,
    modules_started: bool,
    ended: bool,
}

impl KitSink {
    fn new(root: VfsPath, policy: TreePolicy) -> Self {
        Self {
            root,
            policy,
            header: None,
            dependencies: Vec::new(),
            dependency_keys: HashSet::new(),
            modules: HashSet::new(),
            modules_started: false,
            ended: false,
        }
    }

    fn publish_all(&self, files: Vec<(String, String)>) -> Result<(), TransportDiagnostic> {
        files
            .into_iter()
            .try_for_each(|file| publish(&self.root, Spelling::Kit(self.policy.profile), file))
    }

    fn begin(
        &mut self,
        header: DistributionHeader,
        cursor: &IrCursor,
    ) -> Result<(), TransportDiagnostic> {
        if self.header.is_some() {
            return Err(event_error(
                "duplicate_begin",
                cursor,
                "duplicate tree header",
            ));
        }
        let header = match header {
            DistributionHeader::V4Library {
                format_version,
                package,
            } => SinkHeader {
                format_version,
                distribution: DistributionKind::Library,
                package,
                entry_points: EntryPoints::new(),
            },
            DistributionHeader::V4Specs {
                format_version,
                package,
            } => SinkHeader {
                format_version,
                distribution: DistributionKind::Specs,
                package,
                entry_points: EntryPoints::new(),
            },
            DistributionHeader::V4Application {
                format_version,
                package,
                entry_points,
            } => SinkHeader {
                format_version,
                distribution: DistributionKind::Application,
                package,
                entry_points,
            },
            _ => {
                return Err(event_error(
                    "version_mismatch",
                    cursor,
                    "the v4 document-tree sink received a Classic v3 header",
                ));
            }
        };
        prune(&self.root)?;
        self.header = Some(header);
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
        let kind = self.header.as_ref().map(|header| header.distribution);
        let unsupported = || {
            event_error(
                "unsupported_dependencies",
                cursor,
                "the dependency's kind does not match the distribution kind",
            )
        };
        // A `Library` or a `Specs` publishes its dependencies' public faces; an `Application`
        // links them statically, so its `deps/` holds definitions (distributions-0010). A
        // dependency of the other kind has no file to go in.
        match dependency {
            DependencyEvent::V4 {
                package,
                specification,
            } => {
                if kind == Some(DistributionKind::Application) {
                    return Err(unsupported());
                }
                let header =
                    self.require_header(cursor, "a dependency appeared before the header")?;
                let name = dependency_package(&package)?;
                let format_version = header.format_version.clone();
                self.record_dependency(package, name.clone(), cursor)?;
                for (module_name, module) in &specification.modules {
                    let files = layout::write_specification_module(
                        Root::Deps,
                        &name,
                        module_name,
                        module,
                        &format_version,
                        &self.policy,
                    )
                    .map_err(core_error)?;
                    self.publish_all(files)?;
                }
                Ok(())
            }
            DependencyEvent::V4Definition {
                package,
                definition,
            } => {
                if kind != Some(DistributionKind::Application) {
                    return Err(unsupported());
                }
                let header =
                    self.require_header(cursor, "a dependency appeared before the header")?;
                let name = dependency_package(&package)?;
                let format_version = header.format_version.clone();
                self.record_dependency(package, name.clone(), cursor)?;
                for (module_name, module) in &definition.modules {
                    let files = layout::write_definition_module(
                        Root::Deps,
                        &name,
                        module_name,
                        module,
                        &format_version,
                        &self.policy,
                    )
                    .map_err(core_error)?;
                    self.publish_all(files)?;
                }
                Ok(())
            }
            DependencyEvent::ClassicV3 { .. } => Err(event_error(
                "version_mismatch",
                cursor,
                "the v4 document-tree sink received a Classic v3 dependency",
            )),
        }
    }

    /// The header, or the `missing_begin` an event arriving before it earns.
    fn require_header(
        &self,
        cursor: &IrCursor,
        message: &'static str,
    ) -> Result<&SinkHeader, TransportDiagnostic> {
        self.header
            .as_ref()
            .ok_or_else(|| event_error("missing_begin", cursor, message))
    }

    fn record_dependency(
        &mut self,
        key: String,
        name: PackageName,
        cursor: &IrCursor,
    ) -> Result<(), TransportDiagnostic> {
        if !self.dependency_keys.insert(key) {
            return Err(event_error(
                "duplicate_dependency",
                cursor,
                "the tree contains a duplicate dependency",
            ));
        }
        self.dependencies.push(name);
        Ok(())
    }

    fn module(
        &mut self,
        module: ModuleEvent,
        cursor: &IrCursor,
    ) -> Result<(), TransportDiagnostic> {
        let header = self.require_header(cursor, "a module appeared before the header")?;
        let kind = header.distribution;
        let package = header.package.clone();
        let format_version = header.format_version.clone();
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
        let files = match (kind, &module) {
            (
                DistributionKind::Library | DistributionKind::Application,
                ModuleEvent::V4Definition { path, module },
            ) => layout::write_definition_module(
                Root::Pkg,
                &package,
                path,
                module,
                &format_version,
                &self.policy,
            ),
            (DistributionKind::Specs, ModuleEvent::V4Specification { path, module }) => {
                layout::write_specification_module(
                    Root::Pkg,
                    &package,
                    path,
                    module,
                    &format_version,
                    &self.policy,
                )
            }
            _ => {
                return Err(event_error(
                    "module_kind_mismatch",
                    cursor,
                    "the module event does not match the distribution kind",
                ));
            }
        }
        .map_err(core_error)?;
        self.publish_all(files)
    }

    fn end(&mut self, cursor: &IrCursor) -> Result<(), TransportDiagnostic> {
        if self.ended {
            return Err(event_error(
                "duplicate_end",
                cursor,
                "duplicate tree end event",
            ));
        }
        let header = self.header.as_ref().ok_or_else(|| {
            event_error("missing_begin", cursor, "the tree ended before its header")
        })?;
        let manifest = layout::write_manifest_header(
            &ManifestHeader {
                format_version: header.format_version.clone(),
                distribution: header.distribution,
                package: header.package.clone(),
                dependencies: self.dependencies.clone(),
                entry_points: header.entry_points.clone(),
            },
            &self.policy,
        );
        publish(&self.root, Spelling::Kit(self.policy.profile), manifest)?;
        self.ended = true;
        Ok(())
    }
}

impl EventSink for KitSink {
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

// =============================================================================
// The source
// =============================================================================

/// Pull-based document-tree source.
///
/// A directory answers nothing until it has been walked, and the kit's reader takes the whole tree
/// at once, so the tree is read and assembled in [`DocumentTreeSource::open`] and the events are
/// the ones the equivalent single document would have produced. That is the same trade the YAML
/// encoder makes in the other direction: a canonical whole is worth one document in memory. The
/// *sink* is where this transport's streaming guarantee lives.
pub struct DocumentTreeSource {
    events: VecDeque<SemanticEvent>,
}

impl DocumentTreeSource {
    /// Open a homogeneous v4 document tree.
    ///
    /// The warnings [`layout::read_tree`] reports — a legacy spelling accepted, say — are dropped
    /// here. An [`EventSource`] has no channel for a non-fatal observation, exactly as
    /// `IrCodec::decode` has none for the YAML codec's header observations; a caller that wants
    /// them calls [`layout::read_tree`] itself.
    pub fn open(root: VfsPath, options: CodecOptions) -> Result<Self, TransportDiagnostic> {
        let policy = validate_options(&options)?;
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
        let files = read_tree_files(&root, policy.spelling)?;
        // Discovery answers `is_file`, which resolves a link; the walk skips links. When the two
        // disagree the manifest is a link, and the kit's reader would otherwise report a tree with
        // no manifest at all — true, but not the reason.
        if !files.contains_key(layout::MANIFEST) {
            return Err(tree_error(
                "morphir::ir::detection::linked_manifest",
                Stage::Detection,
                "the tree manifest was found but not read, which is what a symlink or junction in \
                 its place does: a document tree is read through real files",
                "replace the link with the manifest file, or open the directory the link resolves to",
            ));
        }
        let mut queue = QueueSink::default();
        match policy.kit() {
            Some(kit) => {
                let (file, _warnings) =
                    layout::read_tree(&files, kit.profile).map_err(core_error)?;
                semantic::emit_v4(file, &mut queue)?;
            }
            None => ion::read_tree(&files, options.version(), &mut queue)?,
        }
        Ok(Self {
            events: queue.events,
        })
    }
}

/// Collects the events `emit_v4` pushes, so the source can hand them back one at a time.
#[derive(Default)]
struct QueueSink {
    events: VecDeque<SemanticEvent>,
}

impl EventSink for QueueSink {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        self.events.push_back(event);
        Ok(())
    }
}

impl EventSource for DocumentTreeSource {
    fn next_event(&mut self) -> Result<Option<SemanticEvent>, TransportDiagnostic> {
        Ok(self.events.pop_front())
    }
}

// =============================================================================
// Whole-value convenience
// =============================================================================

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
        semantic::SemanticFile::V4(file) => Ok(file),
        // `collect` is asked for v4 and every event came from `emit_v4`, so a classic v3 file is
        // not a shape this can answer with; refusing it keeps the reader total.
        semantic::SemanticFile::ClassicV3(_) => Err(event_error(
            "version_mismatch",
            &IrCursor::root().child(CursorSegment::Distribution),
            "a v4 document tree assembled a Classic v3 distribution",
        )),
    }
}

/// Write a concrete v4 distribution using the JSON tree default.
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
