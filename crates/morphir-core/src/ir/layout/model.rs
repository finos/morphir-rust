//! The version-specific half of a document tree.
//!
//! A tree's layout — the root manifest, the `pkg/` and `deps/` roots, a `module` file per module
//! directory, one node file per type or value, the path budget and the rule that every file under
//! a package root is claimed — is the same whichever IR version wrote the files. What a file says,
//! and how the pieces become one distribution, is not. [`super::read`] owns the first and asks a
//! [`TreeModel`] for the second, so a new IR version is a new model rather than a new reader.
//!
//! The types here are what the two halves hand each other: an [`Envelope`] for the root manifest,
//! a [`ModuleFile`] for a module manifest, a [`Node`] for a node file, and [`Packages`] for every
//! module the reader found, ready to be assembled. Writing goes the other way through the same
//! trait: [`super::write`] owns the budget, the stems and the file order, and hands the model an
//! [`Envelope`], a [`ModuleHeader`] or a [`Node`] to encode.

use indexmap::IndexMap;

use crate::ir::Diagnostic;
use crate::ir::layout::paths::Root;
use crate::ir::v4::module::Documentation;
use crate::naming::{Name, PackageName, Path};

/// What a distribution manifest says, whichever IR version wrote it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Envelope<K, E> {
    pub kind: K,
    pub package: PackageName,
    pub path_budget: u32,
    pub dependencies: Vec<PackageName>,
    /// Anything only one version has (v4: format version and entry points).
    pub extra: E,
}

/// One module directory's manifest, decoded.
pub(crate) struct ModuleFile<TD, VD, TS, VS> {
    pub path: Path,
    pub public: bool,
    /// Kept as the decoder normalised it: turning it back into text and normalising it again is
    /// not a no-op for every string, so the reader never does.
    pub doc: Option<Documentation>,
    pub types: Entries<TD, TS>,
    pub values: Entries<VD, VS>,
    /// The names whose file stem was truncated for the path budget, each with the stem its file is
    /// under.
    pub file_names: Vec<(Name, String)>,
}

/// What a module manifest says about the module itself, for the writer to hand a model: its
/// listings and `fileNames` come from the budget, so the writer passes them apart.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ModuleHeader {
    pub path: Path,
    pub public: bool,
    pub doc: Option<Documentation>,
}

/// A module manifest's `types` or `values` member: the names whose files hold the entries, or the
/// entries themselves written out inline in one of the two styles.
pub(crate) enum Entries<D, S> {
    Names(Vec<Name>),
    Definitions(IndexMap<String, D>),
    Specifications(IndexMap<String, S>),
}

/// Whether a module directory holds definitions or specifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Definitions,
    Specifications,
}

/// One parsed tree file, whatever the profile parsed it into. The shared layout asks it only
/// for its `formatVersion`, to check that every file agrees with the manifest under a model that
/// asks for it ([`TreeModel::FILES_REPEAT_MANIFEST_VERSION`]).
pub(crate) trait Payload {
    /// The `formatVersion` member as canonical JSON text (`3`, `"3.1.0"`), if present.
    fn format_version(&self) -> Option<String>;
}

impl Payload for serde_json::Value {
    fn format_version(&self) -> Option<String> {
        self.get("formatVersion").map(|value| value.to_string())
    }
}

/// The version-specific half of a document tree: how its files decode and encode, and how
/// the pieces the shared layout found become one distribution. `Doc` is the payload type:
/// `serde_json::Value` for the JSON and YAML profiles here; a later bead gives the Ion tree
/// `ion_rs::Element`.
///
/// Every decoder is handed the parsed file and the cursor of its root, and reports its
/// diagnostics against that cursor; the reader re-cursors them onto the file's logical path.
pub(crate) trait TreeModel {
    type Doc: Payload;
    type Kind: Copy;
    type Extra: Clone;
    /// A type entry of a package definition, as the model holds it.
    type TypeDef;
    /// A value entry of a package definition.
    type ValueDef;
    /// A type entry of a package specification.
    type TypeSpec;
    /// A value entry of a package specification.
    type ValueSpec;
    /// The whole distribution a tree reads into.
    type File;
    /// What every file of a written tree repeats and the manifest alone does not supply to a
    /// module writer: v4's format version, which a caller chooses.
    type Version;

    /// Whether every file of a tree has to say exactly the `formatVersion` its manifest says.
    ///
    /// The shared reader holds each file to the manifest when this is set. A v4 tree does not:
    /// the distribution's version is the manifest's, and a node file may spell its own
    /// differently. A v3 tree does: every file of it says `"3.1.0"`.
    const FILES_REPEAT_MANIFEST_VERSION: bool;

    /// The root manifest.
    fn decode_manifest(
        value: &Self::Doc,
        cursor: &str,
    ) -> Result<Envelope<Self::Kind, Self::Extra>, Diagnostic>;

    /// Which role each root's modules have for a manifest of this kind.
    fn role(kind: Self::Kind, root: Root) -> Role;

    /// A module manifest, whose inline entries are read in the style `role` calls for.
    fn decode_module(
        value: &Self::Doc,
        cursor: &str,
        role: Role,
    ) -> Result<ModuleFileOf<Self>, Diagnostic>;

    /// A `.type` node file: the name it says it holds, and what it holds. The reader refuses a
    /// node of the wrong role, so a model may decode either whatever `role` says.
    fn decode_type_file(
        value: &Self::Doc,
        cursor: &str,
        role: Role,
    ) -> Result<(Name, TypeNode<Self>), Diagnostic>;

    /// A `.value` node file, the other half of [`Self::decode_type_file`].
    fn decode_value_file(
        value: &Self::Doc,
        cursor: &str,
        role: Role,
    ) -> Result<(Name, ValueNode<Self>), Diagnostic>;

    /// The distribution the manifest and the modules the reader found make together.
    fn assemble(
        envelope: Envelope<Self::Kind, Self::Extra>,
        packages: Packages<Self>,
    ) -> Result<Self::File, Diagnostic>
    where
        Self: Sized;

    /// The root manifest a tree is written with.
    ///
    /// An encoder reports a failure against the file's root (an empty cursor); the writer
    /// re-cursors it onto the file's logical path.
    fn encode_manifest(
        envelope: &Envelope<Self::Kind, Self::Extra>,
    ) -> Result<Self::Doc, Diagnostic>;

    /// A module manifest that lists its entries by name: `names` holds the type names and then
    /// the value names, in listing order, and `file_names` the names whose stem the budget cut.
    fn encode_module(
        version: &Self::Version,
        module: &ModuleHeader,
        role: Role,
        names: (&[Name], &[Name]),
        file_names: &[(Name, String)],
    ) -> Result<Self::Doc, Diagnostic>;

    /// A `.type` node file. The node is borrowed from the module being written, so a model
    /// copies at most the one entry it encodes.
    fn encode_type_file(
        version: &Self::Version,
        name: &Name,
        node: &TypeNodeRef<'_, Self>,
    ) -> Result<Self::Doc, Diagnostic>;

    /// A `.value` node file, borrowed the same way.
    fn encode_value_file(
        version: &Self::Version,
        name: &Name,
        node: &ValueNodeRef<'_, Self>,
    ) -> Result<Self::Doc, Diagnostic>;
}

/// One entry of a module: a definition, or the public face of one.
pub(crate) enum Node<D, S> {
    Def(D),
    Spec(S),
}

/// A module manifest decoded by the model `M`.
pub(crate) type ModuleFileOf<M> = ModuleFile<
    <M as TreeModel>::TypeDef,
    <M as TreeModel>::ValueDef,
    <M as TreeModel>::TypeSpec,
    <M as TreeModel>::ValueSpec,
>;
/// A type entry of the model `M`, whichever role its module was read in.
pub(crate) type TypeNode<M> = Node<<M as TreeModel>::TypeDef, <M as TreeModel>::TypeSpec>;
/// A value entry of the model `M`, whichever role its module was read in.
pub(crate) type ValueNode<M> = Node<<M as TreeModel>::ValueDef, <M as TreeModel>::ValueSpec>;
/// A type entry of the model `M`, borrowed from a module the writer is laying out.
pub(crate) type TypeNodeRef<'a, M> =
    Node<&'a <M as TreeModel>::TypeDef, &'a <M as TreeModel>::TypeSpec>;
/// A value entry of the model `M`, borrowed from a module the writer is laying out.
pub(crate) type ValueNodeRef<'a, M> =
    Node<&'a <M as TreeModel>::ValueDef, &'a <M as TreeModel>::ValueSpec>;

/// Every module the tree held, grouped by package: the distribution's own first, then each
/// dependency in manifest order.
pub(crate) struct Packages<M: TreeModel + ?Sized> {
    pub own: Vec<AssembledModule<M>>,
    pub dependencies: Vec<(PackageName, Vec<AssembledModule<M>>)>,
}

/// One module with its listings resolved: every entry, whether it came inline or from its own
/// file, in the order the manifest listed it.
///
/// Every entry has the role the module was read in: the reader refuses a node of the other one.
pub(crate) struct AssembledModule<M: TreeModel + ?Sized> {
    pub path: Path,
    pub public: bool,
    pub doc: Option<Documentation>,
    pub types: IndexMap<String, TypeNode<M>>,
    pub values: IndexMap<String, ValueNode<M>>,
}
