//! Writing a document tree: a distribution becomes a list of logical paths and their text.
//!
//! Mirrors `IR/src/layout/write-tree.ts` in `ecosystem/morphir-typescript`; see
//! `.dev/docs/superpowers/maps/2026-09-17-reference-tree-layout-map.md` section 5. Nothing here
//! touches a filesystem: the profile is the only thing that decides what the text looks like, and
//! the result is an ordered list a caller writes, streams or compares as it likes.
//!
//! The canonical layout is the manifest style: one definition per file, the module manifest
//! listing names rather than inlining them, and `fileNames` present only for the names the path
//! budget had to cut. The order is the order the specification gives — the distribution manifest,
//! then each module's own manifest followed by its types and then its values, the own package
//! before the dependencies — and modules keep the order the model carries them in, never sorted:
//! a reader sorts on the way in, so a round trip is stable without the writer reordering anything.
//!
//! The budget is the reason writing a tree can fail at all. A path is measured physically,
//! extension included, from the distribution root; when a stem cannot be cut small enough, or when
//! the module directory alone is already over, there is no tree to write. The budget's *floor* is
//! a reader's rule, not a writer's: a small budget earns a refusal here only by producing a path
//! that does not fit.
//!
//! The per-module writers are public and take one module each, so a caller streaming a
//! distribution can emit a module's files without holding the whole tree.
//!
//! What a file says is the version's business: the layout here — the budget, the stems, the order
//! — hands a [`TreeModel`] the pieces to encode, and the profile renders what the model returns.

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;

use super::Profile;
use super::model::{Envelope, ModuleHeader, Node, Role, TreeModel, TypeNodeRef, ValueNodeRef};
use super::paths::{
    MANIFEST, NodeFileKind, Root, module_dir, module_dir_prefix, module_manifest_path,
    node_file_path, package_dir, to_physical,
};
use super::stems::stem_for;
use super::v4_model::{V4, V4Extra};
use crate::ir::v4::access::{Access, AccessControlled};
use crate::ir::v4::distribution::{Distribution, EntryPoints};
use crate::ir::v4::module::{ModuleDefinition, ModuleSpecification};
use crate::ir::v4::package::{PackageDefinition, PackageSpecification};
use crate::ir::v4::tree_files::DistributionKind;
use crate::ir::v4::{DocumentMeta, FormatVersion, IRFile, LinkedMetadataCarrier};
use crate::ir::{Diagnostic, DiagnosticCode, DiagnosticStage};
use crate::naming::{ModuleName, Name, PackageName};

/// How a distribution is laid out: which profile spells it, and how long a path may be.
///
/// The budget is a count of characters of the *physical* path from the distribution root, the
/// profile's extension included. There is no floor here: [`crate::ir::v4::tree_files::MIN_PATH_BUDGET`]
/// is what a reader holds a manifest to, and enforcing it again on write would refuse a tree the
/// reference writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreePolicy {
    pub profile: Profile,
    pub path_budget: u32,
}

/// One entry of a module, paired with the file stem the budget gave it.
struct Stem<'a, T> {
    name: Name,
    value: &'a T,
    stem: String,
    truncated: bool,
}

/// Everything the distribution manifest says, held apart from the distribution it describes.
///
/// A distribution manifest names its package, kind, dependencies, entry points, and optional 4.1
/// document metadata — never a module — so a caller streaming one module at a time can write the
/// manifest at the end. [`write_manifest`] builds one from a complete distribution; a streaming
/// writer builds one from its header, metadata event, and dependency names.
#[derive(Debug, Clone, PartialEq)]
pub struct ManifestHeader {
    pub format_version: FormatVersion,
    pub distribution: DistributionKind,
    pub package: PackageName,
    /// The dependency packages, in the order the distribution lists them.
    pub dependencies: Vec<PackageName>,
    /// An application's entry points; empty on the other two kinds.
    pub entry_points: EntryPoints,
    /// The 4.1 document graph and its source records, owned by the manifest.
    pub metadata: Option<Box<DocumentMeta>>,
}

/// The distribution manifest a header spells: the tree's root file, and the only one that names
/// the whole.
///
/// Refuses metadata on a pre-4.1 manifest instead of publishing an invalid or empty file.
pub fn write_manifest_header(
    header: &ManifestHeader,
    policy: &TreePolicy,
) -> Result<(String, String), Diagnostic> {
    let envelope = Envelope {
        kind: header.distribution,
        package: header.package.clone(),
        path_budget: policy.path_budget,
        dependencies: header.dependencies.clone(),
        extra: V4Extra {
            format_version: header.format_version.clone(),
            entry_points: header.entry_points.clone(),
            metadata: header.metadata.clone(),
        },
    };
    let value = V4::encode_manifest(&envelope)?;
    Ok((MANIFEST.to_owned(), policy.profile.write(&value)))
}

/// The distribution manifest of a whole distribution.
pub fn write_manifest(file: &IRFile, policy: &TreePolicy) -> Result<(String, String), Diagnostic> {
    write_manifest_header(&manifest_header(file), policy)
}

/// The header a complete distribution carries.
fn manifest_header(file: &IRFile) -> ManifestHeader {
    ManifestHeader {
        format_version: file.format_version.clone(),
        distribution: kind_of(&file.distribution),
        package: file.distribution.package_name().clone(),
        dependencies: dependency_names(&file.distribution),
        entry_points: entry_points_of(&file.distribution),
        metadata: file.metadata.clone(),
    }
}

/// Lays one module definition out as its manifest and one file per type and value.
///
/// The files come back in the reference's emission order: the module manifest first, then the
/// types and then the values, each in the order the module lists them.
pub fn write_definition_module(
    root: Root,
    package: &PackageName,
    module_name: &str,
    module: &AccessControlled<ModuleDefinition>,
    format_version: &FormatVersion,
    policy: &TreePolicy,
) -> Result<Vec<(String, String)>, Diagnostic> {
    refuse_old_version_metadata(module, format_version)?;
    let path = module_path(root, package, module_name)?;
    let definition = &module.value;
    let header = ModuleHeader {
        path: path.into_path(),
        public: module.access != Access::Private,
        doc: definition.doc.clone(),
    };
    write_module_with::<V4>(
        root,
        package,
        &header,
        Role::Definitions,
        &nodes(&definition.types, Node::Def),
        &nodes(&definition.values, Node::Def),
        policy,
        format_version,
        &|doc| policy.profile.write(doc),
    )
}

/// Lays one module specification out the same way, with `spec` bodies.
///
/// A module manifest has no place for annotations, so a specification carrying any cannot be
/// written as a tree at all — and that is decided before the budget is, so a module with both
/// problems reports the one a larger budget would not fix. A specification publishes nothing
/// private, so its manifest never writes an `access` member.
pub fn write_specification_module(
    root: Root,
    package: &PackageName,
    module_name: &str,
    module: &ModuleSpecification,
    format_version: &FormatVersion,
    policy: &TreePolicy,
) -> Result<Vec<(String, String)>, Diagnostic> {
    refuse_old_version_metadata(module, format_version)?;
    let path = module_path(root, package, module_name)?;
    let dir = module_dir(root, package, path.as_path());

    if !module.annotations.is_empty() {
        return Err(Diagnostic::new(
            DiagnosticCode::InvalidDistributionShape,
            DiagnosticStage::Semantic,
            module_manifest_path(root, &dir),
            "module annotations cannot be written to a document tree",
        ));
    }

    let header = ModuleHeader {
        path: path.into_path(),
        public: true,
        doc: module.doc.clone(),
    };
    write_module_with::<V4>(
        root,
        package,
        &header,
        Role::Specifications,
        &nodes(&module.types, Node::Spec),
        &nodes(&module.values, Node::Spec),
        policy,
        format_version,
        &|doc| policy.profile.write(doc),
    )
}

/// A module's entries of one kind as the nodes a model encodes, in listing order. The nodes
/// borrow the entries: only the model's encoder copies one, for the file it is writing.
fn nodes<'a, T, N>(entries: &'a IndexMap<String, T>, node: fn(&'a T) -> N) -> IndexMap<String, N> {
    entries
        .iter()
        .map(|(key, value)| (key.clone(), node(value)))
        .collect()
}

/// Lays a whole distribution out as a document tree under `policy`.
///
/// The list is the tree in emission order: the distribution manifest, the own package's modules,
/// then the dependencies in the order the model lists them. A `Library` or `Specs` tree's
/// dependencies are package specifications; an `Application` links its dependencies statically, so
/// its `deps/` holds definitions (distributions-0010). A logical path appears exactly once, at the
/// position it was first written and carrying the last text written to it — see [`Files`].
///
/// Fails with `invalid_distribution_shape` when the path budget cannot hold the tree or when
/// linked metadata is written under an older format version.
pub fn write_tree(file: &IRFile, policy: &TreePolicy) -> Result<Vec<(String, String)>, Diagnostic> {
    if file.format_version != FormatVersion::String("4.1.0".to_owned())
        && file.has_linked_metadata()
    {
        return Err(Diagnostic::new(
            DiagnosticCode::InvalidDistributionShape,
            DiagnosticStage::Semantic,
            "",
            "linked metadata requires formatVersion 4.1.0",
        ));
    }
    let mut out = Files::default();
    out.set(write_manifest(file, policy)?);
    let format_version = &file.format_version;

    match &file.distribution {
        Distribution::Library(content) => {
            write_definition_modules(
                Root::Pkg,
                &content.package_name,
                &content.def,
                format_version,
                policy,
                &mut out,
            )?;
            for (key, specification) in &content.dependencies {
                let package = dependency_name(key)?;
                write_specification_modules(
                    Root::Deps,
                    &package,
                    specification,
                    format_version,
                    policy,
                    &mut out,
                )?;
            }
        }
        Distribution::Specs(content) => {
            write_specification_modules(
                Root::Pkg,
                &content.package_name,
                &content.spec,
                format_version,
                policy,
                &mut out,
            )?;
            for (key, specification) in &content.dependencies {
                let package = dependency_name(key)?;
                write_specification_modules(
                    Root::Deps,
                    &package,
                    specification,
                    format_version,
                    policy,
                    &mut out,
                )?;
            }
        }
        Distribution::Application(content) => {
            write_definition_modules(
                Root::Pkg,
                &content.package_name,
                &content.def,
                format_version,
                policy,
                &mut out,
            )?;
            for (key, definition) in &content.dependencies {
                let package = dependency_name(key)?;
                write_definition_modules(
                    Root::Deps,
                    &package,
                    definition,
                    format_version,
                    policy,
                    &mut out,
                )?;
            }
        }
    }

    Ok(out.into_vec())
}

fn refuse_old_version_metadata<T: LinkedMetadataCarrier>(
    value: &T,
    version: &FormatVersion,
) -> Result<(), Diagnostic> {
    if *version != FormatVersion::String("4.1.0".to_owned()) && value.contains_linked_metadata() {
        return Err(Diagnostic::new(
            DiagnosticCode::InvalidDistributionShape,
            DiagnosticStage::Semantic,
            "",
            "linked metadata requires formatVersion 4.1.0",
        ));
    }
    Ok(())
}

/// The tree as the reference accumulates it: a map keyed by logical path, iterated in the order
/// each path was *first* written.
///
/// Two module keys can escape to one directory — `user-ID` and `user--id` are the two canonical
/// encodings of one name, and nothing validates a module key on read — so two modules can write
/// the same paths. The reference's `Map.set` keeps one entry per path: the last value written,
/// under the position the path first took. Accumulating into a plain list instead would emit the
/// path twice, and a tree is a map of files.
#[derive(Default)]
pub(crate) struct Files {
    entries: Vec<(String, String)>,
    positions: HashMap<String, usize>,
}

impl Files {
    pub(crate) fn set(&mut self, (path, text): (String, String)) {
        match self.positions.get(&path) {
            Some(&at) => self.entries[at].1 = text,
            None => {
                self.positions.insert(path.clone(), self.entries.len());
                self.entries.push((path, text));
            }
        }
    }

    pub(crate) fn into_vec(self) -> Vec<(String, String)> {
        self.entries
    }
}

// =============================================================================
// Per package
// =============================================================================

fn write_definition_modules(
    root: Root,
    package: &PackageName,
    definition: &PackageDefinition,
    format_version: &FormatVersion,
    policy: &TreePolicy,
    out: &mut Files,
) -> Result<(), Diagnostic> {
    for (name, module) in &definition.modules {
        for file in write_definition_module(root, package, name, module, format_version, policy)? {
            out.set(file);
        }
    }
    Ok(())
}

fn write_specification_modules(
    root: Root,
    package: &PackageName,
    specification: &PackageSpecification,
    format_version: &FormatVersion,
    policy: &TreePolicy,
    out: &mut Files,
) -> Result<(), Diagnostic> {
    for (name, module) in &specification.modules {
        for file in write_specification_module(root, package, name, module, format_version, policy)?
        {
            out.set(file);
        }
    }
    Ok(())
}

// =============================================================================
// Per module
// =============================================================================

/// The body every per-module writer shares, whichever model `M` spells the files: the module
/// directory has to fit, then every stem, then the manifest — which is written last of the three
/// because `fileNames` is exactly the list of names the budget had to cut — and then one file per
/// entry. `render` turns what the model encodes into the profile's text.
///
/// The files come back in the reference's emission order: the module manifest first, then the
/// types and then the values, each in the order the module lists them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_module_with<M: TreeModel>(
    root: Root,
    package: &PackageName,
    header: &ModuleHeader,
    role: Role,
    types: &IndexMap<String, TypeNodeRef<'_, M>>,
    values: &IndexMap<String, ValueNodeRef<'_, M>>,
    policy: &TreePolicy,
    version: &M::Version,
    render: &dyn Fn(&M::Doc) -> String,
) -> Result<Vec<(String, String)>, Diagnostic> {
    let dir = module_dir(root, package, &header.path);
    fits(root, &dir, policy)?;
    let type_stems = stems_for(types, root, &dir, NodeFileKind::Type, policy)?;
    let value_stems = stems_for(values, root, &dir, NodeFileKind::Value, policy)?;

    let type_names: Vec<Name> = type_stems.iter().map(|s| s.name.clone()).collect();
    let value_names: Vec<Name> = value_stems.iter().map(|s| s.name.clone()).collect();
    let file_names: Vec<(Name, String)> = truncated(&type_stems)
        .chain(truncated(&value_stems))
        .collect();

    let mut out = Vec::with_capacity(1 + type_stems.len() + value_stems.len());
    let manifest_path = module_manifest_path(root, &dir);
    let manifest = M::encode_module(
        version,
        header,
        role,
        (&type_names, &value_names),
        &file_names,
    )
    .map_err(|diagnostic| in_file(&manifest_path, diagnostic))?;
    out.push((manifest_path, render(&manifest)));

    for stem in &type_stems {
        let path = node_file_path(root, &dir, &stem.stem, NodeFileKind::Type);
        let file = M::encode_type_file(version, &stem.name, stem.value)
            .map_err(|diagnostic| in_file(&path, diagnostic))?;
        out.push((path, render(&file)));
    }

    for stem in &value_stems {
        let path = node_file_path(root, &dir, &stem.stem, NodeFileKind::Value);
        let file = M::encode_value_file(version, &stem.name, stem.value)
            .map_err(|diagnostic| in_file(&path, diagnostic))?;
        out.push((path, render(&file)));
    }

    Ok(out)
}

/// The names whose stem the budget had to cut, each with the stem its file is under.
fn truncated<'a, T>(stems: &'a [Stem<'a, T>]) -> impl Iterator<Item = (Name, String)> + 'a {
    stems
        .iter()
        .filter(|stem| stem.truncated)
        .map(|stem| (stem.name.clone(), stem.stem.clone()))
}

/// The stems of one kind inside one module, in listing order.
///
/// Escaping is injective, so two untruncated stems collide only when two listing keys spell the
/// same name; two truncated ones can collide on their own. Either way, silently overwriting one
/// file with another is the one outcome worth refusing, and the cursor is the physical path the
/// second file would have taken.
fn stems_for<'a, T>(
    items: &'a IndexMap<String, T>,
    root: Root,
    dir: &str,
    kind: NodeFileKind,
    policy: &TreePolicy,
) -> Result<Vec<Stem<'a, T>>, Diagnostic> {
    let prefix = module_dir_prefix(root, dir);
    let suffix = format!(".{}{}", kind.as_str(), policy.profile.extension());
    let mut seen: HashSet<String> = HashSet::with_capacity(items.len());
    let mut out = Vec::with_capacity(items.len());

    for (key, value) in items {
        let name = entry_name(key, root, dir)?;
        let chosen = stem_for(&name, &prefix, &suffix, policy.path_budget)?;
        if seen.contains(&chosen.stem) {
            return Err(Diagnostic::new(
                DiagnosticCode::InvalidDistributionShape,
                DiagnosticStage::Semantic,
                format!("{prefix}{}{suffix}", chosen.stem),
                format!(
                    "two {} names share the file stem \"{}\"",
                    kind.as_str(),
                    chosen.stem
                ),
            ));
        }
        seen.insert(chosen.stem.clone());
        out.push(Stem {
            name,
            value,
            stem: chosen.stem,
            truncated: chosen.truncated,
        });
    }

    Ok(out)
}

/// The module directory has to fit before anything inside it can: `module` is the shortest leaf a
/// module has, so if that is already over the budget no choice of stem can rescue the module.
fn fits(root: Root, dir: &str, policy: &TreePolicy) -> Result<(), Diagnostic> {
    let physical = to_physical(&module_manifest_path(root, dir), policy.profile);
    if physical.chars().count() > policy.path_budget as usize {
        return Err(Diagnostic::new(
            DiagnosticCode::InvalidDistributionShape,
            DiagnosticStage::Semantic,
            physical.clone(),
            format!("path budget {} cannot fit {physical}", policy.path_budget),
        ));
    }
    Ok(())
}

// =============================================================================
// Names out of the model's map keys
// =============================================================================

/// A module's name, as the package's listing spells it.
///
/// The model keys its modules by their canonical string, so the name has to be parsed back out;
/// a key that does not name a module path is `invalid_path`, cursored at the module manifest the
/// key would have produced.
fn module_path(
    root: Root,
    package: &PackageName,
    module_name: &str,
) -> Result<ModuleName, Diagnostic> {
    ModuleName::from_canonical_string(module_name).map_err(|message| {
        let dir = format!("{}/{module_name}", package_dir(root, package));
        Diagnostic::new(
            DiagnosticCode::InvalidPath,
            DiagnosticStage::Semantic,
            module_manifest_path(root, &dir),
            message,
        )
    })
}

/// One type's or value's name, out of its listing key.
fn entry_name(key: &str, root: Root, dir: &str) -> Result<Name, Diagnostic> {
    Name::from_canonical_string(key).map_err(|message| {
        Diagnostic::new(
            DiagnosticCode::InvalidName,
            DiagnosticStage::Semantic,
            module_manifest_path(root, dir),
            message,
        )
    })
}

/// A dependency's package name, out of the key the distribution lists it under. The manifest is
/// where that name is spelled, so that is where an unparseable one is reported.
fn dependency_name(key: &str) -> Result<PackageName, Diagnostic> {
    PackageName::from_canonical_string(key).map_err(|message| {
        Diagnostic::new(
            DiagnosticCode::InvalidPath,
            DiagnosticStage::Semantic,
            MANIFEST,
            message,
        )
    })
}

// =============================================================================
// The distribution manifest's members
// =============================================================================

fn kind_of(distribution: &Distribution) -> DistributionKind {
    match distribution {
        Distribution::Library(_) => DistributionKind::Library,
        Distribution::Specs(_) => DistributionKind::Specs,
        Distribution::Application(_) => DistributionKind::Application,
    }
}

/// The dependency package names, in the order the distribution lists them.
///
/// The keys are canonical package names already, so this parse is a round trip; the permissive
/// parser keeps [`write_manifest`] total, and [`write_tree`] refuses an unparseable key on its own
/// when it comes to lay the dependency's files out.
fn dependency_names(distribution: &Distribution) -> Vec<PackageName> {
    let keys: Vec<&String> = match distribution {
        Distribution::Library(content) => content.dependencies.keys().collect(),
        Distribution::Specs(content) => content.dependencies.keys().collect(),
        Distribution::Application(content) => content.dependencies.keys().collect(),
    };
    keys.into_iter()
        .map(|key| PackageName::parse(key))
        .collect()
}

/// Entry points belong to an application; the other two kinds have none to write.
fn entry_points_of(distribution: &Distribution) -> EntryPoints {
    match distribution {
        Distribution::Application(content) => content.entry_points.clone(),
        Distribution::Library(_) | Distribution::Specs(_) => EntryPoints::new(),
    }
}

// =============================================================================
// The model boundary
// =============================================================================

/// A model's diagnostic about one file, cursored onto the file's logical path: the path alone for
/// a failure at the file's root, the path and the pointer otherwise.
fn in_file(path: &str, diagnostic: Diagnostic) -> Diagnostic {
    let cursor = if diagnostic.cursor.is_empty() {
        path.to_owned()
    } else {
        format!("{path}#{}", diagnostic.cursor)
    };
    Diagnostic {
        cursor,
        ..diagnostic
    }
}
