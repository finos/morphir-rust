//! Reading a document tree: a map of logical paths to text becomes the distribution the equivalent
//! single document would have produced.
//!
//! Mirrors `IR/src/layout/read-tree.ts` in `ecosystem/morphir-typescript`; see
//! `.dev/docs/superpowers/maps/2026-09-17-reference-tree-layout-map.md` section 4. A tree is a
//! distribution taken apart, so reading one is putting it back together: the root manifest says
//! which kind it is and which packages live under `deps/`, each `…/module` file says what its
//! directory holds, and each node file is one type or one value. Nothing here parses anything
//! itself — the caller's parser turns text into the payload its profile produces, and the tree's
//! `TreeModel` decodes that and assembles the pieces. What this module adds is what a single
//! document does not have: which file a name is in, which package a directory belongs to, and the
//! rule that every file under `pkg/` and `deps/` is claimed by exactly one module.
//!
//! A directory carries no order, so modules are assembled in logical-path order: a distribution
//! whose modules were written in some other order comes back sorted. That is the one thing a tree
//! does not preserve, and it is why a round trip is stable even though the writer never sorts.
//!
//! Two cursor conventions meet here. A file that is not there at all is reported at its bare
//! logical path; everything else is `<logical path>#<json pointer>`, whether the diagnostic is
//! about the shape of the tree or is a file reader's own, re-cursored onto the file it came from.

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use super::model::{
    AssembledModule, Entries, Envelope, ModuleFile, ModuleFileOf, Node, Packages, Payload, Role,
    TreeModel, TypeNode, ValueNode,
};
use super::paths::{
    MANIFEST, NodeFileKind, PathKind, Root, VERSION_SLOT, classify, node_file_path,
};
use super::v4_model::V4;
use super::{Profile, Tree};
// The stack a whole tree read grows onto, and the headroom below which it grows one, are the JSON
// reader's own figures — borrowed rather than copied. A tree is read on one stack: the growth
// happens once, around the whole read, rather than once per file, and every file's parse then finds
// a stack deeper than `RED_ZONE` and grows no further. That only holds while the two agree, so
// there is one pair of constants rather than two.
use crate::ir::json::{READ_STACK_BYTES, RED_ZONE};
use crate::ir::v4::{IRFile, SpellingMode, with_spelling_mode};
use crate::ir::{Diagnostic, DiagnosticCode, DiagnosticStage, Warning};
use crate::naming::{self, Name, PackageName};

/// Reads a document tree into the [`IRFile`] the equivalent single document would have produced,
/// with the warnings its files produced.
///
/// Every file is parsed with the one `profile`: a tree has no per-file profile, so a file spelled
/// in the other one simply fails to parse and the parse diagnostic is re-cursored onto its path.
///
/// Fails at the first thing that is wrong, in the reference's order: the manifest's presence, the
/// manifest itself, then each package's module directories in sorted order, and — last of all —
/// the first file under `pkg/` or `deps/` that no module claimed.
pub fn read_tree(files: &Tree, profile: Profile) -> Result<(IRFile, Vec<Warning>), Diagnostic> {
    read_tree_with::<V4>(files, &|text| profile.read(text))
}

/// Reads a document tree whose files `M` decodes, each file's text parsed by `parse`.
///
/// [`read_tree`] is this with the v4 model and a profile's own parser; the layout rules — the
/// order of checks, the claims, the stray check and the cursors — are the same for every model.
pub(crate) fn read_tree_with<M: TreeModel>(
    files: &Tree,
    parse: &dyn Fn(&str) -> Result<M::Doc, Diagnostic>,
) -> Result<(M::File, Vec<Warning>), Diagnostic> {
    stacker::maybe_grow(RED_ZONE, READ_STACK_BYTES, || {
        Reader::<M> {
            files,
            parse,
            consumed: HashSet::new(),
            warnings: Vec::new(),
            module_dirs_by_owner: HashMap::new(),
            version: None,
        }
        .read()
    })
}

/// One tree read in progress: what it was given, what it has claimed, and what it has to say.
struct Reader<'a, M: TreeModel> {
    files: &'a Tree,
    /// Turns one file's text into the payload `M` decodes.
    parse: &'a dyn Fn(&str) -> Result<M::Doc, Diagnostic>,
    /// The paths a file was parsed from, recorded before the file is decoded, so a file that
    /// failed to decode is still claimed and the stray check does not blame it twice.
    consumed: HashSet<String>,
    /// Each warning with the path it came from, so the collected list can be sorted by path and
    /// not depend on the order the tree was walked in.
    warnings: Vec<(String, Warning)>,
    /// Every module directory, grouped by owning package, computed once the manifest names the
    /// packages — see [`PackageRoots::module_dirs_by_owner`].
    module_dirs_by_owner: HashMap<(Root, String), Vec<String>>,
    /// The manifest's `formatVersion`, as canonical JSON text, once the manifest is read: every
    /// other file of the tree has to say the same.
    version: Option<String>,
}

/// One module directory: which root it is under, the directory itself, and where its manifest is.
struct Where {
    root: Root,
    dir: String,
    manifest_path: String,
}

impl Where {
    fn new(root: Root, dir: String) -> Self {
        let manifest_path = super::paths::module_manifest_path(root, &dir);
        Self {
            root,
            dir,
            manifest_path,
        }
    }
}

/// One package the manifest named, and the directory prefix its modules sit under.
#[derive(PartialEq)]
struct PackageRoot {
    root: Root,
    name: PackageName,
    /// The escaped package path itself, with no version slot.
    pkg_path: String,
    /// The prefix every one of the package's module directories starts with: the package path
    /// under `pkg/`, and the package path plus the bare version slot under `deps/`.
    prefix: String,
    /// [`Self::prefix`] with the trailing separator, computed once here rather than reallocated on
    /// every directory it is compared against.
    prefix_slash: String,
}

/// The packages a tree holds: the manifest's own, then its dependencies in the manifest's order.
///
/// A directory tree does not order its dependencies; the manifest does.
struct PackageRoots {
    own: PackageRoot,
    deps: Vec<PackageRoot>,
}

impl PackageRoots {
    fn of<K, E>(manifest: &Envelope<K, E>) -> Self {
        let own_path = naming::escaped_path(manifest.package.as_path());
        let own_prefix_slash = format!("{own_path}/");
        Self {
            own: PackageRoot {
                root: Root::Pkg,
                name: manifest.package.clone(),
                pkg_path: own_path.clone(),
                prefix: own_path,
                prefix_slash: own_prefix_slash,
            },
            deps: manifest
                .dependencies
                .iter()
                .map(|name| {
                    let prefix = super::paths::package_dir(Root::Deps, name);
                    let prefix_slash = format!("{prefix}/");
                    PackageRoot {
                        root: Root::Deps,
                        pkg_path: naming::escaped_path(name.as_path()),
                        prefix,
                        prefix_slash,
                        name: name.clone(),
                    }
                })
                .collect(),
        }
    }

    fn all(&self) -> impl Iterator<Item = &PackageRoot> {
        std::iter::once(&self.own).chain(self.deps.iter())
    }

    /// Which package a directory belongs to: the listed one, under the same root, whose prefix it
    /// starts with — a strict prefix, so a directory equal to the package directory is not owned.
    ///
    /// Under `deps/` the prefix ends in the version slot, so a package `a` and a package `a/b` can
    /// never both prefix one directory (decision 0015); a manifest listing the same dependency
    /// twice is refused by the manifest decoder before this is ever asked. At most one package
    /// matches, so taking the first is taking the only one.
    fn owner(&self, root: Root, dir: &str) -> Option<&PackageRoot> {
        self.all()
            .find(|p| p.root == root && dir.starts_with(&p.prefix_slash))
    }

    /// Every module directory the tree holds, grouped by the package that owns it — one pass over
    /// the tree's keys rather than one rescan per package, since a directory's owner never changes
    /// between packages asking.
    fn module_dirs_by_owner(&self, files: &Tree) -> HashMap<(Root, String), Vec<String>> {
        let mut groups: HashMap<(Root, String), Vec<String>> = HashMap::new();
        for path in files.keys() {
            let PathKind::Module { root, dir } = classify(path) else {
                continue;
            };
            if let Some(owner) = self.owner(root, &dir) {
                groups
                    .entry((owner.root, owner.pkg_path.clone()))
                    .or_default()
                    .push(dir);
            }
        }
        for dirs in groups.values_mut() {
            dirs.sort();
        }
        groups
    }
}

/// A listing of one module's types or values: the names whose files have to be read, or the
/// entries the manifest wrote out inline.
///
/// Owned, because the entries are the model's: an inline listing moves into the module rather than
/// being copied out of the manifest.
enum Listing<T> {
    Names(Vec<Name>),
    Inline(IndexMap<String, T>),
}

impl<M: TreeModel> Reader<'_, M> {
    fn read(mut self) -> Result<(M::File, Vec<Warning>), Diagnostic> {
        if !self.files.contains_key(MANIFEST) {
            return Err(Diagnostic::new(
                DiagnosticCode::MissingMember,
                DiagnosticStage::Semantic,
                MANIFEST,
                "missing member \"manifest\"",
            ));
        }
        let envelope = self.read_file(MANIFEST, M::decode_manifest)?;
        let roots = PackageRoots::of(&envelope);
        self.module_dirs_by_owner = roots.module_dirs_by_owner(self.files);
        let packages = self.packages(&envelope, &roots)?;
        let file = M::assemble(envelope, packages)?;

        // Everything under `pkg/` or `deps/` belongs to a module; a file no module manifest
        // claimed is in the wrong package, spelled in a way the grammar does not recognize, or
        // simply left behind, and either way the tree is not the distribution it says it is. Only
        // files outside those two roots are ignored — and this runs last, so a tree with both a
        // stray file and a defect inside a module reports the defect.
        if let Some(stray) = self.stray() {
            return Err(shape(&stray, "/", stray_message(&stray, &roots)));
        }

        self.warnings.sort_by(|left, right| left.0.cmp(&right.0));
        let warnings = self
            .warnings
            .into_iter()
            .map(|(_, warning)| warning)
            .collect();
        Ok((file, warnings))
    }

    /// Every module of every package the manifest named: its own package first, then each
    /// dependency in the manifest's order, each read in the role the model gives its root for
    /// the manifest's kind.
    fn packages(
        &mut self,
        envelope: &Envelope<M::Kind, M::Extra>,
        roots: &PackageRoots,
    ) -> Result<Packages<M>, Diagnostic> {
        let own = self.package(&roots.own, M::role(envelope.kind, Root::Pkg))?;
        let role = M::role(envelope.kind, Root::Deps);
        let mut dependencies = Vec::with_capacity(roots.deps.len());
        for package in &roots.deps {
            dependencies.push((package.name.clone(), self.package(package, role)?));
        }
        Ok(Packages { own, dependencies })
    }

    // =========================================================================
    // Per package
    // =========================================================================

    /// One package's modules, in logical-path order, each with its listings resolved.
    fn package(
        &mut self,
        package: &PackageRoot,
        role: Role,
    ) -> Result<Vec<AssembledModule<M>>, Diagnostic> {
        let mut modules = Vec::new();
        for dir in self.module_dirs(package) {
            let at = Where::new(package.root, dir);
            let ModuleFile {
                path,
                public,
                doc,
                types,
                values,
                file_names,
            } = self.read_module_manifest(&at, package, role)?;
            let types = self.resolve(
                &at,
                &file_names,
                listing(types, &at, "types", role)?,
                |reader, at, name, stem| reader.load_type(at, name, stem, role),
            )?;
            let values = self.resolve(
                &at,
                &file_names,
                listing(values, &at, "values", role)?,
                |reader, at, name, stem| reader.load_value(at, name, stem, role),
            )?;
            modules.push(AssembledModule {
                path,
                public,
                doc,
                types,
                values,
            });
        }
        Ok(modules)
    }

    /// The module directories of one package, in logical-path order.
    ///
    /// A directory is a module exactly when it holds a `module` file; a directory of node files
    /// without one is left unclaimed and reported as such by the stray check.
    fn module_dirs(&self, package: &PackageRoot) -> Vec<String> {
        self.module_dirs_by_owner
            .get(&(package.root, package.pkg_path.clone()))
            .cloned()
            .unwrap_or_default()
    }

    // =========================================================================
    // Per module
    // =========================================================================

    /// A module manifest, checked against the directory it was found in.
    ///
    /// The directory is the authority on where a module lives: a manifest that disagrees would put
    /// the same module in two places at once. The module's *name*, on the other hand, is the
    /// manifest's, because the escaped directory cannot tell a word from an initialism.
    fn read_module_manifest(
        &mut self,
        at: &Where,
        package: &PackageRoot,
        role: Role,
    ) -> Result<ModuleFileOf<M>, Diagnostic> {
        let manifest = self.read_file(&at.manifest_path, |value, cursor| {
            M::decode_module(value, cursor, role)
        })?;
        // `owner` matched a strict prefix and a separator, so the relative directory is what
        // follows both; the fallback keeps this total rather than trusting the arithmetic.
        let relative = at.dir.get(package.prefix.len() + 1..).unwrap_or_default();
        let spelled = naming::escaped_path(&manifest.path);
        if spelled != relative {
            return Err(shape(
                &at.manifest_path,
                "/path",
                format!("module path \"{spelled}\" does not match its directory \"{relative}\""),
            ));
        }
        Ok(manifest)
    }

    /// One listing, resolved to the entries it names.
    ///
    /// A names-style listing reads one file per name, in the order the manifest listed them; an
    /// inline listing is already the entries themselves. The model keys entries by their canonical
    /// name, so a listing that names one name twice reads its file twice and keeps one entry.
    fn resolve<T>(
        &mut self,
        at: &Where,
        file_names: &[(Name, String)],
        listing: Listing<T>,
        load: impl Fn(&mut Self, &Where, &Name, &str) -> Result<T, Diagnostic>,
    ) -> Result<IndexMap<String, T>, Diagnostic> {
        match listing {
            Listing::Inline(items) => Ok(items),
            Listing::Names(names) => {
                let mut out = IndexMap::with_capacity(names.len());
                for name in names {
                    let value = load(self, at, &name, &stem_of(file_names, &name))?;
                    out.insert(name.to_canonical_string(), value);
                }
                Ok(out)
            }
        }
    }

    /// One listed type's `.type` file, refused when it holds the other role's node.
    fn load_type(
        &mut self,
        at: &Where,
        name: &Name,
        stem: &str,
        role: Role,
    ) -> Result<TypeNode<M>, Diagnostic> {
        let (path, node) = self.node_file(
            at,
            NodeFileKind::Type,
            |value, cursor| M::decode_type_file(value, cursor, role),
            name,
            stem,
        )?;
        body(&path, node, role)
    }

    /// One listed value's `.value` file, the other half of [`Self::load_type`].
    fn load_value(
        &mut self,
        at: &Where,
        name: &Name,
        stem: &str,
        role: Role,
    ) -> Result<ValueNode<M>, Diagnostic> {
        let (path, node) = self.node_file(
            at,
            NodeFileKind::Value,
            |value, cursor| M::decode_value_file(value, cursor, role),
            name,
            stem,
        )?;
        body(&path, node, role)
    }

    /// The file one listed name lives in, checked against the name that pointed at it.
    ///
    /// A manifest that lists a name with no file, or a file whose own name is not the one that
    /// found it, would silently rename a definition.
    fn node_file<T>(
        &mut self,
        at: &Where,
        kind: NodeFileKind,
        read: impl FnOnce(&M::Doc, &str) -> Result<(Name, T), Diagnostic>,
        name: &Name,
        stem: &str,
    ) -> Result<(String, T), Diagnostic> {
        let path = node_file_path(at.root, &at.dir, stem, kind);
        if !self.files.contains_key(&path) {
            return Err(Diagnostic::new(
                DiagnosticCode::MissingMember,
                DiagnosticStage::Semantic,
                path.clone(),
                format!(
                    "{} lists \"{}\" but there is no {path}",
                    at.manifest_path,
                    name.to_canonical_string()
                ),
            ));
        }
        let (found, file) = self.read_file(&path, read)?;
        if &found != name {
            return Err(shape(
                &path,
                "/name",
                format!(
                    "expected \"{}\", the name {} listed, found \"{}\"",
                    name.to_canonical_string(),
                    at.manifest_path,
                    found.to_canonical_string()
                ),
            ));
        }
        Ok((path, file))
    }

    // =========================================================================
    // Per file
    // =========================================================================

    /// One file of the tree, parsed by the caller's parser and decoded as the node it is.
    ///
    /// The path is claimed as soon as the text is parsed and before it is decoded, so a file that
    /// fails to decode is never also reported as unclaimed. The file's own diagnostics and
    /// warnings come back re-cursored onto its logical path.
    ///
    /// Under a model whose files repeat the manifest's version
    /// ([`TreeModel::FILES_REPEAT_MANIFEST_VERSION`]), a parsed file is held to the manifest's
    /// `formatVersion` before it is decoded: the tree is one distribution, so a file of another
    /// version is refused for that alone, whatever else it says.
    fn read_file<T>(
        &mut self,
        path: &str,
        read: impl FnOnce(&M::Doc, &str) -> Result<T, Diagnostic>,
    ) -> Result<T, Diagnostic> {
        let Some(text) = self.files.get(path) else {
            return Err(Diagnostic::new(
                DiagnosticCode::MissingMember,
                DiagnosticStage::Semantic,
                path,
                format!("missing file \"{path}\""),
            ));
        };
        let parsed = (self.parse)(text);
        self.consumed.insert(path.to_owned());
        let value = parsed.map_err(|diagnostic| recursor(path, diagnostic))?;

        if M::FILES_REPEAT_MANIFEST_VERSION {
            self.agree(path, &value)?;
        }

        let (decoded, warnings) = with_spelling_mode(SpellingMode::Current, || read(&value, ""));
        for warning in warnings {
            self.warnings.push((
                path.to_owned(),
                Warning {
                    code: warning.code,
                    cursor: at(path, &warning.cursor),
                },
            ));
        }
        decoded.map_err(|diagnostic| recursor(path, diagnostic))
    }

    /// Records the manifest's `formatVersion`, or holds any other file to it.
    fn agree(&mut self, path: &str, value: &M::Doc) -> Result<(), Diagnostic> {
        let found = value.format_version();
        if path == MANIFEST {
            self.version = found;
            return Ok(());
        }
        // A file that says no version at all is left to its decoder, which answers
        // `missing_format_version` as it would for a single document.
        match (&self.version, found) {
            (Some(expected), Some(found)) if found != *expected => Err(Diagnostic::normalization(
                DiagnosticCode::VersionMismatch,
                format!("{path}#/formatVersion"),
                format!("formatVersion {found} does not match the manifest's {expected}"),
            )),
            _ => Ok(()),
        }
    }

    /// The first file under `pkg/` or `deps/` that no module claimed, in sorted order.
    ///
    /// A [`Tree`] iterates its keys in sorted order, so the first unclaimed one found is the first
    /// in sorted order — which is the one and only stray the reference reports.
    fn stray(&self) -> Option<String> {
        self.files
            .keys()
            .find(|path| !self.consumed.contains(*path) && is_under_package_root(path))
            .cloned()
    }
}

// =============================================================================
// Node files
// =============================================================================

/// The node a file carries, or the refusal a node of the other role earns: a definitions module
/// holds only definition files, and a specifications module only specification files.
fn body<D, S>(path: &str, node: Node<D, S>, role: Role) -> Result<Node<D, S>, Diagnostic> {
    match (role, node) {
        (Role::Definitions, Node::Spec(_)) => Err(shape(path, "/", "expected a definition file")),
        (Role::Specifications, Node::Def(_)) => {
            Err(shape(path, "/", "expected a specification file"))
        }
        (_, node) => Ok(node),
    }
}

// =============================================================================
// Listings
// =============================================================================

/// A listing read in the role its module has, each inline entry as the node that role holds.
///
/// A module manifest decoded in one role never comes back in the other role's inline style —
/// which of the two an inline object is read as is decided by the role, never guessed from the
/// shape — so the two mismatched arms cannot happen. They are still refusals rather than empty
/// listings: silently dropping a module's entries would turn a defect in this reader into a
/// distribution missing half of itself.
fn listing<D, S>(
    entries: Entries<D, S>,
    at: &Where,
    member: &str,
    role: Role,
) -> Result<Listing<Node<D, S>>, Diagnostic> {
    match (entries, role) {
        (Entries::Names(names), _) => Ok(Listing::Names(names)),
        (Entries::Definitions(items), Role::Definitions) => Ok(Listing::Inline(
            items
                .into_iter()
                .map(|(key, item)| (key, Node::Def(item)))
                .collect(),
        )),
        (Entries::Specifications(items), Role::Specifications) => Ok(Listing::Inline(
            items
                .into_iter()
                .map(|(key, item)| (key, Node::Spec(item)))
                .collect(),
        )),
        (Entries::Specifications(_), Role::Definitions) => Err(shape(
            &at.manifest_path,
            &format!("/{member}"),
            format!("expected definitions in {member}, found specifications"),
        )),
        (Entries::Definitions(_), Role::Specifications) => Err(shape(
            &at.manifest_path,
            &format!("/{member}"),
            format!("expected specifications in {member}, found definitions"),
        )),
    }
}

/// The stem a name's file is under: the one the manifest recorded for a name the path budget
/// truncated, the escaped name otherwise.
///
/// A reader trusts `fileNames`. It never recomputes the truncation and never checks that the
/// recorded stem is the one the budget would have produced — the manifest is what says where a
/// file is.
fn stem_of(file_names: &[(Name, String)], name: &Name) -> String {
    let canonical = name.to_canonical_string();
    file_names
        .iter()
        .find(|(listed, _)| listed.to_canonical_string() == canonical)
        .map(|(_, stem)| stem.clone())
        .unwrap_or_else(|| naming::file_stem(name))
}

// =============================================================================
// Cursors and the stray message
// =============================================================================

/// Whether a logical path is one a distribution owns, whatever shape it has.
///
/// A plain string test, not a classification: a path under those two roots that the grammar does
/// not recognize is still the distribution's to account for.
fn is_under_package_root(path: &str) -> bool {
    path.starts_with("pkg/") || path.starts_with("deps/")
}

/// A diagnostic about the tree itself rather than about the inside of one file: the cursor is the
/// logical path, with the file's own pointer after `#`.
fn shape(path: &str, pointer: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::InvalidDistributionShape,
        DiagnosticStage::Semantic,
        format!("{path}#{pointer}"),
        message,
    )
}

/// A file's own diagnostic, re-cursored onto the path it came from. The code, stage, message and
/// location stay the file decoder's; only the cursor grows a prefix.
fn recursor(path: &str, diagnostic: Diagnostic) -> Diagnostic {
    Diagnostic {
        cursor: at(path, &diagnostic.cursor),
        ..diagnostic
    }
}

/// A cursor inside one file: the file's logical path, then the pointer, with a root pointer
/// spelled `/`.
fn at(path: &str, cursor: &str) -> String {
    let pointer = if cursor.is_empty() { "/" } else { cursor };
    format!("{path}#{pointer}")
}

/// The message for a file no module claimed.
///
/// A `deps/` directory whose leading segments match a listed dependency is missing or misspelling
/// the version slot, and the more useful of the three wordings says which; anything else belongs
/// to no listed package at all, the way any unclaimed file does.
fn stray_message(path: &str, packages: &PackageRoots) -> String {
    const GENERIC: &str = "file belongs to no module";

    let (root, dir) = match classify(path) {
        PathKind::Module { root, dir } => (root, dir),
        PathKind::Type { root, dir, .. } | PathKind::Value { root, dir, .. } => (root, dir),
        PathKind::Manifest | PathKind::Other => return GENERIC.to_owned(),
    };
    if root != Root::Deps {
        return GENERIC.to_owned();
    }

    for package in packages.all() {
        if package.root != Root::Deps {
            continue;
        }
        if dir != package.pkg_path && !dir.starts_with(&format!("{}/", package.pkg_path)) {
            continue;
        }
        let segment = dir
            .get(package.pkg_path.len() + 1..)
            .unwrap_or_default()
            .split('/')
            .next()
            .unwrap_or_default();
        if segment.starts_with(VERSION_SLOT) && segment != VERSION_SLOT {
            return format!(
                "the dependency directory's version segment \"{segment}\" carries a version, but \
                 the v4 model has no package version to hold (decision 0015); expected a bare \
                 \"{VERSION_SLOT}\""
            );
        }
    }
    format!(
        "{GENERIC}; a dependency directory expects a version segment (\"{VERSION_SLOT}\") after \
         the package path"
    )
}
