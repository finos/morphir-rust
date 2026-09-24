//! The v3 document tree: a classic `Library` or `Specs` distribution laid out with the v4 tree's
//! layout, and every file saying `formatVersion: "3.1.0"`.
//!
//! The layout is the shared one — the root manifest, the `pkg/` and `deps/` roots, a `module`
//! file per module directory, one node file per type or value, the path budget, the stems and
//! `fileNames` — so this model decides only what a file says. Names, module paths and package
//! names are spelled the way a v4 tree spells them (canonical strings, which a classic word list
//! turns into and back out of losslessly); what a node file holds under `def` or `spec` is the
//! classic payload exactly as a single v3 document writes it.

use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value as JsonValue};

use super::model::{
    AssembledModule, Entries, Envelope, ModuleFile, ModuleFileOf, ModuleHeader, Node, Packages,
    Role, TreeModel, TypeNode, TypeNodeRef, ValueNode, ValueNodeRef,
};
use super::paths::{MANIFEST, Root, module_dir, module_manifest_path, package_dir};
use super::read::{read_tree, read_tree_with};
use super::write::{Files, TreePolicy, write_module_with};
use super::{Profile, Tree};
use crate::ir::classic::package::ModuleSpecEntry;
use crate::ir::classic::{self, Attrs};
use crate::ir::v4::IRFile;
use crate::ir::v4::module::Documentation;
use crate::ir::v4::tree_files::{MIN_PATH_BUDGET, is_escaped_stem};
use crate::ir::{Diagnostic, DiagnosticCode, DiagnosticStage, Warning};
use crate::migration::migrate_name;
use crate::naming::{ModuleName, Name, PackageName, Path};
use crate::traversal::IrCursor;

/// The `formatVersion` every file of a v3 document tree carries.
pub const V3_TREE_FORMAT_VERSION: &str = "3.1.0";

/// The two kinds of distribution a v3 document tree can hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V3Kind {
    Library,
    Specs,
}

impl V3Kind {
    fn as_str(self) -> &'static str {
        match self {
            V3Kind::Library => "Library",
            V3Kind::Specs => "Specs",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "Library" => Some(V3Kind::Library),
            "Specs" => Some(V3Kind::Specs),
            _ => None,
        }
    }
}

/// A document tree read without knowing its version first.
#[derive(Debug, Clone, PartialEq)]
pub enum AnyTree {
    V3(classic::Distribution),
    V4(IRFile),
}

type TypeDef = classic::AccessControlled<classic::Documented<classic::TypeDefinition<Attrs>>>;
type ValueDef = classic::AccessControlled<
    classic::Documented<classic::ValueDefinition<Attrs, classic::Type<Attrs>>>,
>;
type TypeSpec = classic::Documented<classic::TypeSpecification<Attrs>>;
type ValueSpec = classic::Documented<classic::ValueSpecification<Attrs>>;

/// The v3 document tree.
pub(crate) struct V3;

impl TreeModel for V3 {
    type Doc = JsonValue;
    type Kind = V3Kind;
    type Extra = ();
    type TypeDef = TypeDef;
    type ValueDef = ValueDef;
    type TypeSpec = TypeSpec;
    type ValueSpec = ValueSpec;
    type File = classic::Distribution;
    /// Every file says [`V3_TREE_FORMAT_VERSION`], so there is nothing for a caller to choose.
    type Version = ();
    const FILES_REPEAT_MANIFEST_VERSION: bool = true;

    fn decode_manifest(
        value: &JsonValue,
        cursor: &str,
    ) -> Result<Envelope<V3Kind, ()>, Diagnostic> {
        let members = members_of(value, cursor, "a distribution manifest")?;
        check_version(members, cursor)?;
        if members.contains_key("entryPoints") {
            return Err(Diagnostic::normalization(
                DiagnosticCode::UnknownMember,
                format!("{cursor}/entryPoints"),
                "unknown member \"entryPoints\": a v3 distribution has no entry points",
            ));
        }
        check_members(
            members,
            cursor,
            &[
                "formatVersion",
                "distribution",
                "package",
                "pathBudget",
                "dependencies",
            ],
        )?;

        // Every required member is fetched before any of them is read, so a file missing one
        // answers `missing_member` rather than whatever a member that is present says.
        let written_kind = required(members, "distribution", cursor)?;
        let written_package = required(members, "package", cursor)?;
        let written_budget = required(members, "pathBudget", cursor)?;

        let kind_at = format!("{cursor}/distribution");
        let kind_text = string(written_kind, &kind_at)?;
        let kind = V3Kind::parse(kind_text).ok_or_else(|| {
            invalid_distribution_shape(&kind_at, format!("unknown distribution \"{kind_text}\""))
        })?;

        let package_at = format!("{cursor}/package");
        let package = package_name(string(written_package, &package_at)?, &package_at)?;
        let path_budget = path_budget(written_budget, &format!("{cursor}/pathBudget"))?;
        let dependencies = match members.get("dependencies") {
            None => Vec::new(),
            Some(written) => {
                dependency_names(written, &format!("{cursor}/dependencies"), &package)?
            }
        };

        Ok(Envelope {
            kind,
            package,
            path_budget,
            dependencies,
            extra: (),
        })
    }

    /// A `Library` holds its own package's definitions and its dependencies' public faces; a
    /// `Specs` tree is specifications everywhere.
    fn role(kind: V3Kind, root: Root) -> Role {
        match (kind, root) {
            (V3Kind::Library, Root::Pkg) => Role::Definitions,
            (V3Kind::Library, Root::Deps) | (V3Kind::Specs, _) => Role::Specifications,
        }
    }

    fn decode_module(
        value: &JsonValue,
        cursor: &str,
        role: Role,
    ) -> Result<ModuleFileOf<Self>, Diagnostic> {
        let members = members_of(value, cursor, "a module manifest")?;
        check_version(members, cursor)?;
        check_members(
            members,
            cursor,
            &[
                "formatVersion",
                "path",
                "access",
                "doc",
                "types",
                "values",
                "fileNames",
            ],
        )?;

        let path_at = format!("{cursor}/path");
        let path = ModuleName::from_canonical_string(string(
            required(members, "path", cursor)?,
            &path_at,
        )?)
        .map_err(|error| Diagnostic::normalization(DiagnosticCode::InvalidPath, &path_at, error))?
        .into_path();

        let public = match (members.get("access"), role) {
            (None, _) => true,
            // A specification is the public face of a module, so it has no access to record.
            (Some(_), Role::Specifications) => {
                return Err(Diagnostic::normalization(
                    DiagnosticCode::UnknownMember,
                    format!("{cursor}/access"),
                    "unexpected member access: a specification module has no access",
                ));
            }
            (Some(written), Role::Definitions) => match written.as_str() {
                Some("Public") => true,
                Some("Private") => false,
                _ => {
                    return Err(Diagnostic::normalization(
                        DiagnosticCode::InvalidAccess,
                        format!("{cursor}/access"),
                        "access is Public or Private",
                    ));
                }
            },
        };

        let doc = match members.get("doc") {
            None => None,
            Some(written) => Some(module_doc(written, &format!("{cursor}/doc"))?),
        };
        let types = entries(
            members,
            "types",
            cursor,
            role,
            decode_definition,
            decode_specification,
        )?;
        let values = entries(
            members,
            "values",
            cursor,
            role,
            decode_definition,
            decode_specification,
        )?;
        let mut listed = listed_names(&types);
        listed.extend(listed_names(&values));
        let file_names = match members.get("fileNames") {
            None => Vec::new(),
            Some(written) => file_names(written, &format!("{cursor}/fileNames"), &listed)?,
        };

        Ok(ModuleFile {
            path,
            public,
            doc,
            types,
            values,
            file_names,
        })
    }

    fn decode_type_file(
        value: &JsonValue,
        cursor: &str,
        role: Role,
    ) -> Result<(Name, TypeNode<Self>), Diagnostic> {
        node_file(value, cursor, role, decode_definition, decode_specification)
    }

    fn decode_value_file(
        value: &JsonValue,
        cursor: &str,
        role: Role,
    ) -> Result<(Name, ValueNode<Self>), Diagnostic> {
        node_file(value, cursor, role, decode_definition, decode_specification)
    }

    fn assemble(
        envelope: Envelope<V3Kind, ()>,
        packages: Packages<Self>,
    ) -> Result<classic::Distribution, Diagnostic> {
        let Packages { own, dependencies } = packages;
        let package = classic_path(envelope.package.as_path());
        let dependencies = dependencies
            .into_iter()
            .map(|(name, modules)| {
                Ok((
                    classic_path(name.as_path()),
                    specification_package(modules)?,
                ))
            })
            .collect::<Result<Vec<_>, Diagnostic>>()?;
        let distribution = match envelope.kind {
            V3Kind::Library => {
                classic::DistributionBody::Library(package, dependencies, definition_package(own)?)
            }
            V3Kind::Specs => {
                classic::DistributionBody::Specs(package, dependencies, specification_package(own)?)
            }
        };
        Ok(classic::Distribution {
            format_version: 3,
            distribution,
        })
    }

    fn encode_manifest(envelope: &Envelope<V3Kind, ()>) -> Result<JsonValue, Diagnostic> {
        let mut file = Map::new();
        file.insert("formatVersion".into(), V3_TREE_FORMAT_VERSION.into());
        file.insert("distribution".into(), envelope.kind.as_str().into());
        file.insert(
            "package".into(),
            envelope.package.to_canonical_string().into(),
        );
        file.insert("pathBudget".into(), envelope.path_budget.into());
        if !envelope.dependencies.is_empty() {
            file.insert(
                "dependencies".into(),
                envelope
                    .dependencies
                    .iter()
                    .map(|name| JsonValue::from(name.to_canonical_string()))
                    .collect(),
            );
        }
        Ok(JsonValue::Object(file))
    }

    fn encode_module(
        _version: &(),
        module: &ModuleHeader,
        _role: Role,
        (types, values): (&[Name], &[Name]),
        file_names: &[(Name, String)],
    ) -> Result<JsonValue, Diagnostic> {
        let mut file = Map::new();
        file.insert("formatVersion".into(), V3_TREE_FORMAT_VERSION.into());
        file.insert(
            "path".into(),
            ModuleName::new(module.path.clone())
                .to_canonical_string()
                .into(),
        );
        if !module.public {
            file.insert("access".into(), "Private".into());
        }
        if let Some(doc) = &module.doc {
            file.insert("doc".into(), doc.text().into());
        }
        file.insert("types".into(), names(types));
        file.insert("values".into(), names(values));
        if !file_names.is_empty() {
            file.insert(
                "fileNames".into(),
                JsonValue::Object(
                    file_names
                        .iter()
                        .map(|(name, stem)| (name.to_canonical_string(), stem.as_str().into()))
                        .collect(),
                ),
            );
        }
        Ok(JsonValue::Object(file))
    }

    fn encode_type_file(
        _version: &(),
        name: &Name,
        node: &TypeNodeRef<'_, Self>,
    ) -> Result<JsonValue, Diagnostic> {
        encode_node(name, node)
    }

    fn encode_value_file(
        _version: &(),
        name: &Name,
        node: &ValueNodeRef<'_, Self>,
    ) -> Result<JsonValue, Diagnostic> {
        encode_node(name, node)
    }
}

// =============================================================================
// Reading and writing whole trees
// =============================================================================

/// Reads a v3 document tree into the classic distribution it lays out, with the warnings its
/// files produced.
///
/// The layout's rules — the order of checks, the claims, the stray check and the cursors — are
/// the v4 tree's; see [`read_tree`]. A tree whose manifest does not say `"3.1.0"` is refused at
/// `manifest#/formatVersion`, and so is any other file that does not say what the manifest says.
pub fn read_tree_v3(
    files: &Tree,
    profile: Profile,
) -> Result<(classic::Distribution, Vec<Warning>), Diagnostic> {
    read_tree_with::<V3>(files, &|text| profile.read(text))
}

/// Reads a document tree of either version, chosen by the manifest's `formatVersion`: a v3
/// version (`3`, or a string that starts `3.`) reads as a v3 tree and anything else as a v4 one.
///
/// A manifest that is missing or does not parse is left to the v4 reader, which says why.
pub fn read_any_tree(
    files: &Tree,
    profile: Profile,
) -> Result<(AnyTree, Vec<Warning>), Diagnostic> {
    let says_v3 = files
        .get(MANIFEST)
        .and_then(|text| profile.read(text).ok())
        .and_then(|manifest| manifest.get("formatVersion").map(names_v3))
        .unwrap_or(false);
    if says_v3 {
        read_tree_v3(files, profile).map(|(file, warnings)| (AnyTree::V3(file), warnings))
    } else {
        read_tree(files, profile).map(|(file, warnings)| (AnyTree::V4(file), warnings))
    }
}

fn names_v3(version: &JsonValue) -> bool {
    match version {
        JsonValue::String(text) => text == "3" || text.starts_with("3."),
        JsonValue::Number(number) => number.as_u64() == Some(3),
        _ => false,
    }
}

/// Lays a classic `Library` or `Specs` distribution out as a v3 document tree under `policy`.
///
/// The list is the tree in emission order: the manifest, the own package's modules, then each
/// dependency's in the order the distribution lists them. Modules keep the order the distribution
/// carries them in; a reader sorts on the way in.
///
/// Fails with `invalid_distribution_shape` when the path budget cannot hold the tree, when a
/// dependency names the distribution's own package or another dependency, or when a module
/// lists two entries of one kind under the same name; and with `invalid_name` when a classic
/// name has no canonical spelling.
pub fn write_tree_v3(
    file: &classic::Distribution,
    policy: &TreePolicy,
) -> Result<Vec<(String, String)>, Diagnostic> {
    let (kind, package, dependencies) = match &file.distribution {
        classic::DistributionBody::Library(package, dependencies, _) => {
            (V3Kind::Library, package, dependencies)
        }
        classic::DistributionBody::Specs(package, dependencies, _) => {
            (V3Kind::Specs, package, dependencies)
        }
    };
    let package = PackageName::new(canonical_path(package, MANIFEST)?);
    let mut dependency_names: Vec<PackageName> = Vec::with_capacity(dependencies.len());
    for (name, _) in dependencies {
        let name = PackageName::new(canonical_path(name, MANIFEST)?);
        if name == package {
            return Err(invalid_distribution_shape(MANIFEST, OWN_PACKAGE_DEPENDENCY));
        }
        if dependency_names.contains(&name) {
            return Err(invalid_distribution_shape(
                MANIFEST,
                format!("duplicate dependency \"{}\"", name.to_canonical_string()),
            ));
        }
        dependency_names.push(name);
    }

    let mut out = Files::default();
    out.set(write_v3_manifest(kind, &package, &dependency_names, policy));
    match &file.distribution {
        classic::DistributionBody::Library(_, _, definition) => {
            for module in &definition.modules {
                for file in write_v3_definition_module(Root::Pkg, &package, module, policy)? {
                    out.set(file);
                }
            }
        }
        classic::DistributionBody::Specs(_, _, specification) => {
            for module in &specification.modules {
                for file in write_v3_specification_module(Root::Pkg, &package, module, policy)? {
                    out.set(file);
                }
            }
        }
    }
    for ((_, specification), name) in dependencies.iter().zip(&dependency_names) {
        for module in &specification.modules {
            for file in write_v3_specification_module(Root::Deps, name, module, policy)? {
                out.set(file);
            }
        }
    }
    Ok(out.into_vec())
}

/// The root manifest of a v3 document tree.
///
/// Total, because a manifest carries only names, a kind and a number; the fallback below is
/// unreachable rather than a case to handle.
pub fn write_v3_manifest(
    kind: V3Kind,
    package: &PackageName,
    dependencies: &[PackageName],
    policy: &TreePolicy,
) -> (String, String) {
    let envelope = Envelope {
        kind,
        package: package.clone(),
        path_budget: policy.path_budget,
        dependencies: dependencies.to_vec(),
        extra: (),
    };
    let value = V3::encode_manifest(&envelope).unwrap_or(JsonValue::Null);
    (MANIFEST.to_owned(), policy.profile.write(&value))
}

/// Lays one classic module definition out as its manifest and one file per type and value, in
/// the order the module lists them.
pub fn write_v3_definition_module(
    root: Root,
    package: &PackageName,
    module: &classic::ModuleEntry<Attrs, classic::Type<Attrs>>,
    policy: &TreePolicy,
) -> Result<Vec<(String, String)>, Diagnostic> {
    let path = canonical_path(&module.path, &package_dir(root, package))?;
    let manifest_path = module_manifest_path(root, &module_dir(root, package, &path));
    let definition = &module.definition.value;
    let header = ModuleHeader {
        path,
        public: module.definition.access == classic::Access::Public,
        doc: definition.doc.as_deref().map(Documentation::new),
    };
    write_module_with::<V3>(
        root,
        package,
        &header,
        Role::Definitions,
        &nodes(&definition.types, &manifest_path, "types", Node::Def)?,
        &nodes(&definition.values, &manifest_path, "values", Node::Def)?,
        policy,
        &(),
        &|doc| policy.profile.write(doc),
    )
}

/// Lays one classic module specification out the same way, with `spec` bodies. A specification
/// publishes nothing private, so its manifest never writes an `access` member.
pub fn write_v3_specification_module(
    root: Root,
    package: &PackageName,
    module: &ModuleSpecEntry<Attrs>,
    policy: &TreePolicy,
) -> Result<Vec<(String, String)>, Diagnostic> {
    let path = canonical_path(&module.path, &package_dir(root, package))?;
    let manifest_path = module_manifest_path(root, &module_dir(root, package, &path));
    let specification = &module.specification;
    let header = ModuleHeader {
        path,
        public: true,
        doc: specification.doc.as_deref().map(Documentation::new),
    };
    write_module_with::<V3>(
        root,
        package,
        &header,
        Role::Specifications,
        &nodes(&specification.types, &manifest_path, "types", Node::Spec)?,
        &nodes(&specification.values, &manifest_path, "values", Node::Spec)?,
        policy,
        &(),
        &|doc| policy.profile.write(doc),
    )
}

/// A classic module's entries of one kind as the nodes the layout encodes, keyed by canonical
/// name in listing order. The nodes borrow the entries: only the encoder copies one, for the
/// file it is writing.
///
/// Two entries whose names spell the same canonical name would be one file, so the second is
/// refused rather than silently overwriting the first.
fn nodes<'a, T, N>(
    entries: &'a [(classic::Name, T)],
    manifest_path: &str,
    member: &str,
    node: fn(&'a T) -> N,
) -> Result<IndexMap<String, N>, Diagnostic> {
    let mut out = IndexMap::with_capacity(entries.len());
    for (name, value) in entries {
        let key = canonical_name(name, manifest_path)?.to_canonical_string();
        if out.insert(key.clone(), node(value)).is_some() {
            return Err(invalid_distribution_shape(
                manifest_path,
                format!("{member} lists \"{key}\" twice"),
            ));
        }
    }
    Ok(out)
}

// =============================================================================
// Names
// =============================================================================

/// A classic name as a canonical one: letter-fragmented initialisms collapse back into one.
fn canonical_name(name: &classic::Name, cursor: &str) -> Result<Name, Diagnostic> {
    migrate_name(name, &IrCursor::root()).map_err(|diagnostic| {
        Diagnostic::new(
            DiagnosticCode::InvalidName,
            DiagnosticStage::Semantic,
            cursor,
            diagnostic.message,
        )
    })
}

fn canonical_path(path: &classic::Path, cursor: &str) -> Result<Path, Diagnostic> {
    Ok(Path {
        segments: path
            .segments
            .iter()
            .map(|segment| canonical_name(segment, cursor))
            .collect::<Result<_, _>>()?,
    })
}

/// A canonical name as the classic word list it came from: an initialism explodes back into
/// single letters, so the two conversions round-trip.
fn classic_name(name: &Name) -> classic::Name {
    classic::Name::new(name.words())
}

fn classic_path(path: &Path) -> classic::Path {
    classic::Path::new(path.segments.iter().map(classic_name).collect())
}

// =============================================================================
// Decoding
// =============================================================================

/// The message a manifest listing its own package as a dependency earns.
const OWN_PACKAGE_DEPENDENCY: &str = "a v3 dependency cannot name the distribution package";

fn members_of<'a>(
    value: &'a JsonValue,
    cursor: &str,
    what: &str,
) -> Result<&'a Map<String, JsonValue>, Diagnostic> {
    value
        .as_object()
        .ok_or_else(|| invalid_type(cursor, format!("{what} must be an object")))
}

/// Every file of a v3 tree says `"3.1.0"`. Read before the member check, so a file of another
/// version answers `version_mismatch` rather than whatever member it has that a v3 file does not.
fn check_version(members: &Map<String, JsonValue>, cursor: &str) -> Result<(), Diagnostic> {
    let written = members.get("formatVersion").ok_or_else(|| {
        Diagnostic::normalization(
            DiagnosticCode::MissingFormatVersion,
            cursor,
            "the root has no formatVersion member",
        )
    })?;
    if written.as_str() == Some(V3_TREE_FORMAT_VERSION) {
        return Ok(());
    }
    Err(Diagnostic::normalization(
        DiagnosticCode::VersionMismatch,
        format!("{cursor}/formatVersion"),
        format!(
            "a v3 document tree's files say formatVersion \"{V3_TREE_FORMAT_VERSION}\", found {written}"
        ),
    ))
}

/// Refuses the first member not in `allowed`.
fn check_members(
    members: &Map<String, JsonValue>,
    cursor: &str,
    allowed: &[&str],
) -> Result<(), Diagnostic> {
    match members.keys().find(|key| !allowed.contains(&key.as_str())) {
        None => Ok(()),
        Some(key) => Err(Diagnostic::normalization(
            DiagnosticCode::UnknownMember,
            format!("{cursor}/{key}"),
            format!("unexpected member {key}"),
        )),
    }
}

fn required<'a>(
    members: &'a Map<String, JsonValue>,
    member: &str,
    cursor: &str,
) -> Result<&'a JsonValue, Diagnostic> {
    members.get(member).ok_or_else(|| {
        Diagnostic::normalization(
            DiagnosticCode::MissingMember,
            cursor,
            format!("missing member \"{member}\""),
        )
    })
}

fn string<'a>(value: &'a JsonValue, cursor: &str) -> Result<&'a str, Diagnostic> {
    value.as_str().ok_or_else(|| {
        invalid_type(
            cursor,
            format!("expected a string, found {}", kind_of(value)),
        )
    })
}

fn package_name(text: &str, cursor: &str) -> Result<PackageName, Diagnostic> {
    Path::from_canonical_string(text)
        .map(PackageName::new)
        .map_err(|error| Diagnostic::normalization(DiagnosticCode::InvalidPath, cursor, error))
}

/// `pathBudget`: a number first, then an integer of at least [`MIN_PATH_BUDGET`].
fn path_budget(value: &JsonValue, cursor: &str) -> Result<u32, Diagnostic> {
    let JsonValue::Number(number) = value else {
        return Err(invalid_type(
            cursor,
            format!("expected a number, found {}", kind_of(value)),
        ));
    };
    let refuse = || {
        invalid_type(
            cursor,
            format!("pathBudget must be an integer of at least {MIN_PATH_BUDGET}"),
        )
    };
    let budget = number.as_u64().ok_or_else(refuse)?;
    let budget = u32::try_from(budget).map_err(|_| refuse())?;
    if budget < MIN_PATH_BUDGET {
        return Err(refuse());
    }
    Ok(budget)
}

/// The manifest's `dependencies`: canonical package names, none twice and none the distribution's
/// own. A classic distribution lists its dependencies beside its own package, so one that names
/// itself would be the same package twice.
fn dependency_names(
    value: &JsonValue,
    cursor: &str,
    own: &PackageName,
) -> Result<Vec<PackageName>, Diagnostic> {
    let items = value
        .as_array()
        .ok_or_else(|| invalid_type(cursor, "dependencies is an array of package names"))?;
    let mut names: Vec<PackageName> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let at = format!("{cursor}/{index}");
        let name = package_name(string(item, &at)?, &at)?;
        if &name == own {
            return Err(invalid_distribution_shape(&at, OWN_PACKAGE_DEPENDENCY));
        }
        if names.contains(&name) {
            return Err(Diagnostic::normalization(
                DiagnosticCode::DuplicateMember,
                &at,
                format!("duplicate dependency \"{}\"", name.to_canonical_string()),
            ));
        }
        names.push(name);
    }
    Ok(names)
}

/// A module manifest's `doc`: one string, or an array of lines joined the way the text would
/// have read.
fn module_doc(value: &JsonValue, cursor: &str) -> Result<Documentation, Diagnostic> {
    match value {
        JsonValue::Array(items) => {
            let mut lines = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                lines.push(string(item, &format!("{cursor}/{index}"))?);
            }
            Ok(Documentation::new(lines.join("\n")))
        }
        other => string(other, cursor).map(Documentation::new),
    }
}

fn decode_name(value: &JsonValue, cursor: &str) -> Result<Name, Diagnostic> {
    let text = value
        .as_str()
        .ok_or_else(|| invalid_type(cursor, "a name must be a canonical string"))?;
    Name::from_canonical_string(text)
        .map_err(|error| Diagnostic::normalization(DiagnosticCode::InvalidName, cursor, error))
}

/// A module manifest's `types` or `values`: absent, an array of names, or an object of entries
/// read in the style `role` calls for — never guessed from the shape.
fn entries<D, S>(
    members: &Map<String, JsonValue>,
    member: &str,
    cursor: &str,
    role: Role,
    definition: fn(&JsonValue, &str) -> Result<D, Diagnostic>,
    specification: fn(&JsonValue, &str) -> Result<S, Diagnostic>,
) -> Result<Entries<D, S>, Diagnostic> {
    let Some(written) = members.get(member) else {
        return Ok(Entries::Names(Vec::new()));
    };
    let at = format!("{cursor}/{member}");
    match written {
        JsonValue::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| decode_name(item, &format!("{at}/{index}")))
            .collect::<Result<Vec<_>, _>>()
            .map(Entries::Names),
        JsonValue::Object(items) => match role {
            Role::Definitions => inline(items, &at, definition).map(Entries::Definitions),
            Role::Specifications => inline(items, &at, specification).map(Entries::Specifications),
        },
        other => Err(invalid_type(
            &at,
            format!(
                "expected an array of names or an object of entries, found {}",
                kind_of(other)
            ),
        )),
    }
}

/// An inline listing, keyed by canonical name.
fn inline<T>(
    items: &Map<String, JsonValue>,
    cursor: &str,
    decode: fn(&JsonValue, &str) -> Result<T, Diagnostic>,
) -> Result<IndexMap<String, T>, Diagnostic> {
    let mut out = IndexMap::with_capacity(items.len());
    for (key, written) in items {
        let at = format!("{cursor}/{key}");
        let name = decode_name(&JsonValue::String(key.clone()), &at)?.to_canonical_string();
        if out.contains_key(&name) {
            return Err(Diagnostic::normalization(
                DiagnosticCode::DuplicateMember,
                &at,
                format!("\"{name}\" is listed twice"),
            ));
        }
        out.insert(name, decode(written, &at)?);
    }
    Ok(out)
}

fn listed_names<D, S>(entries: &Entries<D, S>) -> Vec<String> {
    match entries {
        Entries::Names(names) => names.iter().map(Name::to_canonical_string).collect(),
        Entries::Definitions(items) => items.keys().cloned().collect(),
        Entries::Specifications(items) => items.keys().cloned().collect(),
    }
}

/// `fileNames`: the names whose stem was truncated for the path budget, each with the stem its
/// file is under. The key has to be a name the module lists and the stem has to be a stem; the
/// truncation itself is trusted, never recomputed.
fn file_names(
    value: &JsonValue,
    cursor: &str,
    listed: &[String],
) -> Result<Vec<(Name, String)>, Diagnostic> {
    let entries = members_of(value, cursor, "fileNames")?;
    let mut recorded = Vec::with_capacity(entries.len());
    for (key, written) in entries {
        let at = format!("{cursor}/{key}");
        let name = decode_name(&JsonValue::String(key.clone()), &at)?;
        if !listed.contains(&name.to_canonical_string()) {
            return Err(invalid_distribution_shape(
                &at,
                "fileNames key not listed in types or values",
            ));
        }
        let stem = string(written, &at)?;
        if !is_escaped_stem(stem) {
            return Err(Diagnostic::normalization(
                DiagnosticCode::InvalidName,
                &at,
                format!("\"{stem}\" is not an escaped stem"),
            ));
        }
        recorded.push((name, stem.to_owned()));
    }
    Ok(recorded)
}

/// A node file: a format version, a name, and exactly one of `def` and `spec`.
///
/// A body of the other role is refused before its payload is read, with the words the shared
/// reader uses for the same refusal, so what the file says is wrong does not depend on whether
/// the payload would have decoded.
fn node_file<D, S>(
    value: &JsonValue,
    cursor: &str,
    role: Role,
    definition: fn(&JsonValue, &str) -> Result<D, Diagnostic>,
    specification: fn(&JsonValue, &str) -> Result<S, Diagnostic>,
) -> Result<(Name, Node<D, S>), Diagnostic> {
    let members = members_of(value, cursor, "a node file")?;
    check_version(members, cursor)?;
    check_members(members, cursor, &["formatVersion", "name", "def", "spec"])?;
    let name = decode_name(
        required(members, "name", cursor)?,
        &format!("{cursor}/name"),
    )?;
    let node = match (members.get("def"), members.get("spec"), role) {
        (Some(_), None, Role::Specifications) => {
            return Err(wrong_role(cursor, "expected a specification file"));
        }
        (None, Some(_), Role::Definitions) => {
            return Err(wrong_role(cursor, "expected a definition file"));
        }
        (Some(written), None, Role::Definitions) => {
            Node::Def(definition(written, &format!("{cursor}/def"))?)
        }
        (None, Some(written), Role::Specifications) => {
            Node::Spec(specification(written, &format!("{cursor}/spec"))?)
        }
        _ => {
            return Err(invalid_distribution_shape(
                cursor,
                "exactly one of def or spec",
            ));
        }
    };
    Ok((name, node))
}

/// The refusal the shared reader gives a node of the other role, at the file's root.
fn wrong_role(cursor: &str, message: &str) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::InvalidDistributionShape,
        DiagnosticStage::Semantic,
        cursor,
        message,
    )
}

/// A classic definition payload: `{"access": …, "value": {"doc": …, "value": …}}`.
///
/// The classic types read their wrappers leniently — a member they do not know is dropped — so
/// the two wrapper layers are held to their members here, and a payload that says something the
/// distribution has no place for is refused rather than quietly shortened.
fn decode_definition<T: DeserializeOwned>(
    value: &JsonValue,
    cursor: &str,
) -> Result<classic::AccessControlled<classic::Documented<T>>, Diagnostic> {
    let members = members_of(value, cursor, "a definition")?;
    check_members(members, cursor, &["access", "value"])?;
    if let Some(documented) = members.get("value") {
        let at = format!("{cursor}/value");
        check_members(
            members_of(documented, &at, "a documented definition")?,
            &at,
            &["doc", "value"],
        )?;
    }
    classic_payload(value, cursor)
}

/// A classic specification payload: `{"doc": …, "value": …}`, held to its members the same way.
fn decode_specification<T: DeserializeOwned>(
    value: &JsonValue,
    cursor: &str,
) -> Result<classic::Documented<T>, Diagnostic> {
    let members = members_of(value, cursor, "a specification")?;
    if members.contains_key("access") {
        return Err(invalid_distribution_shape(
            cursor,
            "expected a specification, found an access-controlled definition",
        ));
    }
    check_members(members, cursor, &["doc", "value"])?;
    classic_payload(value, cursor)
}

fn classic_payload<T: DeserializeOwned>(value: &JsonValue, cursor: &str) -> Result<T, Diagnostic> {
    T::deserialize(value).map_err(|error| invalid_type(cursor, error.to_string()))
}

fn invalid_type(cursor: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::normalization(DiagnosticCode::InvalidType, cursor, message)
}

fn invalid_distribution_shape(cursor: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::normalization(DiagnosticCode::InvalidDistributionShape, cursor, message)
}

fn kind_of(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "boolean",
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}

// =============================================================================
// Encoding
// =============================================================================

fn names(names: &[Name]) -> JsonValue {
    names
        .iter()
        .map(|name| JsonValue::from(name.to_canonical_string()))
        .collect()
}

/// A node file, with the classic payload exactly as a single v3 document writes it. Every payload
/// is a model value that serializes, so a failure is a defect in a node; it is reported at the
/// file's root, and the writer says which file.
fn encode_node<D: serde::Serialize, S: serde::Serialize>(
    name: &Name,
    node: &Node<&D, &S>,
) -> Result<JsonValue, Diagnostic> {
    let (member, payload) = match node {
        Node::Def(definition) => ("def", serde_json::to_value(definition)),
        Node::Spec(specification) => ("spec", serde_json::to_value(specification)),
    };
    let payload = payload.map_err(|error| invalid_distribution_shape("", error.to_string()))?;
    let mut file = Map::new();
    file.insert("formatVersion".into(), V3_TREE_FORMAT_VERSION.into());
    file.insert("name".into(), name.to_canonical_string().into());
    file.insert(member.into(), payload);
    Ok(JsonValue::Object(file))
}

// =============================================================================
// Assembling
// =============================================================================

type Module = AssembledModule<V3>;

fn definition_package(
    modules: Vec<Module>,
) -> Result<classic::PackageDefinition<Attrs, classic::Type<Attrs>>, Diagnostic> {
    let mut out = Vec::with_capacity(modules.len());
    for module in modules {
        let key = ModuleName::new(module.path.clone()).to_canonical_string();
        out.push(classic::ModuleEntry {
            path: classic_path(&module.path),
            definition: classic::AccessControlled {
                access: if module.public {
                    classic::Access::Public
                } else {
                    classic::Access::Private
                },
                value: classic::ModuleDefinition {
                    types: definitions(module.types, &key)?,
                    values: definitions(module.values, &key)?,
                    doc: module.doc.map(|doc| doc.text().to_owned()),
                },
            },
        });
    }
    Ok(classic::PackageDefinition { modules: out })
}

fn specification_package(
    modules: Vec<Module>,
) -> Result<classic::PackageSpecification<Attrs>, Diagnostic> {
    let mut out = Vec::with_capacity(modules.len());
    for module in modules {
        let key = ModuleName::new(module.path.clone()).to_canonical_string();
        out.push(ModuleSpecEntry {
            path: classic_path(&module.path),
            specification: classic::ModuleSpecification {
                types: specifications(module.types, &key)?,
                values: specifications(module.values, &key)?,
                doc: module.doc.map(|doc| doc.text().to_owned()),
            },
        });
    }
    Ok(classic::PackageSpecification { modules: out })
}

/// The definitions of a module read in the definitions role, each under its classic name.
///
/// The reader refuses a node of the other role before it gets here, so the refusal cannot
/// happen; it is still a refusal rather than a dropped entry.
fn definitions<D, S>(
    entries: IndexMap<String, Node<D, S>>,
    module: &str,
) -> Result<Vec<(classic::Name, D)>, Diagnostic> {
    entries
        .into_iter()
        .map(|(key, entry)| match entry {
            Node::Def(definition) => Ok((entry_name(&key, module)?, definition)),
            Node::Spec(_) => Err(assembly_role(module, "a definition", "a specification")),
        })
        .collect()
}

/// The specifications of a module read in the specifications role, the other half of
/// [`definitions`].
fn specifications<D, S>(
    entries: IndexMap<String, Node<D, S>>,
    module: &str,
) -> Result<Vec<(classic::Name, S)>, Diagnostic> {
    entries
        .into_iter()
        .map(|(key, entry)| match entry {
            Node::Spec(specification) => Ok((entry_name(&key, module)?, specification)),
            Node::Def(_) => Err(assembly_role(module, "a specification", "a definition")),
        })
        .collect()
}

/// An entry's classic name, out of the canonical key the reader filed it under.
fn entry_name(key: &str, module: &str) -> Result<classic::Name, Diagnostic> {
    Name::from_canonical_string(key)
        .map(|name| classic_name(&name))
        .map_err(|error| {
            Diagnostic::new(
                DiagnosticCode::InvalidName,
                DiagnosticStage::Semantic,
                module,
                error,
            )
        })
}

fn assembly_role(module: &str, expected: &str, found: &str) -> Diagnostic {
    Diagnostic::new(
        DiagnosticCode::InvalidDistributionShape,
        DiagnosticStage::Semantic,
        module,
        format!("expected {expected} in module \"{module}\", found {found}"),
    )
}
