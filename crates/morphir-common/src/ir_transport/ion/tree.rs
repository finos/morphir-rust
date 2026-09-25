//! The Ion document tree: the single-file elements, one file per path.
//!
//! A tree file holds the annotated elements of the single-file spelling. The path is the scope.
//! `pkg/<package>/<module>/module` holds that module and any of its children,
//! `<stem>.type` holds one type, and `<stem>.value` holds one value; `deps/<package>/@/...` holds
//! a dependency the same way. The path supplies the package, the module, and a node's name, so a
//! file may omit them. A name that is present must match the path.
//!
//! A tree reads as the datagram that holds the same elements: the manifest's header, then each
//! dependency, then each module with its children inline, then the footer. Fragments merge by the
//! single-file rules. In a module directory the module file comes first, then the type files, then
//! the value files, each in path order. A name that is defined twice after the merge is refused.
//!
//! The writer takes the single-file datagram apart. It writes the module file with the module
//! header only, one file per type and per value, and omits every name the path supplies. A stem
//! that the path budget cut does not supply its name, so that file keeps it.

use std::collections::{BTreeMap, HashSet};

use indexmap::IndexMap;
use ion_rs::{Element, Sequence, Struct};
use morphir_core::ir::layout::{
    MANIFEST, NodeFileKind, PathKind, Root, Tree, VERSION_SLOT, classify, module_dir,
    module_dir_prefix, module_manifest_path, node_file_path, package_dir, stem_for,
};
use morphir_core::naming::{self, Name, PackageName, Path};

use super::{IonCodec, annotation_names, display_annotations, ion_text, symbol_text};
use crate::ir_transport::{Stage, TransportDiagnostic};

/// The extension every Ion tree file carries.
pub(crate) const EXTENSION: &str = ".ion";

/// The members a tree path supplies, and so removes from a normalized element.
const SCOPE_MEMBERS: [&str; 3] = ["package", "module", "name"];

// =============================================================================
// Reading
// =============================================================================

/// The files of one module directory, keyed by logical path.
#[derive(Default)]
struct ModuleFiles<'a> {
    module: Option<(&'a str, &'a str)>,
    types: Vec<(&'a str, &'a str, &'a str)>,
    values: Vec<(&'a str, &'a str, &'a str)>,
}

/// Where a module directory's elements belong.
struct Scope {
    package: PackageName,
    module: Path,
}

/// The datagram a tree reads as.
pub(super) fn read(files: &Tree) -> Result<Sequence, TransportDiagnostic> {
    let manifest = files.get(MANIFEST).ok_or_else(|| {
        error(
            "morphir::ir::ion::missing_member",
            MANIFEST,
            "the tree has no manifest",
        )
    })?;
    let mut manifest = parse(MANIFEST, manifest)?.into_iter();
    let header = manifest.next().ok_or_else(|| {
        error(
            "morphir::ir::ion::unexpected_value",
            MANIFEST,
            "the manifest starts with morphir::",
        )
    })?;
    if annotation_names(&header)? != ["morphir"] {
        return Err(error(
            "morphir::ir::ion::unexpected_value",
            MANIFEST,
            format!(
                "the manifest starts with morphir::, found {}",
                display_annotations(&annotation_names(&header)?)
            ),
        ));
    }
    let package = header_package(&header)?;
    // An application links its dependencies' definitions; the other kinds publish their faces.
    let dependency_role = match header_kind(&header)? {
        "application" => "def",
        _ => "spec",
    };

    // A dependency's modules, keyed by its canonical package name, in the order the manifest
    // names the packages and then in path order.
    let mut dependencies: IndexMap<String, Vec<Element>> = IndexMap::new();
    let mut metadata = None;
    for element in manifest {
        match annotation_names(&element)?.as_slice() {
            ["morphir", "$meta"] => {
                if metadata.replace(element).is_some() {
                    return Err(error(
                        "morphir::ir::ion::duplicate_name",
                        MANIFEST,
                        "document metadata is listed twice",
                    ));
                }
            }
            ["package", role] if *role == dependency_role => {
                let fields = fields(MANIFEST, &element)?;
                let name = canonical_package(MANIFEST, required_text(MANIFEST, &fields, "name")?)?;
                let modules = dependencies.entry(name.to_canonical_string()).or_default();
                if let Some(list) = fields.get("modules") {
                    modules.extend(list_items(MANIFEST, list, "modules")?);
                }
            }
            ["morphir_footer"] => {
                return Err(error(
                    "morphir::ir::ion::unexpected_value",
                    MANIFEST,
                    "a tree file has no morphir_footer; the end of the file ends it",
                ));
            }
            names => {
                return Err(error(
                    "morphir::ir::ion::unexpected_value",
                    MANIFEST,
                    format!(
                        "the manifest holds the morphir:: header and package::{dependency_role} \
                         elements, found {}",
                        display_annotations(names)
                    ),
                ));
            }
        }
    }

    // Keyed by the root's spelling, so `deps/` sorts before `pkg/`.
    let mut directories: BTreeMap<(&str, String), ModuleFiles> = BTreeMap::new();
    for (logical, text) in files {
        match classify(logical) {
            PathKind::Manifest => {}
            PathKind::Module { root, dir } => {
                directories.entry((root.as_str(), dir)).or_default().module = Some((logical, text));
            }
            PathKind::Type { root, dir, .. } => {
                let stem = stem_of(logical);
                directories
                    .entry((root.as_str(), dir))
                    .or_default()
                    .types
                    .push((logical, stem, text));
            }
            PathKind::Value { root, dir, .. } => {
                let stem = stem_of(logical);
                directories
                    .entry((root.as_str(), dir))
                    .or_default()
                    .values
                    .push((logical, stem, text));
            }
            PathKind::Other => {
                return Err(error(
                    "morphir::ir::ion::unexpected_value",
                    logical,
                    "a tree file is the manifest, a module file, a type file, or a value file",
                ));
            }
        }
    }

    let own = package_dir(Root::Pkg, &package);
    let mut modules = Vec::new();
    for ((root, dir), module_files) in &directories {
        let root = if *root == Root::Pkg.as_str() {
            Root::Pkg
        } else {
            Root::Deps
        };
        let scope = scope_of(root, dir, &package, &own)?;
        let element = assemble(root, dir, &scope, module_files)?;
        match root {
            Root::Pkg => modules.push(element),
            Root::Deps => dependencies
                .entry(scope.package.to_canonical_string())
                .or_default()
                .push(element),
        }
    }

    let mut datagram = Sequence::builder().push(header);
    if let Some(metadata) = metadata {
        datagram = datagram.push(metadata);
    }
    for (name, modules) in dependencies {
        let mut spec = Struct::builder().with_field("name", name.as_str());
        if !modules.is_empty() {
            spec = spec.with_field("modules", list(modules));
        }
        datagram = datagram
            .push(Element::from(spec.build()).with_annotations(["package", dependency_role]));
    }
    for module in modules {
        datagram = datagram.push(module);
    }
    let footer = Element::from(Struct::builder().build()).with_annotations(["morphir_footer"]);
    Ok(datagram.push(footer).build())
}

/// The last segment of a node file's logical path, without `.type` or `.value`.
fn stem_of(logical: &str) -> &str {
    let leaf = logical.rsplit('/').next().unwrap_or(logical);
    leaf.strip_suffix(".type")
        .or_else(|| leaf.strip_suffix(".value"))
        .unwrap_or(leaf)
}

/// The package and module a module directory names.
fn scope_of(
    root: Root,
    dir: &str,
    package: &PackageName,
    own: &str,
) -> Result<Scope, TransportDiagnostic> {
    let at = module_manifest_path(root, dir);
    match root {
        Root::Pkg => {
            if dir == own {
                return Err(error(
                    "morphir::ir::ion::unexpected_member",
                    &at,
                    format!(
                        "a module path has at least one name, so its file is \
                         pkg/{own}/<module>/module, not pkg/{own}/module"
                    ),
                ));
            }
            let module = dir
                .strip_prefix(own)
                .and_then(|rest| rest.strip_prefix('/'))
                .ok_or_else(|| {
                    error(
                        "morphir::ir::ion::unexpected_member",
                        &at,
                        format!(
                            "a module under pkg/ belongs to the package {}",
                            package.to_canonical_string()
                        ),
                    )
                })?;
            Ok(Scope {
                package: package.clone(),
                module: unescape(&at, module)?,
            })
        }
        Root::Deps => {
            let (package, module) =
                dir.split_once(&format!("/{VERSION_SLOT}/"))
                    .ok_or_else(|| {
                        error(
                            "morphir::ir::ion::unexpected_member",
                            &at,
                            "a dependency module sits under deps/<package>/@/",
                        )
                    })?;
            Ok(Scope {
                package: PackageName::new(unescape(&at, package)?),
                module: unescape(&at, module)?,
            })
        }
    }
}

/// The path an escaped directory spells. Every segment must be the escaped stem of its name.
fn unescape(at: &str, dir: &str) -> Result<Path, TransportDiagnostic> {
    let mut segments = Vec::new();
    for segment in dir.split('/') {
        let name = Name::from_file_stem(segment)
            .ok()
            .filter(|name| !name.is_empty() && name.to_file_stem() == segment)
            .ok_or_else(|| {
                error(
                    "morphir::ir::ion::invalid_name",
                    at,
                    format!("'{segment}' is not the escaped stem of a name"),
                )
            })?;
        segments.push(name);
    }
    Ok(Path { segments })
}

/// One module element with its children inline, from the files of its directory.
fn assemble(
    root: Root,
    dir: &str,
    scope: &Scope,
    files: &ModuleFiles,
) -> Result<Element, TransportDiagnostic> {
    let (at, text) = files.module.ok_or_else(|| {
        error(
            "morphir::ir::ion::missing_member",
            &module_manifest_path(root, dir),
            "a module directory holds its module file",
        )
    })?;
    let mut module = Module::default();
    for element in parse(at, text)? {
        match kind_of(&element)? {
            Kind::Module => module.header(at, scope, element)?,
            Kind::Type => module.member(at, scope, Kind::Type, element, None)?,
            Kind::Value => module.member(at, scope, Kind::Value, element, None)?,
        }
    }
    for (kind, node_files) in [(Kind::Type, &files.types), (Kind::Value, &files.values)] {
        for (at, stem, text) in node_files {
            let mut elements = parse(at, text)?.into_iter();
            let element = match (elements.next(), elements.next()) {
                (Some(element), None) if kind_of(&element)? == kind => element,
                (Some(element), None) => {
                    return Err(error(
                        "morphir::ir::ion::unexpected_value",
                        at,
                        format!(
                            "a {} file holds one {}, found {}",
                            kind.as_str(),
                            kind.as_str(),
                            display_annotations(&annotation_names(&element)?)
                        ),
                    ));
                }
                _ => {
                    return Err(error(
                        "morphir::ir::ion::unexpected_value",
                        at,
                        format!("a {} file holds one {}", kind.as_str(), kind.as_str()),
                    ));
                }
            };
            module.member(at, scope, kind, element, Some(stem))?;
        }
    }
    module.finish(at, scope)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Module,
    Type,
    Value,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Module => "module",
            Kind::Type => "type",
            Kind::Value => "value",
        }
    }
}

/// An element's kind: the last annotation names it.
fn kind_of(element: &Element) -> Result<Kind, TransportDiagnostic> {
    let names = annotation_names(element)?;
    match names.as_slice() {
        [.., "module"] | ["module", "spec"] => Ok(Kind::Module),
        [.., "type"] => Ok(Kind::Type),
        [.., "value"] => Ok(Kind::Value),
        _ => Err(IonCodec::error(
            "morphir::ir::ion::unexpected_value",
            Stage::Detection,
            format!(
                "a module directory holds modules, types, and values, found {}",
                display_annotations(&names)
            ),
        )),
    }
}

/// A module header's annotations, and its members other than the scope and the children.
type Header = (Vec<String>, Vec<(String, Element)>);

/// A module as its fragments merge.
#[derive(Default)]
struct Module {
    header: Option<Header>,
    types: Vec<Element>,
    values: Vec<Element>,
    type_names: HashSet<Name>,
    value_names: HashSet<Name>,
}

impl Module {
    /// Merges a module element: the header's members, then its inline children in order.
    ///
    /// A definition module has one owner, so a second `def` header is refused. A `module::spec`
    /// may repeat, and its members merge; a member stated twice is refused.
    fn header(
        &mut self,
        at: &str,
        scope: &Scope,
        element: Element,
    ) -> Result<(), TransportDiagnostic> {
        let annotations: Vec<String> = annotation_names(&element)?
            .into_iter()
            .map(str::to_owned)
            .collect();
        let fields = fields(at, &element)?;
        check_scope(at, scope, &fields, None)?;
        let mut members = Vec::new();
        for (name, value) in fields.iter() {
            match name {
                "package" | "module" | "name" | "types" | "values" => {}
                other => members.push((other.to_owned(), value.clone())),
            }
        }
        match &mut self.header {
            None => self.header = Some((annotations, members)),
            Some((existing, merged)) => {
                if *existing != annotations || existing.as_slice() != ["module", "spec"] {
                    return Err(error(
                        "morphir::ir::ion::duplicate_name",
                        at,
                        format!(
                            "module '{}' is already defined; one {} owns a module",
                            scope.module.to_canonical_string(),
                            existing.join("::")
                        ),
                    ));
                }
                for (name, value) in members {
                    if merged.iter().any(|(existing, _)| *existing == name) {
                        return Err(error(
                            "morphir::ir::ion::duplicate_field",
                            at,
                            format!("module::spec states {name} twice"),
                        ));
                    }
                    merged.push((name, value));
                }
            }
        }
        for (list_name, kind) in [("types", Kind::Type), ("values", Kind::Value)] {
            if let Some(list) = fields.get(list_name) {
                for child in list_items(at, list, list_name)? {
                    self.member(at, scope, kind, child, None)?;
                }
            }
        }
        Ok(())
    }

    /// Merges one type or value. `stem` is the file stem a node file supplies the name with.
    fn member(
        &mut self,
        at: &str,
        scope: &Scope,
        kind: Kind,
        element: Element,
        stem: Option<&str>,
    ) -> Result<(), TransportDiagnostic> {
        if kind_of(&element)? != kind {
            return Err(error(
                "morphir::ir::ion::unexpected_value",
                at,
                format!(
                    "expected a {}, found {}",
                    kind.as_str(),
                    display_annotations(&annotation_names(&element)?)
                ),
            ));
        }
        let fields = fields(at, &element)?;
        check_scope(at, scope, &fields, Some(&scope.module))?;
        let name = member_name(at, &fields, stem)?;
        let (names, members) = match kind {
            Kind::Type => (&mut self.type_names, &mut self.types),
            _ => (&mut self.value_names, &mut self.values),
        };
        if !names.insert(name.clone()) {
            return Err(error(
                "morphir::ir::ion::duplicate_name",
                at,
                format!(
                    "{} '{}' is already defined in module '{}'",
                    kind.as_str(),
                    name.to_canonical_string(),
                    scope.module.to_canonical_string()
                ),
            ));
        }
        let mut normalized = Struct::builder().with_field("name", name.to_canonical_string());
        for (field, value) in fields.iter() {
            if !SCOPE_MEMBERS.contains(&field) {
                normalized = normalized.with_field(field, value.clone());
            }
        }
        members.push(Element::from(normalized.build()).with_annotations(annotations_of(&element)?));
        Ok(())
    }

    fn finish(self, at: &str, scope: &Scope) -> Result<Element, TransportDiagnostic> {
        let (annotations, members) = self.header.ok_or_else(|| {
            error(
                "morphir::ir::ion::missing_member",
                at,
                "a module file holds its module element",
            )
        })?;
        let mut module = Struct::builder().with_field("name", scope.module.to_canonical_string());
        for (name, value) in members {
            module = module.with_field(name, value);
        }
        if !self.types.is_empty() {
            module = module.with_field("types", list(self.types));
        }
        if !self.values.is_empty() {
            module = module.with_field("values", list(self.values));
        }
        Ok(Element::from(module.build()).with_annotations(annotations))
    }
}

/// Checks the `package` and `module` a file states against the path.
///
/// `module` is the owner a child names; a module header names itself with `name`.
fn check_scope(
    at: &str,
    scope: &Scope,
    fields: &Fields,
    owner: Option<&Path>,
) -> Result<(), TransportDiagnostic> {
    if let Some(text) = optional_text(at, fields, "package")? {
        let stated = canonical_package(at, text)?;
        if stated != scope.package {
            return Err(mismatch(
                at,
                "package",
                text,
                &scope.package.to_canonical_string(),
            ));
        }
    }
    let (member, expected) = match owner {
        Some(owner) => ("module", owner),
        None => ("name", &scope.module),
    };
    if let Some(text) = optional_text(at, fields, member)? {
        let stated = Path::from_canonical_string(text)
            .map_err(|message| error("morphir::ir::ion::invalid_name", at, message))?;
        if stated != *expected {
            return Err(mismatch(at, member, text, &expected.to_canonical_string()));
        }
    }
    Ok(())
}

/// The name of a type or value: the one the file states, else the one its stem spells.
fn member_name(at: &str, fields: &Fields, stem: Option<&str>) -> Result<Name, TransportDiagnostic> {
    let stated = optional_text(at, fields, "name")?
        .map(|text| {
            Name::from_canonical_string(text)
                .map_err(|message| error("morphir::ir::ion::invalid_name", at, message))
        })
        .transpose()?;
    match (stated, stem) {
        (Some(name), Some(stem)) => {
            if !naming::is_stem_of(stem, &name.to_file_stem()) {
                return Err(mismatch(
                    at,
                    "name",
                    &name.to_canonical_string(),
                    &format!("the name stem {stem} spells"),
                ));
            }
            Ok(name)
        }
        (Some(name), None) => Ok(name),
        (None, Some(stem)) => Name::from_file_stem(stem)
            .ok()
            .filter(|name| !name.is_empty() && name.to_file_stem() == stem)
            .ok_or_else(|| {
                error(
                    "morphir::ir::ion::missing_member",
                    at,
                    format!("the stem {stem} does not spell a name, so the file states its name"),
                )
            }),
        (None, None) => Err(error(
            "morphir::ir::ion::missing_member",
            at,
            "a type or value in a module file states its name",
        )),
    }
}

fn mismatch(at: &str, member: &str, stated: &str, expected: &str) -> TransportDiagnostic {
    error(
        "morphir::ir::ion::name_mismatch",
        at,
        format!("{member} '{stated}' does not match the path, which gives {expected}"),
    )
}

fn header_kind(header: &Element) -> Result<&str, TransportDiagnostic> {
    let fields = fields(MANIFEST, header)?;
    let kind = fields.get("kind").ok_or_else(|| {
        error(
            "morphir::ir::ion::missing_member",
            MANIFEST,
            "kind is required",
        )
    })?;
    kind.as_symbol()
        .and_then(|symbol| symbol.text())
        .or_else(|| kind.as_string())
        .ok_or_else(|| {
            error(
                "morphir::ir::ion::invalid_member",
                MANIFEST,
                "kind is a symbol",
            )
        })
}

fn header_package(header: &Element) -> Result<PackageName, TransportDiagnostic> {
    let fields = fields(MANIFEST, header)?;
    canonical_package(MANIFEST, required_text(MANIFEST, &fields, "packageName")?)
}

fn canonical_package(at: &str, text: &str) -> Result<PackageName, TransportDiagnostic> {
    PackageName::from_canonical_string(text)
        .map_err(|message| error("morphir::ir::ion::invalid_name", at, message))
}

// =============================================================================
// Writing
// =============================================================================

/// A module on its way to its directory.
struct Outgoing {
    root: Root,
    package: PackageName,
    header: Element,
    types: Vec<Element>,
    values: Vec<Element>,
}

/// The files a datagram lays out as, keyed by logical path, in write order.
pub(super) fn write(
    datagram: Sequence,
    path_budget: u32,
) -> Result<Vec<(String, String)>, TransportDiagnostic> {
    let mut elements = datagram.into_iter();
    let header = elements.next().ok_or_else(|| {
        IonCodec::error(
            "morphir::ir::ion::unexpected_value",
            Stage::Encoding,
            "a datagram starts with morphir::",
        )
    })?;
    let package = header_package(&header)?;
    let mut manifest = vec![with_path_budget(&header, path_budget)?];
    let mut modules: IndexMap<(Root, String, String), Outgoing> = IndexMap::new();

    for element in elements {
        let names = annotation_names(&element)?;
        let fields = fields(MANIFEST, &element)?;
        match names.as_slice() {
            ["morphir_footer"] => {}
            ["morphir", "$meta"] => manifest.push(element),
            ["package", role @ ("spec" | "def")] => {
                let name = required_text(MANIFEST, &fields, "name")?;
                let dependency = canonical_package(MANIFEST, name)?;
                manifest.push(
                    Element::from(Struct::builder().with_field("name", name).build())
                        .with_annotations(["package", *role]),
                );
                if let Some(list) = fields.get("modules") {
                    for module in list_items(MANIFEST, list, "modules")? {
                        outgoing(&mut modules, Root::Deps, &dependency, module)?;
                    }
                }
            }
            [.., "module"] | ["module", "spec"] => {
                outgoing(&mut modules, Root::Pkg, &package, element)?
            }
            [.., kind @ ("type" | "value")] => {
                let owner = required_text(MANIFEST, &fields, "module")?;
                let key = (Root::Pkg, package.to_canonical_string(), owner.to_owned());
                let module = modules.get_mut(&key).ok_or_else(|| {
                    IonCodec::error(
                        "morphir::ir::ion::missing_member",
                        Stage::Encoding,
                        format!("module '{owner}' is not defined"),
                    )
                })?;
                if *kind == "type" {
                    module.types.push(element.clone());
                } else {
                    module.values.push(element.clone());
                }
            }
            other => {
                return Err(IonCodec::error(
                    "morphir::ir::ion::unexpected_value",
                    Stage::Encoding,
                    format!("unexpected datagram value {}", display_annotations(other)),
                ));
            }
        }
    }

    let mut files = vec![(MANIFEST.to_owned(), ion_text(sequence(manifest))?)];
    for ((_, _, module_name), module) in modules {
        files.extend(module_files(&module_name, module, path_budget)?);
    }
    Ok(files)
}

/// Records a module element; its inline children become its members.
fn outgoing(
    modules: &mut IndexMap<(Root, String, String), Outgoing>,
    root: Root,
    package: &PackageName,
    element: Element,
) -> Result<(), TransportDiagnostic> {
    let fields = fields(MANIFEST, &element)?;
    let name = required_text(MANIFEST, &fields, "name")?.to_owned();
    let mut types = Vec::new();
    let mut values = Vec::new();
    if let Some(list) = fields.get("types") {
        types = list_items(MANIFEST, list, "types")?;
    }
    if let Some(list) = fields.get("values") {
        values = list_items(MANIFEST, list, "values")?;
    }
    let header = without(&element, &["package", "name", "types", "values"])?;
    modules.insert(
        (root, package.to_canonical_string(), name),
        Outgoing {
            root,
            package: package.clone(),
            header,
            types,
            values,
        },
    );
    Ok(())
}

fn module_files(
    module_name: &str,
    module: Outgoing,
    path_budget: u32,
) -> Result<Vec<(String, String)>, TransportDiagnostic> {
    let path = Path::from_canonical_string(module_name).map_err(|message| {
        IonCodec::error("morphir::ir::ion::invalid_name", Stage::Encoding, message)
    })?;
    let dir = module_dir(module.root, &module.package, &path);
    let manifest_path = module_manifest_path(module.root, &dir);
    let physical = format!("{manifest_path}{EXTENSION}");
    if physical.chars().count() > path_budget as usize {
        return Err(IonCodec::error(
            "morphir::ir::ion::path_budget",
            Stage::Encoding,
            format!("path budget {path_budget} cannot fit {physical}"),
        ));
    }
    let mut files = vec![(manifest_path, ion_text(sequence(vec![module.header]))?)];
    let prefix = module_dir_prefix(module.root, &dir);
    for (kind, members) in [
        (NodeFileKind::Type, module.types),
        (NodeFileKind::Value, module.values),
    ] {
        let suffix = format!(".{}{EXTENSION}", kind.as_str());
        let mut seen = HashSet::new();
        for member in members {
            let fields = fields(MANIFEST, &member)?;
            let text = required_text(MANIFEST, &fields, "name")?;
            let name = Name::from_canonical_string(text).map_err(|message| {
                IonCodec::error("morphir::ir::ion::invalid_name", Stage::Encoding, message)
            })?;
            let chosen = stem_for(&name, &prefix, &suffix, path_budget).map_err(|diagnostic| {
                IonCodec::error(
                    "morphir::ir::ion::path_budget",
                    Stage::Encoding,
                    diagnostic.message,
                )
            })?;
            if !seen.insert(chosen.stem.clone()) {
                return Err(IonCodec::error(
                    "morphir::ir::ion::path_budget",
                    Stage::Encoding,
                    format!(
                        "two {} names share the file stem \"{}\"",
                        kind.as_str(),
                        chosen.stem
                    ),
                ));
            }
            let removed: &[&str] = if chosen.truncated {
                &["package", "module"]
            } else {
                &SCOPE_MEMBERS
            };
            let element = without(&member, removed)?;
            files.push((
                node_file_path(module.root, &dir, &chosen.stem, kind),
                ion_text(sequence(vec![element]))?,
            ));
        }
    }
    Ok(files)
}

fn with_path_budget(header: &Element, path_budget: u32) -> Result<Element, TransportDiagnostic> {
    let mut builder = Struct::builder();
    for (name, value) in fields(MANIFEST, header)?.iter() {
        if name != "pathBudget" {
            builder = builder.with_field(name, value.clone());
        }
    }
    let header_annotations = annotations_of(header)?;
    Ok(Element::from(
        builder
            .with_field("pathBudget", i64::from(path_budget))
            .build(),
    )
    .with_annotations(header_annotations))
}

/// `element` without the named members.
fn without(element: &Element, removed: &[&str]) -> Result<Element, TransportDiagnostic> {
    let mut builder = Struct::builder();
    for (name, value) in fields(MANIFEST, element)?.iter() {
        if !removed.contains(&name) {
            builder = builder.with_field(name, value.clone());
        }
    }
    Ok(Element::from(builder.build()).with_annotations(annotations_of(element)?))
}

// =============================================================================
// Shared helpers
// =============================================================================

fn parse(at: &str, text: &str) -> Result<Vec<Element>, TransportDiagnostic> {
    let values = Element::read_all(text.as_bytes()).map_err(|syntax| {
        error("morphir::ir::ion::invalid_syntax", at, syntax.to_string())
            .with_guidance("correct the Ion syntax of the tree file")
    })?;
    Ok(values.into_iter().collect())
}

/// A struct's members in document order. A repeated member is refused.
struct Fields<'a>(Vec<(&'a str, &'a Element)>);

impl<'a> Fields<'a> {
    fn get(&self, name: &str) -> Option<&'a Element> {
        self.0
            .iter()
            .find(|(member, _)| *member == name)
            .map(|(_, value)| *value)
    }

    fn iter(&self) -> impl Iterator<Item = (&'a str, &'a Element)> + '_ {
        self.0.iter().copied()
    }
}

fn fields<'a>(at: &str, element: &'a Element) -> Result<Fields<'a>, TransportDiagnostic> {
    let Some(value) = element.as_struct() else {
        return Err(error(
            "morphir::ir::ion::unexpected_value",
            at,
            format!(
                "{}:: is a struct",
                display_annotations(&annotation_names(element)?)
            ),
        ));
    };
    let mut fields = Vec::new();
    for (symbol, field) in value.fields() {
        let name = symbol_text(symbol)?;
        if fields.iter().any(|(existing, _)| *existing == name) {
            return Err(error(
                "morphir::ir::ion::duplicate_field",
                at,
                format!("the element repeats field '{name}'"),
            ));
        }
        fields.push((name, field));
    }
    Ok(Fields(fields))
}

fn optional_text<'a>(
    at: &str,
    fields: &Fields<'a>,
    name: &str,
) -> Result<Option<&'a str>, TransportDiagnostic> {
    match fields.get(name) {
        None => Ok(None),
        Some(element) => element.as_string().map(Some).ok_or_else(|| {
            error(
                "morphir::ir::ion::invalid_member",
                at,
                format!("{name} is a string"),
            )
        }),
    }
}

fn required_text<'a>(
    at: &str,
    fields: &Fields<'a>,
    name: &str,
) -> Result<&'a str, TransportDiagnostic> {
    optional_text(at, fields, name)?.ok_or_else(|| {
        error(
            "morphir::ir::ion::missing_member",
            at,
            format!("{name} is required"),
        )
    })
}

fn list_items(
    at: &str,
    element: &Element,
    name: &str,
) -> Result<Vec<Element>, TransportDiagnostic> {
    let items = element.as_list().ok_or_else(|| {
        error(
            "morphir::ir::ion::invalid_member",
            at,
            format!("{name} is a list"),
        )
    })?;
    Ok(items.iter().cloned().collect())
}

fn annotations_of(element: &Element) -> Result<Vec<String>, TransportDiagnostic> {
    Ok(annotation_names(element)?
        .into_iter()
        .map(str::to_owned)
        .collect())
}

fn list(elements: Vec<Element>) -> ion_rs::List {
    elements
        .into_iter()
        .fold(Sequence::builder(), |builder, element| {
            builder.push(element)
        })
        .build_list()
}

fn sequence(elements: Vec<Element>) -> Sequence {
    elements
        .into_iter()
        .fold(Sequence::builder(), |builder, element| {
            builder.push(element)
        })
        .build()
}

/// A diagnostic about the tree file at logical path `at`.
fn error(code: &'static str, at: &str, message: impl Into<String>) -> TransportDiagnostic {
    IonCodec::error(
        code,
        Stage::Normalization,
        format!("{at}: {}", message.into()),
    )
}
