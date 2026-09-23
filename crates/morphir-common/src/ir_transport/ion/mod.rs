//! Amazon Ion codec for a single-file Morphir IR distribution.
//!
//! `ionVersion` is the spelling contract. A missing value means the latest
//! version this reader implements. `formatVersion` selects the IR.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{Read, Write};

use ion_rs::{Element, IonType, Symbol};
use morphir_core::ir::classic;
use morphir_core::traversal::IrCursor;

use super::semantic;

mod type_expr;
mod value_expr;
use super::{
    CodecOptions, EventSink, EventSource, FormatId, IrCodec, IrVersion, Stage, TransportDiagnostic,
};

/// Spelling contract implemented by this reader. Drafts match only this string.
const ION_CONTRACT: &str = "0.1.0-draft.1";

const HEADER_MEMBERS: &[&str] = &[
    "critical",
    "formatVersion",
    "ionVersion",
    "kind",
    "modules",
    "packageName",
];

const MODULE_MEMBERS: &[&str] = &["critical", "doc", "name", "package", "types", "values"];

type ClassicAlias = (
    classic::Name,
    classic::AccessControlled<classic::Documented<classic::TypeDefinition<classic::Attrs>>>,
);

const ALIAS_MEMBERS: &[&str] = &[
    "critical",
    "doc",
    "module",
    "name",
    "package",
    "typeExp",
    "typeParams",
];

type ClassicModule = classic::ModuleEntry<classic::Attrs, classic::Type<classic::Attrs>>;

/// Built-in Ion IR codec.
pub struct IonCodec {
    format: FormatId,
}

impl IonCodec {
    /// Create the built-in Ion codec.
    pub fn new() -> Self {
        Self {
            format: FormatId::ion(),
        }
    }

    fn error(code: &'static str, stage: Stage, message: impl Into<String>) -> TransportDiagnostic {
        TransportDiagnostic::error(code, stage, IrCursor::root(), message)
    }
}

impl Default for IonCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl IrCodec for IonCodec {
    fn format(&self) -> &FormatId {
        &self.format
    }

    fn decode(
        &self,
        reader: &mut dyn Read,
        options: &CodecOptions,
        sink: &mut dyn EventSink,
    ) -> Result<(), TransportDiagnostic> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).map_err(|error| {
            IonCodec::error(
                "morphir::ir::ion::read_failed",
                Stage::Syntax,
                error.to_string(),
            )
        })?;
        let values = Element::read_all(&bytes).map_err(|error| {
            IonCodec::error(
                "morphir::ir::ion::invalid_syntax",
                Stage::Syntax,
                error.to_string(),
            )
            .with_guidance("correct the Ion syntax or select the actual input format")
        })?;
        let header = values.get(0).ok_or_else(|| {
            IonCodec::error(
                "morphir::ir::ion::unexpected_value",
                Stage::Detection,
                "an Ion IR document starts with morphir::",
            )
        })?;
        expect_marker(header, "morphir")?;
        let header_fields = struct_fields(header, "morphir")?;
        let library = read_library_header(&header_fields, options.version())?;
        let modules = read_library_modules(&values, &header_fields, &library.package)?;
        let distribution = classic::Distribution {
            format_version: library.format_version,
            distribution: classic::DistributionBody::Library(
                library.package,
                Vec::new(),
                classic::PackageDefinition { modules },
            ),
        };
        semantic::emit_classic_v3(distribution, sink)
    }

    fn encode(
        &self,
        source: &mut dyn EventSource,
        writer: &mut dyn Write,
        options: &CodecOptions,
    ) -> Result<(), TransportDiagnostic> {
        match semantic::collect(source, options.version())? {
            semantic::SemanticFile::ClassicV3(distribution) => {
                write_v3_library(distribution, writer)
            }
            semantic::SemanticFile::V4(_) => Err(IonCodec::error(
                "morphir::ir::ion::encode_unsupported",
                Stage::Encoding,
                "the Ion codec encodes formatVersion 3 only",
            )),
        }
    }
}

fn write_v3_library(
    distribution: classic::Distribution,
    writer: &mut dyn Write,
) -> Result<(), TransportDiagnostic> {
    if distribution.format_version != 3 {
        return Err(IonCodec::error(
            "morphir::ir::ion::version_mismatch",
            Stage::Encoding,
            format!(
                "the v3 Ion writer received formatVersion {}",
                distribution.format_version
            ),
        ));
    }
    let classic::DistributionBody::Library(package, dependencies, definition) =
        distribution.distribution;
    if !dependencies.is_empty() {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_node",
            Stage::Encoding,
            "the Ion writer does not encode dependencies yet",
        ));
    }
    let header = Element::from(ion_rs::ion_struct! {
        "ionVersion": ION_CONTRACT,
        "formatVersion": "3.0.0",
        "kind": Element::symbol("library"),
        "packageName": canonical_package(&package),
    })
    .with_annotations(["morphir"]);
    let footer =
        Element::from(ion_rs::Struct::builder().build()).with_annotations(["morphir_footer"]);
    let mut sequence = ion_rs::Sequence::builder().push(header);
    for module in &definition.modules {
        sequence = sequence.push(module_element(module)?);
        for type_definition in &module.definition.value.types {
            sequence = sequence.push(type_element(&module.path, type_definition)?);
        }
        for value_definition in &module.definition.value.values {
            sequence = sequence.push(value_element(&module.path, value_definition)?);
        }
    }
    let text: String = sequence
        .push(footer)
        .build()
        .encode_as(ion_rs::v1_0::Text.with_format(ion_rs::TextFormat::Pretty))
        .map_err(|error| {
            IonCodec::error(
                "morphir::ir::ion::encode_failed",
                Stage::Encoding,
                error.to_string(),
            )
        })?;
    writer.write_all(text.as_bytes()).map_err(|error| {
        IonCodec::error(
            "morphir::ir::ion::encode_failed",
            Stage::Encoding,
            error.to_string(),
        )
    })?;
    if !text.ends_with('\n') {
        writer.write_all(b"\n").map_err(|error| {
            IonCodec::error(
                "morphir::ir::ion::encode_failed",
                Stage::Encoding,
                error.to_string(),
            )
        })?;
    }
    Ok(())
}

fn module_element(module: &ClassicModule) -> Result<Element, TransportDiagnostic> {
    let definition = &module.definition.value;
    let access = match module.definition.access {
        classic::Access::Public => "public",
        classic::Access::Private => "private",
    };
    let mut builder = ion_rs::Struct::builder().with_field("name", canonical_package(&module.path));
    if let Some(doc) = definition.doc.as_deref().filter(|doc| !doc.is_empty()) {
        builder = builder.with_field("doc", doc);
    }
    Ok(Element::from(builder.build()).with_annotations([access, "def", "module"]))
}

fn alias_element(
    module: &classic::Path,
    definition: &ClassicAlias,
) -> Result<Element, TransportDiagnostic> {
    let (name, body) = definition;
    let classic::TypeDefinition::Alias(parameters, type_exp) = &body.value.value else {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_node",
            Stage::Encoding,
            "the Ion writer encodes alias types only",
        ));
    };
    let access = match body.access {
        classic::Access::Public => "public",
        classic::Access::Private => "private",
    };
    let mut builder = ion_rs::Struct::builder()
        .with_field("module", canonical_package(module))
        .with_field("name", canonical_name(name))
        .with_field("typeExp", type_expr::write_type(type_exp));
    if !parameters.is_empty() {
        let parameters = parameters
            .iter()
            .map(canonical_name)
            .fold(ion_rs::Sequence::builder(), |builder, name| {
                builder.push(name)
            })
            .build_list();
        builder = builder.with_field("typeParams", parameters);
    }
    if !body.value.doc.is_empty() {
        builder = builder.with_field("doc", body.value.doc.as_str());
    }
    Ok(Element::from(builder.build()).with_annotations([access, "def", "alias", "type"]))
}

fn type_element(
    module: &classic::Path,
    definition: &ClassicAlias,
) -> Result<Element, TransportDiagnostic> {
    match &definition.1.value.value {
        classic::TypeDefinition::Alias(_, _) => alias_element(module, definition),
        classic::TypeDefinition::Custom(parameters, constructors) => {
            custom_element(module, definition, parameters, constructors)
        }
    }
}

fn custom_element(
    module: &classic::Path,
    definition: &ClassicAlias,
    parameters: &[classic::Name],
    constructors: &classic::AccessControlled<Vec<classic::Constructor<classic::Attrs>>>,
) -> Result<Element, TransportDiagnostic> {
    let (name, body) = definition;
    let access = access_symbol(&body.access);
    let mut builder = ion_rs::Struct::builder()
        .with_field("module", canonical_package(module))
        .with_field("name", canonical_name(name))
        .with_field(
            "access",
            Element::symbol(access_symbol(&constructors.access)),
        )
        .with_field(
            "constructors",
            type_expr::write_constructors(&constructors.value),
        );
    if !parameters.is_empty() {
        builder = builder.with_field("typeParams", name_list(parameters));
    }
    if !body.value.doc.is_empty() {
        builder = builder.with_field("doc", body.value.doc.as_str());
    }
    Ok(Element::from(builder.build()).with_annotations([access, "def", "custom", "type"]))
}

fn value_element(
    module: &classic::Path,
    definition: &value_expr::ClassicValueEntry,
) -> Result<Element, TransportDiagnostic> {
    let (name, body) = definition;
    let access = access_symbol(&body.access);
    let mut builder = ion_rs::Struct::builder()
        .with_field("module", canonical_package(module))
        .with_field("name", canonical_name(name))
        .with_field(
            "outputType",
            type_expr::write_type(&body.value.value.output_type),
        )
        .with_field("body", value_expr::write_value(&body.value.value.body)?);
    if !body.value.value.input_types.is_empty() {
        builder = builder.with_field(
            "inputTypes",
            value_expr::write_inputs(&body.value.value.input_types),
        );
    }
    if !body.value.doc.is_empty() {
        builder = builder.with_field("doc", body.value.doc.as_str());
    }
    Ok(Element::from(builder.build()).with_annotations([access, "def", "value"]))
}

fn access_symbol(access: &classic::Access) -> &'static str {
    match access {
        classic::Access::Public => "public",
        classic::Access::Private => "private",
    }
}

fn name_list(names: &[classic::Name]) -> ion_rs::List {
    names
        .iter()
        .map(canonical_name)
        .fold(ion_rs::Sequence::builder(), |builder, name| {
            builder.push(name)
        })
        .build_list()
}

fn canonical_name(name: &classic::Name) -> String {
    let words = name
        .words
        .iter()
        .copied()
        .map(morphir_core::naming::resolve);
    morphir_core::naming::Name::from_words(words).to_canonical_string()
}

fn canonical_package(path: &classic::Path) -> String {
    path.segments
        .iter()
        .map(canonical_name)
        .collect::<Vec<_>>()
        .join("/")
}

struct LibraryHeader {
    format_version: u32,
    package: classic::Path,
}

fn read_library_header(
    fields: &BTreeMap<&str, &Element>,
    selected: IrVersion,
) -> Result<LibraryHeader, TransportDiagnostic> {
    reject_critical_unknowns(fields, HEADER_MEMBERS)?;
    accept_ion_version(optional_string(fields, "ionVersion")?)?;
    let format_version = required_string(fields, "formatVersion")?;
    let major = accept_format_version(format_version, selected)?;
    if selected != IrVersion::V3 {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_version",
            Stage::Normalization,
            "the Ion codec decodes formatVersion 3 only",
        ));
    }
    let kind = required_text(fields, "kind")?;
    if kind != "library" {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_kind",
            Stage::Normalization,
            format!("a v3 Ion distribution has kind library, found {kind}"),
        ));
    }
    let package_name = required_string(fields, "packageName")?;
    Ok(LibraryHeader {
        format_version: major,
        package: classic_path(package_name)?,
    })
}

fn read_library_modules(
    values: &ion_rs::Sequence,
    header_fields: &BTreeMap<&str, &Element>,
    package: &classic::Path,
) -> Result<Vec<ClassicModule>, TransportDiagnostic> {
    if values.len() == 1 {
        return read_inline_modules(header_fields, package);
    }
    reject_inline_modules(header_fields)?;
    let last = values.len() - 1;
    let footer = values.get(last).expect("length is at least 2");
    expect_marker(footer, "morphir_footer")?;
    let footer_fields = struct_fields(footer, "morphir_footer")?;
    if !footer_fields.is_empty() {
        return Err(IonCodec::error(
            "morphir::ir::ion::unexpected_member",
            Stage::Normalization,
            "morphir_footer has no members",
        ));
    }
    let mut modules = Vec::new();
    let mut seen = HashSet::new();
    for index in 1..last {
        let element = values.get(index).expect("index is in range");
        match annotation_names(element)?.as_slice() {
            ["public", "def", "module"] | ["private", "def", "module"] => {
                let module = read_def_module(element, package)?;
                if !seen.insert(module.path.clone()) {
                    return Err(duplicate_name("module", &module.path));
                }
                modules.push(module);
            }
            ["public", "def", "alias", "type"] | ["private", "def", "alias", "type"] => {
                attach_alias(element, package, &mut modules)?;
            }
            ["public", "def", "custom", "type"] | ["private", "def", "custom", "type"] => {
                attach_custom(element, package, &mut modules)?;
            }
            ["public", "def", "value"] | ["private", "def", "value"] => {
                attach_value(element, package, &mut modules)?;
            }
            names => {
                return Err(IonCodec::error(
                    "morphir::ir::ion::unexpected_value",
                    Stage::Detection,
                    format!(
                        "expected a module, type, or value, found {}",
                        display_annotations(names)
                    ),
                ));
            }
        }
    }
    Ok(modules)
}

fn read_inline_modules(
    fields: &BTreeMap<&str, &Element>,
    package: &classic::Path,
) -> Result<Vec<ClassicModule>, TransportDiagnostic> {
    let Some(modules) = fields.get("modules") else {
        return Ok(Vec::new());
    };
    let Some(list) = modules.as_list() else {
        return Err(IonCodec::error(
            "morphir::ir::ion::invalid_member",
            Stage::Normalization,
            "modules is a list",
        ));
    };
    let mut decoded = Vec::new();
    let mut seen = HashSet::new();
    for element in list.iter() {
        let module = read_def_module(element, package)?;
        if !seen.insert(module.path.clone()) {
            return Err(duplicate_name("module", &module.path));
        }
        decoded.push(module);
    }
    Ok(decoded)
}

fn reject_inline_modules(fields: &BTreeMap<&str, &Element>) -> Result<(), TransportDiagnostic> {
    let Some(modules) = fields.get("modules") else {
        return Ok(());
    };
    let Some(list) = modules.as_list() else {
        return Err(IonCodec::error(
            "morphir::ir::ion::invalid_member",
            Stage::Normalization,
            "modules is a list",
        ));
    };
    if list.is_empty() {
        return Ok(());
    }
    Err(IonCodec::error(
        "morphir::ir::ion::unsupported_node",
        Stage::Normalization,
        "a datagram writes each module as its own value",
    ))
}

fn read_def_module(
    element: &Element,
    package: &classic::Path,
) -> Result<ClassicModule, TransportDiagnostic> {
    let access = match annotation_names(element)?.as_slice() {
        ["public", "def", "module"] => classic::Access::Public,
        ["private", "def", "module"] => classic::Access::Private,
        names => {
            return Err(IonCodec::error(
                "morphir::ir::ion::unexpected_value",
                Stage::Detection,
                format!(
                    "expected public::def::module, found {}",
                    display_annotations(names)
                ),
            ));
        }
    };
    let fields = struct_fields(element, "module")?;
    reject_critical_unknowns(&fields, MODULE_MEMBERS)?;
    require_distribution_package(&fields, package)?;
    let path = classic_path(required_string(&fields, "name")?)?;
    Ok(classic::ModuleEntry {
        definition: classic::AccessControlled {
            access,
            value: classic::ModuleDefinition {
                types: read_inline_types(fields.get("types").copied(), package, &path)?,
                values: read_inline_values(fields.get("values").copied(), package, &path)?,
                doc: optional_string(&fields, "doc")?.map(str::to_owned),
            },
        },
        path,
    })
}

fn read_inline_types(
    types: Option<&Element>,
    package: &classic::Path,
    owner: &classic::Path,
) -> Result<Vec<ClassicAlias>, TransportDiagnostic> {
    let Some(types) = types else {
        return Ok(Vec::new());
    };
    let Some(list) = types.as_list() else {
        return Err(IonCodec::error(
            "morphir::ir::ion::invalid_member",
            Stage::Normalization,
            "types is a list",
        ));
    };
    let mut decoded = Vec::new();
    for element in list.iter() {
        let (_owner, definition) = read_type_member(element, package, Some(owner))?;
        if decoded.iter().any(|(name, _)| name == &definition.0) {
            return Err(duplicate_name(
                "type",
                &classic::Path::new(vec![definition.0.clone()]),
            ));
        }
        decoded.push(definition);
    }
    Ok(decoded)
}

fn read_type_member(
    element: &Element,
    package: &classic::Path,
    owner: Option<&classic::Path>,
) -> Result<(classic::Path, ClassicAlias), TransportDiagnostic> {
    match annotation_names(element)?.as_slice() {
        ["public", "def", "alias", "type"] | ["private", "def", "alias", "type"] => {
            read_alias(element, package, owner)
        }
        ["public", "def", "custom", "type"] | ["private", "def", "custom", "type"] => {
            read_custom(element, package, owner)
        }
        names => Err(IonCodec::error(
            "morphir::ir::ion::unexpected_value",
            Stage::Detection,
            format!(
                "expected an alias or a custom type, found {}",
                display_annotations(names)
            ),
        )),
    }
}

fn attach_alias(
    element: &Element,
    package: &classic::Path,
    modules: &mut [ClassicModule],
) -> Result<(), TransportDiagnostic> {
    let (module_path, definition) = read_alias(element, package, None)?;
    let module = modules
        .iter_mut()
        .find(|module| module.path == module_path)
        .ok_or_else(|| {
            IonCodec::error(
                "morphir::ir::ion::missing_member",
                Stage::Normalization,
                format!(
                    "module '{}' is not defined",
                    canonical_package(&module_path)
                ),
            )
        })?;
    if module
        .definition
        .value
        .types
        .iter()
        .any(|(existing, _)| existing == &definition.0)
    {
        return Err(duplicate_name(
            "type",
            &classic::Path::new(vec![definition.0.clone()]),
        ));
    }
    module.definition.value.types.push(definition);
    Ok(())
}

fn attach_custom(
    element: &Element,
    package: &classic::Path,
    modules: &mut [ClassicModule],
) -> Result<(), TransportDiagnostic> {
    let (module_path, definition) = read_custom(element, package, None)?;
    let module = find_module(modules, &module_path)?;
    if module
        .definition
        .value
        .types
        .iter()
        .any(|(existing, _)| existing == &definition.0)
    {
        return Err(duplicate_name(
            "type",
            &classic::Path::new(vec![definition.0.clone()]),
        ));
    }
    module.definition.value.types.push(definition);
    Ok(())
}

fn attach_value(
    element: &Element,
    package: &classic::Path,
    modules: &mut [ClassicModule],
) -> Result<(), TransportDiagnostic> {
    let (module_path, definition) = read_value_member(element, package, None)?;
    let module = find_module(modules, &module_path)?;
    if module
        .definition
        .value
        .values
        .iter()
        .any(|(existing, _)| existing == &definition.0)
    {
        return Err(duplicate_name(
            "value",
            &classic::Path::new(vec![definition.0.clone()]),
        ));
    }
    module.definition.value.values.push(definition);
    Ok(())
}

fn find_module<'a>(
    modules: &'a mut [ClassicModule],
    module_path: &classic::Path,
) -> Result<&'a mut ClassicModule, TransportDiagnostic> {
    modules
        .iter_mut()
        .find(|module| module.path == *module_path)
        .ok_or_else(|| {
            IonCodec::error(
                "morphir::ir::ion::missing_member",
                Stage::Normalization,
                format!("module '{}' is not defined", canonical_package(module_path)),
            )
        })
}

fn read_alias(
    element: &Element,
    package: &classic::Path,
    owner: Option<&classic::Path>,
) -> Result<(classic::Path, ClassicAlias), TransportDiagnostic> {
    let access = match annotation_names(element)?.as_slice() {
        ["public", "def", "alias", "type"] => classic::Access::Public,
        ["private", "def", "alias", "type"] => classic::Access::Private,
        names => {
            return Err(IonCodec::error(
                "morphir::ir::ion::unexpected_value",
                Stage::Detection,
                format!(
                    "expected public::def::alias::type, found {}",
                    display_annotations(names)
                ),
            ));
        }
    };
    let fields = struct_fields(element, "alias::type")?;
    reject_critical_unknowns(&fields, ALIAS_MEMBERS)?;
    require_distribution_package(&fields, package)?;
    let module_path = match optional_string(&fields, "module")? {
        Some(name) => classic_path(name)?,
        None => owner.cloned().ok_or_else(|| {
            IonCodec::error(
                "morphir::ir::ion::missing_member",
                Stage::Normalization,
                "a top-level type names its module",
            )
        })?,
    };
    if let Some(owner) = owner
        && module_path != *owner
    {
        return Err(IonCodec::error(
            "morphir::ir::ion::unexpected_member",
            Stage::Normalization,
            "a nested type belongs to its module",
        ));
    }
    let name = classic_path(required_string(&fields, "name")?)?;
    if name.segments.len() != 1 {
        return Err(IonCodec::error(
            "morphir::ir::ion::invalid_name",
            Stage::Normalization,
            "a type name is one canonical name",
        ));
    }
    let local_name = name.segments[0].clone();
    let type_exp = type_expr::read_type(required_field(&fields, "typeExp")?)?;
    Ok((
        module_path,
        (
            local_name,
            classic::AccessControlled {
                access,
                value: classic::Documented {
                    doc: optional_string(&fields, "doc")?.unwrap_or("").to_owned(),
                    value: classic::TypeDefinition::Alias(
                        canonical_name_list(&fields, "typeParams")?,
                        type_exp,
                    ),
                },
            },
        ),
    ))
}

fn read_custom(
    element: &Element,
    package: &classic::Path,
    owner: Option<&classic::Path>,
) -> Result<(classic::Path, ClassicAlias), TransportDiagnostic> {
    let access = match annotation_names(element)?.as_slice() {
        ["public", "def", "custom", "type"] => classic::Access::Public,
        ["private", "def", "custom", "type"] => classic::Access::Private,
        names => {
            return Err(IonCodec::error(
                "morphir::ir::ion::unexpected_value",
                Stage::Detection,
                format!(
                    "expected public::def::custom::type, found {}",
                    display_annotations(names)
                ),
            ));
        }
    };
    let fields = struct_fields(element, "custom::type")?;
    require_distribution_package(&fields, package)?;
    let module_path = owned_module(&fields, owner)?;
    let local_name = type_expr::local_name(required_string(&fields, "name")?)?;
    let constructor_access = match required_text(&fields, "access")? {
        "public" => classic::Access::Public,
        "private" => classic::Access::Private,
        other => {
            return Err(IonCodec::error(
                "morphir::ir::ion::invalid_member",
                Stage::Normalization,
                format!("constructor access is public or private, found {other}"),
            ));
        }
    };
    let constructors = match fields.get("constructors") {
        Some(element) => type_expr::read_constructors(element)?,
        None => Vec::new(),
    };
    Ok((
        module_path,
        (
            local_name,
            classic::AccessControlled {
                access,
                value: classic::Documented {
                    doc: optional_string(&fields, "doc")?.unwrap_or("").to_owned(),
                    value: classic::TypeDefinition::Custom(
                        canonical_name_list(&fields, "typeParams")?,
                        classic::AccessControlled {
                            access: constructor_access,
                            value: constructors,
                        },
                    ),
                },
            },
        ),
    ))
}

fn read_value_member(
    element: &Element,
    package: &classic::Path,
    owner: Option<&classic::Path>,
) -> Result<(classic::Path, value_expr::ClassicValueEntry), TransportDiagnostic> {
    let access = match annotation_names(element)?.as_slice() {
        ["public", "def", "value"] => classic::Access::Public,
        ["private", "def", "value"] => classic::Access::Private,
        names => {
            return Err(IonCodec::error(
                "morphir::ir::ion::unexpected_value",
                Stage::Detection,
                format!(
                    "expected public::def::value, found {}",
                    display_annotations(names)
                ),
            ));
        }
    };
    let fields = struct_fields(element, "value")?;
    require_distribution_package(&fields, package)?;
    let module_path = owned_module(&fields, owner)?;
    let local_name = type_expr::local_name(required_string(&fields, "name")?)?;
    Ok((
        module_path,
        (
            local_name,
            classic::AccessControlled {
                access,
                value: classic::Documented {
                    doc: optional_string(&fields, "doc")?.unwrap_or("").to_owned(),
                    value: value_expr::read_definition(&fields)?,
                },
            },
        ),
    ))
}

fn read_inline_values(
    values: Option<&Element>,
    package: &classic::Path,
    owner: &classic::Path,
) -> Result<Vec<value_expr::ClassicValueEntry>, TransportDiagnostic> {
    let Some(values) = values else {
        return Ok(Vec::new());
    };
    let Some(list) = values.as_list() else {
        return Err(IonCodec::error(
            "morphir::ir::ion::invalid_member",
            Stage::Normalization,
            "values is a list",
        ));
    };
    let mut decoded = Vec::new();
    for element in list.iter() {
        let (_owner, definition) = read_value_member(element, package, Some(owner))?;
        if decoded.iter().any(|(name, _)| name == &definition.0) {
            return Err(duplicate_name(
                "value",
                &classic::Path::new(vec![definition.0.clone()]),
            ));
        }
        decoded.push(definition);
    }
    Ok(decoded)
}

fn owned_module(
    fields: &BTreeMap<&str, &Element>,
    owner: Option<&classic::Path>,
) -> Result<classic::Path, TransportDiagnostic> {
    let module_path = match optional_string(fields, "module")? {
        Some(name) => classic_path(name)?,
        None => owner.cloned().ok_or_else(|| {
            IonCodec::error(
                "morphir::ir::ion::missing_member",
                Stage::Normalization,
                "a top-level definition names its module",
            )
        })?,
    };
    if let Some(owner) = owner
        && module_path != *owner
    {
        return Err(IonCodec::error(
            "morphir::ir::ion::unexpected_member",
            Stage::Normalization,
            "a nested definition belongs to its module",
        ));
    }
    Ok(module_path)
}

fn canonical_name_list(
    fields: &BTreeMap<&str, &Element>,
    name: &str,
) -> Result<Vec<classic::Name>, TransportDiagnostic> {
    let Some(element) = fields.get(name) else {
        return Ok(Vec::new());
    };
    let Some(list) = element.as_list() else {
        return Err(IonCodec::error(
            "morphir::ir::ion::invalid_member",
            Stage::Normalization,
            format!("{name} is a list"),
        ));
    };
    let mut names = Vec::new();
    for item in list.iter() {
        let Some(text) = item.as_string() else {
            return Err(IonCodec::error(
                "morphir::ir::ion::invalid_member",
                Stage::Normalization,
                format!("{name} contains a canonical name"),
            ));
        };
        let parsed = morphir_core::naming::Name::from_canonical_string(text).map_err(|error| {
            IonCodec::error(
                "morphir::ir::ion::invalid_name",
                Stage::Normalization,
                error,
            )
        })?;
        names.push(classic_name_from(&parsed));
    }
    Ok(names)
}

fn classic_name_from(name: &morphir_core::naming::Name) -> classic::Name {
    classic::Name::new(name.words())
}

fn classic_path_from(path: &morphir_core::naming::Path) -> classic::Path {
    classic::Path::new(path.segments.iter().map(classic_name_from).collect())
}

fn require_distribution_package(
    fields: &BTreeMap<&str, &Element>,
    package: &classic::Path,
) -> Result<(), TransportDiagnostic> {
    let Some(declared) = optional_string(fields, "package")? else {
        return Ok(());
    };
    if classic_path(declared)? != *package {
        return Err(IonCodec::error(
            "morphir::ir::ion::unexpected_member",
            Stage::Normalization,
            "a v3 definition belongs to the distribution package",
        ));
    }
    Ok(())
}

fn duplicate_name(kind: &str, path: &classic::Path) -> TransportDiagnostic {
    IonCodec::error(
        "morphir::ir::ion::duplicate_name",
        Stage::Normalization,
        format!("{kind} '{}' is already defined", canonical_package(path)),
    )
}

fn accept_ion_version(text: Option<&str>) -> Result<(), TransportDiagnostic> {
    let Some(text) = text else {
        return Ok(());
    };
    let version = semver::Version::parse(text).map_err(|error| {
        IonCodec::error(
            "morphir::ir::ion::unsupported_version",
            Stage::Detection,
            format!("ionVersion '{text}' is not a SemVer version: {error}"),
        )
    })?;
    let supported = semver::Version::parse(ION_CONTRACT).expect("ion contract version parses");
    if version != supported || version.to_string() != text {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_version",
            Stage::Detection,
            format!("this reader implements ionVersion {ION_CONTRACT}, found {text}"),
        )
        .with_guidance("omit ionVersion or set it to the implemented draft"));
    }
    Ok(())
}

fn accept_format_version(text: &str, selected: IrVersion) -> Result<u32, TransportDiagnostic> {
    let version = semver::Version::parse(text).map_err(|error| {
        IonCodec::error(
            "morphir::ir::ion::unsupported_version",
            Stage::Detection,
            format!("formatVersion '{text}' is not a SemVer version: {error}"),
        )
    })?;
    if version.to_string() != text || !version.pre.is_empty() || !version.build.is_empty() {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_version",
            Stage::Detection,
            format!("formatVersion must be a canonical release such as 3.0.0, found {text}"),
        ));
    }
    let major = match selected {
        IrVersion::V3 => 3,
        IrVersion::V4 => 4,
    };
    if version.major != major || version.minor != 0 {
        return Err(IonCodec::error(
            "morphir::ir::ion::version_mismatch",
            Stage::Detection,
            format!(
                "the selected {:?} codec accepts formatVersion {major}.0.x, found {text}",
                selected
            ),
        ));
    }
    Ok(u32::try_from(major).expect("IR major fits in u32"))
}

fn reject_critical_unknowns(
    fields: &BTreeMap<&str, &Element>,
    known_members: &[&str],
) -> Result<(), TransportDiagnostic> {
    let Some(critical) = fields.get("critical") else {
        return Ok(());
    };
    let names = text_list(critical, "critical")?;
    let known: BTreeSet<&str> = known_members.iter().copied().collect();
    for name in names {
        if !known.contains(name.as_str()) {
            return Err(IonCodec::error(
                "morphir::ir::ion::critical_member",
                Stage::Normalization,
                format!("critical names '{name}', which this reader does not understand"),
            ));
        }
    }
    Ok(())
}

fn classic_path(canonical: &str) -> Result<classic::Path, TransportDiagnostic> {
    let path = morphir_core::naming::Path::from_canonical_string(canonical).map_err(|error| {
        IonCodec::error(
            "morphir::ir::ion::invalid_name",
            Stage::Normalization,
            error,
        )
    })?;
    if path.is_empty() {
        return Err(IonCodec::error(
            "morphir::ir::ion::invalid_name",
            Stage::Normalization,
            "a canonical name has at least one segment",
        ));
    }
    let segments = path
        .segments
        .into_iter()
        .map(|name| classic::Name::new(name.words()))
        .collect();
    Ok(classic::Path::new(segments))
}

fn annotation_names(element: &Element) -> Result<Vec<&str>, TransportDiagnostic> {
    let mut names = Vec::new();
    for symbol in element.annotations().iter() {
        names.push(symbol_text(symbol)?);
    }
    Ok(names)
}

fn expect_marker(element: &Element, expected: &str) -> Result<(), TransportDiagnostic> {
    let names = annotation_names(element)?;
    if names != [expected] {
        return Err(IonCodec::error(
            "morphir::ir::ion::unexpected_value",
            Stage::Detection,
            format!(
                "expected {expected}::, found {}",
                display_annotations(&names)
            ),
        ));
    }
    Ok(())
}

fn struct_fields<'a>(
    element: &'a Element,
    marker: &str,
) -> Result<BTreeMap<&'a str, &'a Element>, TransportDiagnostic> {
    if element.is_null() || element.ion_type() != IonType::Struct {
        return Err(IonCodec::error(
            "morphir::ir::ion::unexpected_value",
            Stage::Detection,
            format!("{marker}:: is a struct"),
        ));
    }
    let value = element.as_struct().expect("ion type checked");
    let mut fields = BTreeMap::new();
    for (symbol, field) in value.fields() {
        let name = symbol_text(symbol)?;
        if fields.insert(name, field).is_some() {
            return Err(IonCodec::error(
                "morphir::ir::ion::duplicate_field",
                Stage::Normalization,
                format!("{marker} repeats field '{name}'"),
            ));
        }
    }
    Ok(fields)
}

fn symbol_text(symbol: &Symbol) -> Result<&str, TransportDiagnostic> {
    symbol.text().ok_or_else(|| {
        IonCodec::error(
            "morphir::ir::ion::invalid_symbol",
            Stage::Syntax,
            "an Ion symbol with no text cannot name a Morphir member",
        )
    })
}

fn optional_string<'a>(
    fields: &BTreeMap<&str, &'a Element>,
    name: &str,
) -> Result<Option<&'a str>, TransportDiagnostic> {
    match fields.get(name) {
        None => Ok(None),
        Some(element) => Ok(Some(require_string(element, name)?)),
    }
}

fn required_string<'a>(
    fields: &BTreeMap<&str, &'a Element>,
    name: &str,
) -> Result<&'a str, TransportDiagnostic> {
    let element = required_field(fields, name)?;
    require_string(element, name)
}

fn required_text<'a>(
    fields: &BTreeMap<&str, &'a Element>,
    name: &str,
) -> Result<&'a str, TransportDiagnostic> {
    let element = required_field(fields, name)?;
    if let Some(text) = element.as_string() {
        return Ok(text);
    }
    if let Some(symbol) = element.as_symbol() {
        return symbol_text(symbol);
    }
    Err(IonCodec::error(
        "morphir::ir::ion::invalid_member",
        Stage::Normalization,
        format!("{name} is a string or a symbol"),
    ))
}

fn required_field<'a>(
    fields: &BTreeMap<&str, &'a Element>,
    name: &str,
) -> Result<&'a Element, TransportDiagnostic> {
    fields.get(name).copied().ok_or_else(|| {
        IonCodec::error(
            "morphir::ir::ion::missing_member",
            Stage::Normalization,
            format!("{name} is required"),
        )
    })
}

fn require_string<'a>(element: &'a Element, name: &str) -> Result<&'a str, TransportDiagnostic> {
    element.as_string().ok_or_else(|| {
        IonCodec::error(
            "morphir::ir::ion::invalid_member",
            Stage::Normalization,
            format!("{name} is a string"),
        )
    })
}

fn text_list(element: &Element, name: &str) -> Result<Vec<String>, TransportDiagnostic> {
    let Some(list) = element.as_list() else {
        return Err(IonCodec::error(
            "morphir::ir::ion::invalid_member",
            Stage::Normalization,
            format!("{name} is a list"),
        ));
    };
    let mut texts = Vec::new();
    for item in list.iter() {
        let text = if let Some(text) = item.as_string() {
            text
        } else if let Some(symbol) = item.as_symbol() {
            symbol_text(symbol)?
        } else {
            return Err(IonCodec::error(
                "morphir::ir::ion::invalid_member",
                Stage::Normalization,
                format!("{name} contains a string or a symbol"),
            ));
        };
        texts.push(text.to_owned());
    }
    Ok(texts)
}

fn display_annotations(names: &[&str]) -> String {
    if names.is_empty() {
        "no annotation".to_owned()
    } else {
        names.join("::")
    }
}
