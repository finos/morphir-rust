//! Reading a document tree: a map of logical paths to text becomes the `IRFile` the equivalent
//! single document would have produced.
//!
//! Mirrors `IR/src/layout/read-tree.ts` in `ecosystem/morphir-typescript`; see
//! `.dev/docs/superpowers/maps/2026-09-17-reference-tree-layout-map.md` section 4. A tree is a
//! distribution taken apart, so reading one is putting it back together: the root manifest says
//! which kind it is and which packages live under `deps/`, each `…/module` file says what its
//! directory holds, and each node file is one type or one value. Nothing here parses anything
//! itself — the profile turns text into the value tree both profiles produce, and the v4 tree-file
//! decoders turn that into the model. What this module adds is what a single document does not
//! have: which file a name is in, which package a directory belongs to, and the rule that every
//! file under `pkg/` and `deps/` is claimed by exactly one module.
//!
//! A directory carries no order, so modules are assembled in logical-path order: a distribution
//! whose modules were written in some other order comes back sorted. That is the one thing a tree
//! does not preserve, and it is why a round trip is stable even though the writer never sorts.
//!
//! Two cursor conventions meet here. A file that is not there at all is reported at its bare
//! logical path; everything else is `<logical path>#<json pointer>`, whether the diagnostic is
//! about the shape of the tree or is a file reader's own, re-cursored onto the file it came from.

use std::collections::HashSet;

use indexmap::IndexMap;
use serde_json::Value as JsonValue;

use super::paths::{
    MANIFEST, NodeFileKind, PathKind, Root, VERSION_SLOT, classify, node_file_path,
};
use super::{Profile, Tree};
// The stack a whole tree read grows onto, and the headroom below which it grows one, are the JSON
// reader's own figures — borrowed rather than copied. A tree is read on one stack: the growth
// happens once, around the whole read, rather than once per file, and every file's parse then finds
// a stack deeper than `RED_ZONE` and grows no further. That only holds while the two agree, so
// there is one pair of constants rather than two.
use crate::ir::json::{READ_STACK_BYTES, RED_ZONE};
use crate::ir::v4::access::AccessControlled;
use crate::ir::v4::distribution::{
    ApplicationContent, DefinitionDependencies, Dependencies, Distribution, LibraryContent,
    SpecsContent,
};
use crate::ir::v4::module::{Documented, ModuleDefinition, ModuleSpecification};
use crate::ir::v4::package::{PackageDefinition, PackageSpecification};
use crate::ir::v4::tree_files::{
    DistributionKind, DistributionManifestFile, ExpectedEntries, ModuleEntries, ModuleManifestFile,
    NodeFileBody, TypeDefinitionFile, ValueDefinitionFile,
};
use crate::ir::v4::types::{TypeDefinition, TypeSpecification};
use crate::ir::v4::value::{ValueDefinition, ValueSpecification};
use crate::ir::v4::{IRFile, SpellingMode, serde_document, with_spelling_mode};
use crate::ir::{Diagnostic, DiagnosticCode, DiagnosticStage, Warning};
use crate::naming::{self, Name, PackageName};

/// A type entry of a package definition, as the model holds it.
type TypeDef = AccessControlled<Documented<TypeDefinition>>;
/// A value entry of a package definition.
type ValueDef = AccessControlled<Documented<ValueDefinition>>;
/// A type entry of a package specification.
type TypeSpec = Documented<TypeSpecification>;
/// A value entry of a package specification.
type ValueSpec = Documented<ValueSpecification>;

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
    stacker::maybe_grow(RED_ZONE, READ_STACK_BYTES, || {
        Reader {
            files,
            profile,
            consumed: HashSet::new(),
            warnings: Vec::new(),
        }
        .read()
    })
}

/// One tree read in progress: what it was given, what it has claimed, and what it has to say.
struct Reader<'a> {
    files: &'a Tree,
    profile: Profile,
    /// The paths a file was parsed from, recorded before the file is decoded, so a file that
    /// failed to decode is still claimed and the stray check does not blame it twice.
    consumed: HashSet<String>,
    /// Each warning with the path it came from, so the collected list can be sorted by path and
    /// not depend on the order the tree was walked in.
    warnings: Vec<(String, Warning)>,
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
}

/// The packages a tree holds: the manifest's own, then its dependencies in the manifest's order.
///
/// A directory tree does not order its dependencies; the manifest does.
struct Packages {
    own: PackageRoot,
    deps: Vec<PackageRoot>,
}

impl Packages {
    fn of(manifest: &DistributionManifestFile) -> Self {
        let own_path = naming::escaped_path(manifest.package.as_path());
        Self {
            own: PackageRoot {
                root: Root::Pkg,
                name: manifest.package.clone(),
                pkg_path: own_path.clone(),
                prefix: own_path,
            },
            deps: manifest
                .dependencies
                .iter()
                .map(|name| PackageRoot {
                    root: Root::Deps,
                    pkg_path: naming::escaped_path(name.as_path()),
                    prefix: super::paths::package_dir(Root::Deps, name),
                    name: name.clone(),
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
            .find(|p| p.root == root && dir.starts_with(&format!("{}/", p.prefix)))
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

impl Reader<'_> {
    fn read(mut self) -> Result<(IRFile, Vec<Warning>), Diagnostic> {
        if !self.files.contains_key(MANIFEST) {
            return Err(Diagnostic::new(
                DiagnosticCode::MissingMember,
                DiagnosticStage::Semantic,
                MANIFEST,
                "missing member \"manifest\"",
            ));
        }
        let manifest =
            self.read_file(MANIFEST, serde_document::decode_distribution_manifest_file)?;
        let packages = Packages::of(&manifest);
        let distribution = self.assemble(&manifest, &packages)?;

        // Everything under `pkg/` or `deps/` belongs to a module; a file no module manifest
        // claimed is in the wrong package, spelled in a way the grammar does not recognize, or
        // simply left behind, and either way the tree is not the distribution it says it is. Only
        // files outside those two roots are ignored — and this runs last, so a tree with both a
        // stray file and a defect inside a module reports the defect.
        if let Some(stray) = self.stray() {
            return Err(shape(&stray, "/", stray_message(&stray, &packages)));
        }

        self.warnings.sort_by(|left, right| left.0.cmp(&right.0));
        let warnings = self
            .warnings
            .into_iter()
            .map(|(_, warning)| warning)
            .collect();
        Ok((
            IRFile {
                format_version: manifest.format_version.clone(),
                distribution,
            },
            warnings,
        ))
    }

    /// The distribution the manifest's kind calls for.
    ///
    /// A `Specs` tree is specifications everywhere. A `Library` holds its own package's
    /// definitions and its dependencies' public faces. An `Application` links its dependencies
    /// statically, so `deps/` holds definitions there too (distributions-0010), and the entry
    /// points come from the manifest rather than from any file under a package root.
    fn assemble(
        &mut self,
        manifest: &DistributionManifestFile,
        packages: &Packages,
    ) -> Result<Distribution, Diagnostic> {
        let package_name = manifest.package.clone();
        match manifest.distribution {
            DistributionKind::Specs => {
                let spec = self.specification_package(packages, &packages.own)?;
                let dependencies = self.dependency_specifications(packages)?;
                Ok(Distribution::Specs(SpecsContent {
                    package_name,
                    dependencies,
                    spec,
                }))
            }
            DistributionKind::Library => {
                let def = self.definition_package(packages, &packages.own)?;
                let dependencies = self.dependency_specifications(packages)?;
                Ok(Distribution::Library(LibraryContent {
                    package_name,
                    dependencies,
                    def,
                }))
            }
            DistributionKind::Application => {
                let def = self.definition_package(packages, &packages.own)?;
                let dependencies = self.dependency_definitions(packages)?;
                Ok(Distribution::Application(ApplicationContent {
                    package_name,
                    dependencies,
                    def,
                    entry_points: manifest.entry_points.clone(),
                }))
            }
        }
    }

    fn dependency_specifications(
        &mut self,
        packages: &Packages,
    ) -> Result<Dependencies, Diagnostic> {
        let mut out = Dependencies::new();
        for package in &packages.deps {
            let specification = self.specification_package(packages, package)?;
            out.insert(package.name.to_canonical_string(), specification);
        }
        Ok(out)
    }

    fn dependency_definitions(
        &mut self,
        packages: &Packages,
    ) -> Result<DefinitionDependencies, Diagnostic> {
        let mut out = DefinitionDependencies::new();
        for package in &packages.deps {
            let definition = self.definition_package(packages, package)?;
            out.insert(package.name.to_canonical_string(), definition);
        }
        Ok(out)
    }

    // =========================================================================
    // Per package
    // =========================================================================

    fn definition_package(
        &mut self,
        packages: &Packages,
        package: &PackageRoot,
    ) -> Result<PackageDefinition, Diagnostic> {
        let mut modules = IndexMap::new();
        for dir in self.module_dirs(packages, package) {
            let at = Where::new(package.root, dir);
            let ModuleManifestFile {
                path,
                access,
                doc,
                types,
                values,
                file_names,
                ..
            } = self.read_module_manifest(&at, package, ExpectedEntries::Definitions)?;
            let types = self.resolve(
                &at,
                &file_names,
                definition_listing(types, &at, "types")?,
                load_type_definition,
            )?;
            let values = self.resolve(
                &at,
                &file_names,
                definition_listing(values, &at, "values")?,
                load_value_definition,
            )?;
            modules.insert(
                path.to_canonical_string(),
                AccessControlled {
                    access,
                    value: ModuleDefinition { types, values, doc },
                },
            );
        }
        Ok(PackageDefinition { modules })
    }

    fn specification_package(
        &mut self,
        packages: &Packages,
        package: &PackageRoot,
    ) -> Result<PackageSpecification, Diagnostic> {
        let mut modules = IndexMap::new();
        for dir in self.module_dirs(packages, package) {
            let at = Where::new(package.root, dir);
            let ModuleManifestFile {
                path,
                doc,
                types,
                values,
                file_names,
                ..
            } = self.read_module_manifest(&at, package, ExpectedEntries::Specifications)?;
            let types = self.resolve(
                &at,
                &file_names,
                specification_listing(types, &at, "types")?,
                load_type_specification,
            )?;
            let values = self.resolve(
                &at,
                &file_names,
                specification_listing(values, &at, "values")?,
                load_value_specification,
            )?;
            modules.insert(
                path.to_canonical_string(),
                ModuleSpecification {
                    // A tree has nowhere to keep module annotations, so a module read out of one
                    // has none; the writer refuses one that has any.
                    annotations: Vec::new(),
                    types,
                    values,
                    doc,
                },
            );
        }
        Ok(PackageSpecification { modules })
    }

    /// The module directories of one package, in logical-path order.
    ///
    /// A directory is a module exactly when it holds a `module` file; a directory of node files
    /// without one is left unclaimed and reported as such by the stray check.
    fn module_dirs(&self, packages: &Packages, package: &PackageRoot) -> Vec<String> {
        let mut dirs: Vec<String> = self
            .files
            .keys()
            .filter_map(|path| {
                let PathKind::Module { root, dir } = classify(path) else {
                    return None;
                };
                (packages.owner(root, &dir) == Some(package)).then_some(dir)
            })
            .collect();
        dirs.sort();
        dirs
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
        expect: ExpectedEntries,
    ) -> Result<ModuleManifestFile, Diagnostic> {
        let manifest = self.read_file(&at.manifest_path, |value, cursor| {
            serde_document::decode_module_manifest_file(value, cursor, expect)
        })?;
        // `owner` matched a strict prefix and a separator, so the relative directory is what
        // follows both; the fallback keeps this total rather than trusting the arithmetic.
        let relative = at.dir.get(package.prefix.len() + 1..).unwrap_or_default();
        let spelled = naming::escaped_path(manifest.path.as_path());
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

    /// The file one listed name lives in, checked against the name that pointed at it.
    ///
    /// A manifest that lists a name with no file, or a file whose own name is not the one that
    /// found it, would silently rename a definition.
    fn node_file<T>(
        &mut self,
        at: &Where,
        kind: NodeFileKind,
        read: impl FnOnce(&JsonValue, &str) -> Result<T, Diagnostic>,
        name_of: impl Fn(&T) -> &Name,
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
        let file = self.read_file(&path, read)?;
        if name_of(&file) != name {
            return Err(shape(
                &path,
                "/name",
                format!(
                    "expected \"{}\", the name {} listed, found \"{}\"",
                    name.to_canonical_string(),
                    at.manifest_path,
                    name_of(&file).to_canonical_string()
                ),
            ));
        }
        Ok((path, file))
    }

    // =========================================================================
    // Per file
    // =========================================================================

    /// One file of the tree, parsed under the profile and decoded as the node it is.
    ///
    /// The path is claimed as soon as the text is parsed and before it is decoded, so a file that
    /// fails to decode is never also reported as unclaimed. The file's own diagnostics and
    /// warnings come back re-cursored onto its logical path.
    fn read_file<T>(
        &mut self,
        path: &str,
        read: impl FnOnce(&JsonValue, &str) -> Result<T, Diagnostic>,
    ) -> Result<T, Diagnostic> {
        let Some(text) = self.files.get(path) else {
            return Err(Diagnostic::new(
                DiagnosticCode::MissingMember,
                DiagnosticStage::Semantic,
                path,
                format!("missing file \"{path}\""),
            ));
        };
        let parsed = self.profile.read(text);
        self.consumed.insert(path.to_owned());
        let value = parsed.map_err(|diagnostic| recursor(path, diagnostic))?;

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
// The four loaders
// =============================================================================

fn load_type_definition(
    reader: &mut Reader<'_>,
    at: &Where,
    name: &Name,
    stem: &str,
) -> Result<TypeDef, Diagnostic> {
    let (path, file) = reader.node_file(
        at,
        NodeFileKind::Type,
        serde_document::decode_type_definition_file,
        |file: &TypeDefinitionFile| &file.name,
        name,
        stem,
    )?;
    definition_body(&path, file.body)
}

fn load_value_definition(
    reader: &mut Reader<'_>,
    at: &Where,
    name: &Name,
    stem: &str,
) -> Result<ValueDef, Diagnostic> {
    let (path, file) = reader.node_file(
        at,
        NodeFileKind::Value,
        serde_document::decode_value_definition_file,
        |file: &ValueDefinitionFile| &file.name,
        name,
        stem,
    )?;
    definition_body(&path, file.body)
}

fn load_type_specification(
    reader: &mut Reader<'_>,
    at: &Where,
    name: &Name,
    stem: &str,
) -> Result<TypeSpec, Diagnostic> {
    let (path, file) = reader.node_file(
        at,
        NodeFileKind::Type,
        serde_document::decode_type_definition_file,
        |file: &TypeDefinitionFile| &file.name,
        name,
        stem,
    )?;
    specification_body(&path, file.body)
}

fn load_value_specification(
    reader: &mut Reader<'_>,
    at: &Where,
    name: &Name,
    stem: &str,
) -> Result<ValueSpec, Diagnostic> {
    let (path, file) = reader.node_file(
        at,
        NodeFileKind::Value,
        serde_document::decode_value_definition_file,
        |file: &ValueDefinitionFile| &file.name,
        name,
        stem,
    )?;
    specification_body(&path, file.body)
}

/// The definition a node file carries, or the refusal a specification there earns.
fn definition_body<D, S>(path: &str, body: NodeFileBody<D, S>) -> Result<D, Diagnostic> {
    match body {
        NodeFileBody::Def(definition) => Ok(definition),
        NodeFileBody::Spec(_) => Err(shape(path, "/", "expected a definition file")),
    }
}

/// The specification a node file carries, or the refusal a definition there earns.
fn specification_body<D, S>(path: &str, body: NodeFileBody<D, S>) -> Result<S, Diagnostic> {
    match body {
        NodeFileBody::Spec(specification) => Ok(specification),
        NodeFileBody::Def(_) => Err(shape(path, "/", "expected a specification file")),
    }
}

// =============================================================================
// Listings
// =============================================================================

/// A listing read where definitions were expected.
///
/// A manifest decoded with [`ExpectedEntries::Definitions`] never comes back in the specification
/// style — which of the two an inline object is read as is decided by that argument, never guessed
/// from the shape — so the third arm cannot happen. It is still a refusal rather than an empty
/// listing: silently dropping a module's entries would turn a defect in this reader into a
/// distribution missing half of itself.
fn definition_listing<D, S>(
    entries: ModuleEntries<D, S>,
    at: &Where,
    member: &str,
) -> Result<Listing<D>, Diagnostic> {
    match entries {
        ModuleEntries::Names(names) => Ok(Listing::Names(names)),
        ModuleEntries::Definitions(items) => Ok(Listing::Inline(items)),
        ModuleEntries::Specifications(_) => Err(shape(
            &at.manifest_path,
            &format!("/{member}"),
            format!("expected definitions in {member}, found specifications"),
        )),
    }
}

/// A listing read where specifications were expected, the other half of [`definition_listing`].
fn specification_listing<D, S>(
    entries: ModuleEntries<D, S>,
    at: &Where,
    member: &str,
) -> Result<Listing<S>, Diagnostic> {
    match entries {
        ModuleEntries::Names(names) => Ok(Listing::Names(names)),
        ModuleEntries::Specifications(items) => Ok(Listing::Inline(items)),
        ModuleEntries::Definitions(_) => Err(shape(
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
fn stray_message(path: &str, packages: &Packages) -> String {
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
