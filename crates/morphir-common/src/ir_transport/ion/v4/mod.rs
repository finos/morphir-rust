//! Ion spelling of a v4 distribution.
//!
//! The writer emits one datagram: the header, one `package::spec` (or, for an application,
//! `package::def`) for each dependency, then one module per module of the distribution's own
//! package with its members inline, then the footer. A `library` and an `application` write their
//! own modules as `public::def::module`; a `specs` distribution writes them as `module::spec`. An
//! application carries its `entryPoints` on the header.

mod annotations;
mod attributes;
mod json;
mod types;
mod values;

use std::collections::BTreeMap;

use indexmap::IndexMap;
use ion_rs::Element;
use morphir_core::ir::v4::{self, Access};
use morphir_core::naming::{FQName, Name, PackageName};

use super::{
    ION_CONTRACT, IonCodec, annotation_names, expect_marker, required_string, struct_fields,
};
use crate::ir_transport::{Stage, TransportDiagnostic};

use types::{read_type_def, read_type_spec, write_type_def, write_type_spec};
use values::{read_value_def, read_value_spec, write_value_def, write_value_spec};

pub(super) fn read_value_fragment(element: &Element) -> Result<v4::Value, TransportDiagnostic> {
    values::read_value(element)
}

pub(super) fn write_value_fragment(value: &v4::Value) -> Result<Element, TransportDiagnostic> {
    values::write_value(value)
}

const RELEASE: &str = "4.0.0";
const LINKED_RELEASE: &str = "4.1.0";

// =============================================================================
// Distribution
// =============================================================================

pub(super) fn decode(
    values: &ion_rs::Sequence,
    linked_metadata: bool,
) -> Result<v4::IRFile, TransportDiagnostic> {
    let header = values.get(0).ok_or_else(|| {
        IonCodec::error(
            "morphir::ir::ion::unexpected_value",
            Stage::Detection,
            "an Ion IR document starts with morphir::",
        )
    })?;
    expect_marker(header, "morphir")?;
    let fields = struct_fields(header, "morphir")?;
    refuse_top_level_metadata(&fields)?;
    if values.len() > 1 && fields.contains_key("$meta") {
        return Err(member(
            "a datagram places metadata in a morphir::$meta:: element",
        ));
    }
    let ion_version = super::optional_string(&fields, "ionVersion")?;
    super::accept_ion_version(ion_version, linked_metadata)?;
    let version = required_string(&fields, "formatVersion")?;
    if version != RELEASE && version != LINKED_RELEASE {
        return Err(IonCodec::error(
            "morphir::ir::ion::version_mismatch",
            Stage::Detection,
            format!("a v4 Ion document uses formatVersion {RELEASE}, found {version}"),
        ));
    }
    if version == LINKED_RELEASE
        && (!linked_metadata || ion_version != Some(super::LINKED_ION_CONTRACT))
    {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_version",
            Stage::Detection,
            "formatVersion 4.1.0 requires the linked-metadata option and ionVersion 0.1.0-draft.2",
        ));
    }
    if version == RELEASE && ion_version == Some(super::LINKED_ION_CONTRACT) {
        return Err(IonCodec::error(
            "morphir::ir::ion::version_mismatch",
            Stage::Detection,
            "ionVersion 0.1.0-draft.2 requires formatVersion 4.1.0",
        ));
    }
    let kind = super::required_text(&fields, "kind")?;
    let package_name = package_name(required_string(&fields, "packageName")?)?;
    if kind != "application" && fields.contains_key("entryPoints") {
        return Err(member("only an application has entryPoints"));
    }
    let (body, metadata) = read_body(values)?;
    let distribution = match kind {
        "library" => {
            body.refuse_definition_dependencies(kind)?;
            body.refuse_specification_modules(kind)?;
            v4::Distribution::Library(v4::LibraryContent {
                package_name,
                dependencies: body.specifications,
                def: v4::PackageDefinition {
                    modules: body.definitions,
                },
            })
        }
        "specs" => {
            body.refuse_definition_dependencies(kind)?;
            if !body.definitions.is_empty() {
                return Err(member(
                    "a specs distribution writes its modules as module::spec",
                ));
            }
            v4::Distribution::Specs(v4::SpecsContent {
                package_name,
                dependencies: body.specifications,
                spec: v4::PackageSpecification {
                    modules: body.own_specifications,
                },
            })
        }
        "application" => {
            if !body.specifications.is_empty() {
                return Err(member(
                    "an application writes its dependencies as package::def",
                ));
            }
            body.refuse_specification_modules(kind)?;
            v4::Distribution::Application(v4::ApplicationContent {
                package_name,
                dependencies: body.definition_dependencies,
                def: v4::PackageDefinition {
                    modules: body.definitions,
                },
                entry_points: read_entry_points(fields.get("entryPoints").copied())?,
            })
        }
        other => {
            return Err(IonCodec::error(
                "morphir::ir::ion::unsupported_kind",
                Stage::Normalization,
                format!("a v4 distribution is library, specs, or application, found {other}"),
            ));
        }
    };
    let file = v4::IRFile {
        format_version: v4::FormatVersion::String(version.to_owned()),
        distribution,
        metadata,
    };
    if version == RELEASE && file.has_linked_metadata() {
        return Err(member("linked metadata requires formatVersion 4.1.0"));
    }
    if version == LINKED_RELEASE {
        let authored = serde_json::to_value(&file).map_err(|error| member(error.to_string()))?;
        serde_json::from_value(authored).map_err(|error| member(error.to_string()))
    } else {
        Ok(file)
    }
}

pub(super) fn datagram(
    file: v4::IRFile,
    linked_metadata: bool,
) -> Result<ion_rs::Sequence, TransportDiagnostic> {
    let proposed = file.format_version == v4::FormatVersion::String(LINKED_RELEASE.to_owned());
    if proposed && !linked_metadata {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_version",
            Stage::Encoding,
            "formatVersion 4.1.0 requires the linked-metadata option",
        ));
    }
    if !matches!(&file.format_version, v4::FormatVersion::Integer(4))
        && !matches!(&file.format_version, v4::FormatVersion::String(release) if release == RELEASE || release == LINKED_RELEASE)
    {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_version",
            Stage::Encoding,
            "the Ion v4 profile supports formatVersion 4.0.0 and opted-in 4.1.0",
        ));
    }
    if !proposed && file.has_linked_metadata() {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_metadata",
            Stage::Encoding,
            "the Ion v4 profile does not carry linked metadata",
        ));
    }
    let mut sequence = ion_rs::Sequence::builder();
    match &file.distribution {
        v4::Distribution::Library(content) => {
            sequence = sequence.push(header("library", &content.package_name, None, proposed)?);
            for (name, spec) in &content.dependencies {
                sequence = sequence.push(package_spec(name, spec)?);
            }
            for (name, module) in &content.def.modules {
                sequence = sequence.push(def_module(name, module)?);
            }
        }
        v4::Distribution::Specs(content) => {
            sequence = sequence.push(header("specs", &content.package_name, None, proposed)?);
            for (name, spec) in &content.dependencies {
                sequence = sequence.push(package_spec(name, spec)?);
            }
            for (name, module) in &content.spec.modules {
                sequence = sequence.push(module_spec(name, module)?);
            }
        }
        v4::Distribution::Application(content) => {
            sequence = sequence.push(header(
                "application",
                &content.package_name,
                Some(&content.entry_points),
                proposed,
            )?);
            for (name, definition) in &content.dependencies {
                sequence = sequence.push(package_def(name, definition)?);
            }
            for (name, module) in &content.def.modules {
                sequence = sequence.push(def_module(name, module)?);
            }
        }
    }
    let sequence = if let Some(metadata) = &file.metadata {
        // The document container follows the header, before any distribution members.
        let mut ordered = ion_rs::Sequence::builder();
        let written = sequence.build();
        let mut elements = written.iter();
        ordered = ordered.push(elements.next().expect("header was written").clone());
        ordered = ordered.push(write_document_meta(metadata)?);
        for element in elements {
            ordered = ordered.push(element.clone());
        }
        ordered.push(footer()).build()
    } else {
        sequence.push(footer()).build()
    };
    if proposed {
        let decoded = decode(&sequence, true).map_err(|error| {
            IonCodec::error(
                "morphir::ir::ion::unsupported_metadata",
                Stage::Encoding,
                format!(
                    "the Ion draft.2 writer cannot roundtrip this metadata: {}",
                    error.message()
                ),
            )
        })?;
        if decoded != file {
            return Err(IonCodec::error(
                "morphir::ir::ion::unsupported_metadata",
                Stage::Encoding,
                "the Ion draft.2 writer would change this IR document on roundtrip",
            ));
        }
    }
    Ok(sequence)
}

/// The modules and dependencies a datagram's body holds, before the header's kind is applied.
#[derive(Default)]
struct Body {
    specifications: IndexMap<String, v4::PackageSpecification>,
    definition_dependencies: IndexMap<String, v4::PackageDefinition>,
    definitions: IndexMap<String, v4::AccessControlled<v4::ModuleDefinition>>,
    own_specifications: IndexMap<String, v4::ModuleSpecification>,
}

impl Body {
    fn refuse_definition_dependencies(&self, kind: &str) -> Result<(), TransportDiagnostic> {
        if self.definition_dependencies.is_empty() {
            Ok(())
        } else {
            Err(member(format!(
                "a {kind} writes its dependencies as package::spec"
            )))
        }
    }

    fn refuse_specification_modules(&self, kind: &str) -> Result<(), TransportDiagnostic> {
        if self.own_specifications.is_empty() {
            Ok(())
        } else {
            Err(member(format!(
                "a {kind} writes its modules as public::def::module"
            )))
        }
    }
}

fn read_body(
    values: &ion_rs::Sequence,
) -> Result<(Body, Option<Box<v4::DocumentMeta>>), TransportDiagnostic> {
    if values.len() == 1 {
        return read_record_body(values.get(0).expect("length checked"));
    }
    let last = values.len() - 1;
    expect_marker(values.get(last).expect("length checked"), "morphir_footer")?;
    let mut body = Body::default();
    let mut metadata = None;
    for index in 1..last {
        let element = values.get(index).expect("index in range");
        match annotation_names(element)?.as_slice() {
            ["morphir", "$meta"] => {
                if metadata.is_some() {
                    return Err(member("document metadata is listed twice"));
                }
                metadata = Some(Box::new(read_document_meta(element)?));
            }
            ["package", "spec"] => {
                let (name, spec) = read_package_spec(element)?;
                merge_modules(&mut body.specifications, name, spec, |spec| {
                    &mut spec.modules
                })?;
            }
            ["package", "def"] => {
                let (name, definition) = read_package_def(element)?;
                merge_modules(
                    &mut body.definition_dependencies,
                    name,
                    definition,
                    |definition| &mut definition.modules,
                )?;
            }
            ["public", "def", "module"] | ["private", "def", "module"] => {
                let (name, module) = read_def_module(element)?;
                insert_new(&mut body.definitions, "module", name, module)?;
            }
            ["module", "spec"] => {
                let (name, module) = read_module_spec(element)?;
                insert_new(&mut body.own_specifications, "module", name, module)?;
            }
            names => {
                return Err(IonCodec::error(
                    "morphir::ir::ion::unexpected_value",
                    Stage::Detection,
                    format!("unexpected v4 value {}", names.join("::")),
                ));
            }
        }
    }
    Ok((body, metadata))
}

fn read_record_body(
    header: &Element,
) -> Result<(Body, Option<Box<v4::DocumentMeta>>), TransportDiagnostic> {
    let fields = struct_fields(header, "morphir")?;
    let mut values = ion_rs::Sequence::builder().push(header.clone());
    if let Some(metadata) = fields.get("$meta") {
        if !annotation_names(metadata)?.is_empty() {
            return Err(member(
                "the single-record $meta member is an unannotated struct",
            ));
        }
        values = values.push((*metadata).clone().with_annotations(["morphir", "$meta"]));
    }
    for name in ["dependencies", "modules"] {
        if let Some(elements) = fields.get(name) {
            let elements = elements
                .as_list()
                .ok_or_else(|| member(format!("{name} is a list")))?;
            for element in elements.iter() {
                values = values.push(element.clone());
            }
        }
    }
    read_body(&values.push(footer()).build())
}

fn read_document_meta(element: &Element) -> Result<v4::DocumentMeta, TransportDiagnostic> {
    let value = serde_json::Value::Object(json::object_from_ion(
        &element.clone().with_annotations(std::iter::empty::<&str>()),
    )?);
    v4::DocumentMeta::parse(&value).map_err(member)
}

fn write_document_meta(metadata: &v4::DocumentMeta) -> Result<Element, TransportDiagnostic> {
    let value = serde_json::to_value(metadata).map_err(|error| member(error.to_string()))?;
    Ok(json::to_ion(&value)?.with_annotations(["morphir", "$meta"]))
}

fn header(
    kind: &str,
    package: &PackageName,
    entry_points: Option<&v4::EntryPoints>,
    proposed: bool,
) -> Result<Element, TransportDiagnostic> {
    let mut builder = ion_rs::Struct::builder()
        .with_field(
            "ionVersion",
            if proposed {
                super::LINKED_ION_CONTRACT
            } else {
                ION_CONTRACT
            },
        )
        .with_field(
            "formatVersion",
            if proposed { LINKED_RELEASE } else { RELEASE },
        )
        .with_field("kind", Element::symbol(kind))
        .with_field("packageName", package.to_canonical_string());
    if let Some(entry_points) = entry_points.filter(|entry_points| !entry_points.is_empty()) {
        builder = builder.with_field("entryPoints", write_entry_points(entry_points)?);
    }
    Ok(Element::from(builder.build()).with_annotations(["morphir"]))
}

fn footer() -> Element {
    Element::from(ion_rs::Struct::builder().build()).with_annotations(["morphir_footer"])
}

/// Entry points as a struct keyed by name: `{ start: { target, kind: main, doc } }`.
fn read_entry_points(element: Option<&Element>) -> Result<v4::EntryPoints, TransportDiagnostic> {
    let mut entry_points = v4::EntryPoints::new();
    let Some(element) = element else {
        return Ok(entry_points);
    };
    let fields = element
        .as_struct()
        .ok_or_else(|| member("entryPoints is a struct"))?;
    for (symbol, entry) in fields.fields() {
        let name = super::symbol_text(symbol)?.to_owned();
        let members = struct_fields(entry, "entry point")?;
        let kind = super::required_text(&members, "kind")?;
        let kind: v4::EntryPointKind = serde_json::from_value(serde_json::json!(kind))
            .map_err(|_| member(format!("unknown entry point kind '{kind}'")))?;
        let target = required_string(&members, "target")?;
        fq_name(target)?;
        let entry_point = v4::EntryPoint {
            target: target.to_owned(),
            kind,
            doc: super::optional_string(&members, "doc")?.map(str::to_owned),
        };
        insert_new(&mut entry_points, "entry point", name, entry_point)?;
    }
    Ok(entry_points)
}

fn write_entry_points(entry_points: &v4::EntryPoints) -> Result<Element, TransportDiagnostic> {
    let mut builder = ion_rs::Struct::builder();
    for (name, entry_point) in entry_points {
        let kind = serde_json::to_value(entry_point.kind)
            .ok()
            .and_then(|kind| kind.as_str().map(str::to_owned))
            .ok_or_else(|| member("an entry point kind has a name"))?;
        let mut entry = ion_rs::Struct::builder()
            .with_field("target", entry_point.target.as_str())
            .with_field("kind", Element::symbol(kind));
        if let Some(doc) = &entry_point.doc {
            entry = entry.with_field("doc", doc.as_str());
        }
        builder = builder.with_field(name.as_str(), Element::from(entry.build()));
    }
    Ok(Element::from(builder.build()))
}

// =============================================================================
// Packages and modules
// =============================================================================

fn read_package_spec(
    element: &Element,
) -> Result<(String, v4::PackageSpecification), TransportDiagnostic> {
    let fields = struct_fields(element, "package::spec")?;
    refuse_top_level_metadata(&fields)?;
    refuse_annotations(&fields)?;
    let name = canonical_package(required_string(&fields, "name")?)?;
    let mut modules = IndexMap::new();
    for module in list_items(&fields, "modules")? {
        let (module_name, spec) = read_module_spec(module)?;
        insert_new(&mut modules, "module", module_name, spec)?;
    }
    Ok((name, v4::PackageSpecification { modules }))
}

fn package_spec(
    name: &str,
    spec: &v4::PackageSpecification,
) -> Result<Element, TransportDiagnostic> {
    let modules = spec
        .modules
        .iter()
        .map(|(module_name, module)| module_spec(module_name, module))
        .collect::<Result<Vec<_>, _>>()?;
    let mut builder = ion_rs::Struct::builder().with_field("name", name);
    if !modules.is_empty() {
        builder = builder.with_field("modules", list(modules));
    }
    Ok(Element::from(builder.build()).with_annotations(["package", "spec"]))
}

fn read_package_def(
    element: &Element,
) -> Result<(String, v4::PackageDefinition), TransportDiagnostic> {
    let fields = struct_fields(element, "package::def")?;
    refuse_top_level_metadata(&fields)?;
    let name = canonical_package(required_string(&fields, "name")?)?;
    let mut modules = IndexMap::new();
    for module in list_items(&fields, "modules")? {
        let (module_name, definition) = read_def_module(module)?;
        insert_new(&mut modules, "module", module_name, definition)?;
    }
    Ok((name, v4::PackageDefinition { modules }))
}

fn package_def(
    name: &str,
    definition: &v4::PackageDefinition,
) -> Result<Element, TransportDiagnostic> {
    let modules = definition
        .modules
        .iter()
        .map(|(module_name, module)| def_module(module_name, module))
        .collect::<Result<Vec<_>, _>>()?;
    let mut builder = ion_rs::Struct::builder().with_field("name", name);
    if !modules.is_empty() {
        builder = builder.with_field("modules", list(modules));
    }
    Ok(Element::from(builder.build()).with_annotations(["package", "def"]))
}

fn read_module_spec(
    element: &Element,
) -> Result<(String, v4::ModuleSpecification), TransportDiagnostic> {
    if annotation_names(element)?.as_slice() != ["module", "spec"] {
        return Err(member("expected module::spec"));
    }
    let fields = struct_fields(element, "module::spec")?;
    refuse_top_level_metadata(&fields)?;
    let annotations = annotations::read_annotations(&fields)?;
    let name = canonical_module(required_string(&fields, "name")?)?;
    let mut types = IndexMap::new();
    for item in list_items(&fields, "types")? {
        let (type_name, spec) = read_type_spec(item)?;
        insert_new(&mut types, "type", type_name, spec)?;
    }
    let mut values = IndexMap::new();
    for item in list_items(&fields, "values")? {
        let (value_name, spec) = read_value_spec(item)?;
        insert_new(&mut values, "value", value_name, spec)?;
    }
    Ok((
        name,
        v4::ModuleSpecification {
            annotations,
            types,
            values,
            doc: optional_doc(&fields)?,
        },
    ))
}

fn module_spec(name: &str, spec: &v4::ModuleSpecification) -> Result<Element, TransportDiagnostic> {
    let mut builder = annotations::with_annotations(
        ion_rs::Struct::builder().with_field("name", name),
        &spec.annotations,
    )?;
    if let Some(doc) = &spec.doc {
        builder = builder.with_field("doc", doc.text());
    }
    if !spec.types.is_empty() {
        let types = spec
            .types
            .iter()
            .map(|(type_name, documented)| write_type_spec(type_name, documented))
            .collect::<Result<Vec<_>, _>>()?;
        builder = builder.with_field("types", list(types));
    }
    if !spec.values.is_empty() {
        let values = spec
            .values
            .iter()
            .map(|(value_name, documented)| write_value_spec(value_name, documented))
            .collect::<Result<Vec<_>, _>>()?;
        builder = builder.with_field("values", list(values));
    }
    Ok(Element::from(builder.build()).with_annotations(["module", "spec"]))
}

fn read_def_module(
    element: &Element,
) -> Result<(String, v4::AccessControlled<v4::ModuleDefinition>), TransportDiagnostic> {
    let names = annotation_names(element)?;
    if !matches!(names.as_slice(), [_, "def", "module"]) {
        return Err(member(
            "expected public::def::module or private::def::module",
        ));
    }
    let access = access_of(&names)?;
    let fields = struct_fields(element, "module")?;
    refuse_top_level_metadata(&fields)?;
    annotations::refuse_on_definition(&fields)?;
    let name = canonical_module(required_string(&fields, "name")?)?;
    let mut types = IndexMap::new();
    for item in list_items(&fields, "types")? {
        let (type_name, defined) = read_type_def(item)?;
        insert_new(&mut types, "type", type_name, defined)?;
    }
    let mut values = IndexMap::new();
    for item in list_items(&fields, "values")? {
        let (value_name, defined) = read_value_def(item)?;
        insert_new(&mut values, "value", value_name, defined)?;
    }
    Ok((
        name,
        v4::AccessControlled {
            access,
            value: v4::ModuleDefinition {
                types,
                values,
                doc: optional_doc(&fields)?,
            },
        },
    ))
}

fn refuse_top_level_metadata(fields: &BTreeMap<&str, &Element>) -> Result<(), TransportDiagnostic> {
    if fields.contains_key("facts") || fields.contains_key("@context") {
        Err(member(
            "linked metadata on a declaration belongs inside annotations; Type and Value expressions use attributes",
        ))
    } else {
        Ok(())
    }
}

fn def_module(
    name: &str,
    module: &v4::AccessControlled<v4::ModuleDefinition>,
) -> Result<Element, TransportDiagnostic> {
    let mut builder = ion_rs::Struct::builder().with_field("name", name);
    if let Some(doc) = &module.value.doc {
        builder = builder.with_field("doc", doc.text());
    }
    if !module.value.types.is_empty() {
        let types = module
            .value
            .types
            .iter()
            .map(|(type_name, defined)| write_type_def(type_name, defined))
            .collect::<Result<Vec<_>, _>>()?;
        builder = builder.with_field("types", list(types));
    }
    if !module.value.values.is_empty() {
        let values = module
            .value
            .values
            .iter()
            .map(|(value_name, defined)| write_value_def(value_name, defined))
            .collect::<Result<Vec<_>, _>>()?;
        builder = builder.with_field("values", list(values));
    }
    Ok(Element::from(builder.build()).with_annotations([
        access_symbol(&module.access),
        "def",
        "module",
    ]))
}

// =============================================================================
// Shared helpers
// =============================================================================

fn list_items<'a>(
    fields: &BTreeMap<&str, &'a Element>,
    key: &str,
) -> Result<Vec<&'a Element>, TransportDiagnostic> {
    match fields.get(key) {
        None => Ok(Vec::new()),
        Some(element) => Ok(element
            .as_list()
            .ok_or_else(|| member(format!("{key} is a list")))?
            .iter()
            .collect()),
    }
}

fn name_list(
    fields: &BTreeMap<&str, &Element>,
    key: &str,
) -> Result<Vec<Name>, TransportDiagnostic> {
    list_items(fields, key)?
        .into_iter()
        .map(|item| {
            let text = item
                .as_string()
                .ok_or_else(|| member(format!("{key} holds canonical names")))?;
            local_name(text)
        })
        .collect()
}

fn name_elements(names: &[Name]) -> ion_rs::List {
    list(
        names
            .iter()
            .map(|name| Element::string(name.to_canonical_string()))
            .collect(),
    )
}

fn optional_doc(
    fields: &BTreeMap<&str, &Element>,
) -> Result<Option<v4::Documentation>, TransportDiagnostic> {
    Ok(super::optional_string(fields, "doc")?.map(|text| v4::Documentation::new(text.to_owned())))
}

/// Morphir annotations are not read or written yet, so a stated list is refused rather than
/// dropped.
fn refuse_annotations(fields: &BTreeMap<&str, &Element>) -> Result<(), TransportDiagnostic> {
    match fields.get("annotations") {
        Some(list) if list.as_list().is_some_and(|items| items.is_empty()) => Ok(()),
        Some(_) => Err(unwritten("Morphir annotations")),
        None => Ok(()),
    }
}

fn access_of(names: &[&str]) -> Result<Access, TransportDiagnostic> {
    match names.first().copied() {
        Some("public") => Ok(Access::Public),
        Some("private") => Ok(Access::Private),
        _ => Err(member("a definition starts with public or private")),
    }
}

fn access_symbol(access: &Access) -> &'static str {
    match access {
        Access::Public => "public",
        Access::Private => "private",
    }
}

fn package_name(text: &str) -> Result<PackageName, TransportDiagnostic> {
    PackageName::from_canonical_string(text).map_err(member)
}

/// A package name as the model keys it: the canonical spelling.
fn canonical_package(text: &str) -> Result<String, TransportDiagnostic> {
    Ok(package_name(text)?.to_canonical_string())
}

/// A module name as the model keys it: the canonical spelling.
fn canonical_module(text: &str) -> Result<String, TransportDiagnostic> {
    Ok(morphir_core::naming::Path::from_canonical_string(text)
        .map_err(member)?
        .to_canonical_string())
}

fn local_name(text: &str) -> Result<Name, TransportDiagnostic> {
    Name::from_canonical_string(text).map_err(member)
}

fn fq_name(text: &str) -> Result<FQName, TransportDiagnostic> {
    FQName::from_canonical_string(text).map_err(member)
}

fn list(elements: Vec<Element>) -> ion_rs::List {
    elements
        .into_iter()
        .fold(ion_rs::Sequence::builder(), |builder, element| {
            builder.push(element)
        })
        .build_list()
}

/// Inserts an entry whose name must be new. A repeated name is refused, as the v3 reader and the
/// tree merge refuse it.
fn insert_new<T>(
    entries: &mut IndexMap<String, T>,
    kind: &str,
    name: String,
    entry: T,
) -> Result<(), TransportDiagnostic> {
    if entries.contains_key(&name) {
        return Err(IonCodec::error(
            "morphir::ir::ion::duplicate_name",
            Stage::Normalization,
            format!("{kind} '{name}' is already defined"),
        ));
    }
    entries.insert(name, entry);
    Ok(())
}

/// A repeated package element is a fragment of the same dependency, so its modules merge.
fn merge_modules<P, M>(
    packages: &mut IndexMap<String, P>,
    name: String,
    package: P,
    modules: impl Fn(&mut P) -> &mut IndexMap<String, M>,
) -> Result<(), TransportDiagnostic> {
    let Some(existing) = packages.get_mut(&name) else {
        packages.insert(name, package);
        return Ok(());
    };
    let mut package = package;
    for (module_name, module) in std::mem::take(modules(&mut package)) {
        insert_new(modules(existing), "module", module_name, module)?;
    }
    Ok(())
}

/// A node the writer does not encode yet. Refusing it keeps a round trip from dropping it.
fn unwritten(what: &str) -> TransportDiagnostic {
    IonCodec::error(
        "morphir::ir::ion::unsupported_node",
        Stage::Encoding,
        format!("the Ion codec does not encode {what} yet"),
    )
}

fn member(message: impl Into<String>) -> TransportDiagnostic {
    IonCodec::error(
        "morphir::ir::ion::invalid_member",
        Stage::Normalization,
        message,
    )
}
