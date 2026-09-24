//! Ion spelling of a v4 distribution.
//!
//! The writer emits one datagram: the header, one `package::spec` for each
//! dependency, then one `public::def::module` or `module::spec` per module
//! with its members inline, then the footer.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use ion_rs::Element;
use morphir_core::ir::v4::{self, Access, TypeAttributes, ValueAttributes};
use morphir_core::naming::{FQName, Name, PackageName};

use super::{
    ION_CONTRACT, IonCodec, annotation_names, expect_marker, required_field, required_string,
    struct_fields,
};
use crate::ir_transport::{Stage, TransportDiagnostic};

pub(super) fn decode(values: &ion_rs::Sequence) -> Result<v4::IRFile, TransportDiagnostic> {
    let header = values.get(0).ok_or_else(|| {
        IonCodec::error(
            "morphir::ir::ion::unexpected_value",
            Stage::Detection,
            "an Ion IR document starts with morphir::",
        )
    })?;
    expect_marker(header, "morphir")?;
    let fields = struct_fields(header, "morphir")?;
    super::accept_ion_version(super::optional_string(&fields, "ionVersion")?)?;
    let version = required_string(&fields, "formatVersion")?;
    if version != "4.0.0" {
        return Err(IonCodec::error(
            "morphir::ir::ion::version_mismatch",
            Stage::Detection,
            format!("a v4 Ion document uses formatVersion 4.0.0, found {version}"),
        ));
    }
    let kind = super::required_text(&fields, "kind")?;
    let package_name = package_name(required_string(&fields, "packageName")?)?;
    let file = match kind {
        "library" => read_library(&fields, values, package_name)?,
        other => {
            return Err(IonCodec::error(
                "morphir::ir::ion::unsupported_kind",
                Stage::Normalization,
                format!("the Ion reader decodes a v4 library, found kind {other}"),
            ));
        }
    };
    Ok(file)
}

pub(super) fn datagram(file: v4::IRFile) -> Result<ion_rs::Sequence, TransportDiagnostic> {
    let v4::Distribution::Library(content) = file.distribution else {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_kind",
            Stage::Encoding,
            "the Ion writer encodes a v4 library",
        ));
    };
    let mut sequence = ion_rs::Sequence::builder().push(header(&content.package_name));
    for (name, spec) in &content.dependencies {
        sequence = sequence.push(package_spec(name, spec)?);
    }
    for (name, module) in &content.def.modules {
        sequence = sequence.push(def_module(name, module)?);
    }
    Ok(sequence.push(footer()).build())
}

fn read_library(
    header: &BTreeMap<&str, &Element>,
    values: &ion_rs::Sequence,
    package_name: PackageName,
) -> Result<v4::IRFile, TransportDiagnostic> {
    if values.len() == 1 {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_node",
            Stage::Normalization,
            "read a v4 library from its datagram",
        ));
    }
    let last = values.len() - 1;
    expect_marker(values.get(last).expect("length checked"), "morphir_footer")?;
    let mut dependencies = IndexMap::new();
    let mut modules = IndexMap::new();
    for index in 1..last {
        let element = values.get(index).expect("index in range");
        match annotation_names(element)?.as_slice() {
            ["package", "spec"] => {
                let (name, spec) = read_package_spec(element)?;
                merge_dependency(&mut dependencies, name, spec)?;
            }
            ["public", "def", "module"] | ["private", "def", "module"] => {
                let (name, module) = read_def_module(element)?;
                insert_new(&mut modules, "module", name, module)?;
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
    let _ = header;
    Ok(v4::IRFile {
        format_version: v4::FormatVersion::String("4.0.0".to_owned()),
        distribution: v4::Distribution::Library(v4::LibraryContent {
            package_name,
            dependencies,
            def: v4::PackageDefinition { modules },
        }),
    })
}

fn read_package_spec(
    element: &Element,
) -> Result<(String, v4::PackageSpecification), TransportDiagnostic> {
    let fields = struct_fields(element, "package::spec")?;
    let name = required_string(&fields, "name")?.to_owned();
    let mut modules = IndexMap::new();
    if let Some(list) = fields.get("modules") {
        let list = list.as_list().ok_or_else(|| member("modules is a list"))?;
        for module in list.iter() {
            let (module_name, spec) = read_module_spec(module)?;
            insert_new(&mut modules, "module", module_name, spec)?;
        }
    }
    Ok((name, v4::PackageSpecification { modules }))
}

fn read_module_spec(
    element: &Element,
) -> Result<(String, v4::ModuleSpecification), TransportDiagnostic> {
    if annotation_names(element)?.as_slice() != ["module", "spec"] {
        return Err(member("expected module::spec"));
    }
    let fields = struct_fields(element, "module::spec")?;
    let name = required_string(&fields, "name")?.to_owned();
    let mut types = IndexMap::new();
    let mut values = IndexMap::new();
    if let Some(list) = fields.get("types") {
        for item in list
            .as_list()
            .ok_or_else(|| member("types is a list"))?
            .iter()
        {
            let (type_name, spec) = read_type_spec(item)?;
            insert_new(&mut types, "type", type_name, spec)?;
        }
    }
    if let Some(list) = fields.get("values") {
        for item in list
            .as_list()
            .ok_or_else(|| member("values is a list"))?
            .iter()
        {
            let (value_name, spec) = read_value_spec(item)?;
            insert_new(&mut values, "value", value_name, spec)?;
        }
    }
    Ok((
        name,
        v4::ModuleSpecification {
            annotations: Vec::new(),
            types,
            values,
            doc: optional_doc(&fields)?,
        },
    ))
}

fn read_type_spec(
    element: &Element,
) -> Result<(String, v4::Documented<v4::TypeSpecification>), TransportDiagnostic> {
    let names = annotation_names(element)?;
    let fields = struct_fields(element, "type spec")?;
    let name = required_string(&fields, "name")?.to_owned();
    let spec = match names.as_slice() {
        ["public", "spec", "opaque", "type"] => v4::TypeSpecification::OpaqueTypeSpecification {
            annotations: Vec::new(),
            type_params: name_list(&fields, "typeParams")?,
        },
        ["public", "spec", "alias", "type"] => v4::TypeSpecification::TypeAliasSpecification {
            annotations: Vec::new(),
            type_params: name_list(&fields, "typeParams")?,
            type_expr: read_type(required_field(&fields, "typeExp")?)?,
        },
        _ => {
            return Err(member(format!(
                "unsupported type spec {}",
                names.join("::")
            )));
        }
    };
    Ok((name, v4::Documented::new(optional_doc(&fields)?, spec)))
}

fn read_value_spec(
    element: &Element,
) -> Result<(String, v4::Documented<v4::ValueSpecification>), TransportDiagnostic> {
    let fields = struct_fields(element, "value spec")?;
    let name = required_string(&fields, "name")?.to_owned();
    Ok((
        name,
        v4::Documented::new(
            optional_doc(&fields)?,
            v4::ValueSpecification {
                annotations: Vec::new(),
                inputs: read_input_struct(fields.get("inputs").copied())?,
                output: read_type(required_field(&fields, "output")?)?,
            },
        ),
    ))
}

fn read_def_module(
    element: &Element,
) -> Result<(String, v4::AccessControlled<v4::ModuleDefinition>), TransportDiagnostic> {
    let access = match annotation_names(element)?.as_slice() {
        ["public", "def", "module"] => Access::Public,
        ["private", "def", "module"] => Access::Private,
        _ => return Err(member("expected a module definition")),
    };
    let fields = struct_fields(element, "module")?;
    let name = required_string(&fields, "name")?.to_owned();
    let mut types = IndexMap::new();
    let mut values = IndexMap::new();
    if let Some(list) = fields.get("types") {
        for item in list
            .as_list()
            .ok_or_else(|| member("types is a list"))?
            .iter()
        {
            let (type_name, defined) = read_type_def(item)?;
            insert_new(&mut types, "type", type_name, defined)?;
        }
    }
    if let Some(list) = fields.get("values") {
        for item in list
            .as_list()
            .ok_or_else(|| member("values is a list"))?
            .iter()
        {
            let (value_name, defined) = read_value_def(item)?;
            insert_new(&mut values, "value", value_name, defined)?;
        }
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

fn read_type_def(
    element: &Element,
) -> Result<
    (
        String,
        v4::AccessControlled<v4::Documented<v4::TypeDefinition>>,
    ),
    TransportDiagnostic,
> {
    let access = leading_access(element)?;
    let names = annotation_names(element)?;
    let fields = struct_fields(element, "type")?;
    let name = required_string(&fields, "name")?.to_owned();
    let defined = match names.as_slice() {
        [_, "def", "alias", "type"] => v4::TypeDefinition::TypeAliasDefinition {
            type_params: name_list(&fields, "typeParams")?,
            type_expr: read_type(required_field(&fields, "typeExp")?)?,
        },
        [_, "def", "incomplete", "type"] => v4::TypeDefinition::IncompleteTypeDefinition {
            type_params: name_list(&fields, "typeParams")?,
            incompleteness: read_incompleteness(required_field(&fields, "incompleteness")?)?,
            partial_type_expr: match fields.get("partialTypeExp") {
                Some(element) => Some(read_type(element)?),
                None => None,
            },
        },
        _ => {
            return Err(member(format!(
                "unsupported type definition {}",
                names.join("::")
            )));
        }
    };
    Ok((
        name,
        v4::AccessControlled {
            access,
            value: v4::Documented::new(optional_doc(&fields)?, defined),
        },
    ))
}

fn read_value_def(
    element: &Element,
) -> Result<
    (
        String,
        v4::AccessControlled<v4::Documented<v4::ValueDefinition>>,
    ),
    TransportDiagnostic,
> {
    let access = leading_access(element)?;
    let names = annotation_names(element)?;
    let fields = struct_fields(element, "value")?;
    let name = required_string(&fields, "name")?.to_owned();
    let input_types = read_input_struct(fields.get("inputTypes").copied())?;
    let output_type = match fields.get("outputType") {
        Some(element) => Some(read_type(element)?),
        None => None,
    };
    let body = match names.as_slice() {
        [_, "def", "value"] => {
            v4::ValueBody::Expression(read_value(required_field(&fields, "body")?)?)
        }
        [_, "def", "native", "value"] => v4::ValueBody::Native {
            native_info: v4::NativeInfo {
                hint: read_hint(required_field(&fields, "hint")?)?,
                description: super::optional_string(&fields, "description")?.map(str::to_owned),
            },
        },
        _ => {
            return Err(member(format!(
                "unsupported value definition {}",
                names.join("::")
            )));
        }
    };
    Ok((
        name,
        v4::AccessControlled {
            access,
            value: v4::Documented::new(
                optional_doc(&fields)?,
                v4::ValueDefinition {
                    input_types,
                    output_type,
                    body,
                },
            ),
        },
    ))
}

fn read_type(element: &Element) -> Result<v4::Type, TransportDiagnostic> {
    if let Some(text) = element.as_string() {
        return if text.contains('#') || text.contains(':') {
            Ok(v4::Type::Reference(
                TypeAttributes::default(),
                fq_name(text)?,
                Vec::new(),
            ))
        } else {
            Ok(v4::Type::Variable(
                TypeAttributes::default(),
                local_name(text)?,
            ))
        };
    }
    match annotation_names(element)?.as_slice() {
        ["reference"] => {
            let fields = struct_fields(element, "reference")?;
            let mut arguments = Vec::new();
            if let Some(list) = fields.get("arguments") {
                for argument in list
                    .as_list()
                    .ok_or_else(|| member("arguments is a list"))?
                    .iter()
                {
                    arguments.push(read_type(argument)?);
                }
            }
            Ok(v4::Type::Reference(
                TypeAttributes::default(),
                fq_name(required_string(&fields, "name")?)?,
                arguments,
            ))
        }
        ["record"] => {
            let fields = struct_fields(element, "record")?;
            Ok(v4::Type::Record(
                TypeAttributes::default(),
                read_record_fields(fields.get("fields").copied())?,
            ))
        }
        ["function"] => {
            let fields = struct_fields(element, "function")?;
            Ok(v4::Type::Function(
                TypeAttributes::default(),
                Box::new(read_type(required_field(&fields, "parameterType")?)?),
                Box::new(read_type(required_field(&fields, "returnType")?)?),
            ))
        }
        ["unit"] => Ok(v4::Type::Unit(TypeAttributes::default())),
        names => Err(member(format!("unsupported v4 type {}", names.join("::")))),
    }
}

fn write_type(ty: &v4::Type) -> Result<Element, TransportDiagnostic> {
    if *ty.attributes() != TypeAttributes::default() {
        return Err(unwritten_attributes("type"));
    }
    Ok(match ty {
        v4::Type::Variable(_, name) => Element::string(name.to_canonical_string()),
        v4::Type::Reference(_, name, arguments) if arguments.is_empty() => {
            Element::string(name.to_canonical_string())
        }
        v4::Type::Reference(_, name, arguments) => {
            let arguments = arguments
                .iter()
                .map(write_type)
                .collect::<Result<Vec<_>, _>>()?;
            Element::from(
                ion_rs::Struct::builder()
                    .with_field("name", name.to_canonical_string())
                    .with_field("arguments", list(arguments))
                    .build(),
            )
            .with_annotations(["reference"])
        }
        v4::Type::Record(_, fields) => {
            let written = fields
                .iter()
                .map(|field| {
                    Ok(Element::from(
                        ion_rs::Struct::builder()
                            .with_field("name", field.name.to_canonical_string())
                            .with_field("type", write_type(&field.tpe)?)
                            .build(),
                    ))
                })
                .collect::<Result<Vec<_>, TransportDiagnostic>>()?;
            Element::from(
                ion_rs::Struct::builder()
                    .with_field("fields", list(written))
                    .build(),
            )
            .with_annotations(["record"])
        }
        v4::Type::Function(_, parameter, return_type) => Element::from(
            ion_rs::Struct::builder()
                .with_field("parameterType", write_type(parameter)?)
                .with_field("returnType", write_type(return_type)?)
                .build(),
        )
        .with_annotations(["function"]),
        v4::Type::Unit(_) => {
            Element::from(ion_rs::Struct::builder().build()).with_annotations(["unit"])
        }
        other => {
            return Err(member(format!(
                "the Ion writer does not encode this v4 type yet: {other:?}"
            )));
        }
    })
}

fn read_value(element: &Element) -> Result<v4::Value, TransportDiagnostic> {
    if let Some(value) = element.as_float() {
        return Ok(v4::Value::Literal(
            ValueAttributes::default(),
            v4::Literal::Float(v4::FloatLiteral::from_f64(value)),
        ));
    }
    let Some(sequence) = element.as_sexp() else {
        return Err(member("a v4 value is an S-expression or a float"));
    };
    let children: Vec<&Element> = sequence.iter().collect();
    let head = children
        .first()
        .and_then(|element| element.as_symbol())
        .and_then(|symbol| symbol.text())
        .ok_or_else(|| member("a value S-expression has a head"))?;
    match head {
        "float" => {
            let lexeme = children
                .get(1)
                .and_then(|element| element.as_string())
                .ok_or_else(|| member("float keeps its lexeme"))?;
            Ok(v4::Value::Literal(
                ValueAttributes::default(),
                v4::Literal::Float(
                    v4::FloatLiteral::from_lexeme(lexeme)
                        .map_err(|error| member(error.to_string()))?,
                ),
            ))
        }
        "hole" => {
            let reason = read_hole_reason(
                children
                    .get(1)
                    .ok_or_else(|| member("a hole has a reason"))?,
            )?;
            let expected = match children.get(2) {
                Some(element) => Some(Box::new(read_type(element)?)),
                None => None,
            };
            Ok(v4::Value::Hole(
                ValueAttributes::default(),
                reason,
                expected,
            ))
        }
        other => Err(member(format!("unsupported v4 value head '{other}'"))),
    }
}

fn write_value(value: &v4::Value) -> Result<Element, TransportDiagnostic> {
    if *value.attributes() != ValueAttributes::default() {
        return Err(unwritten_attributes("value"));
    }
    match value {
        v4::Value::Literal(_, v4::Literal::Float(literal)) => Ok(sexp(vec![
            Element::symbol("float"),
            Element::string(literal.lexeme()),
        ])),
        v4::Value::Hole(_, reason, expected) => {
            let mut items = vec![Element::symbol("hole"), write_hole_reason(reason)?];
            if let Some(expected) = expected {
                items.push(write_type(expected)?);
            }
            Ok(sexp(items))
        }
        other => Err(member(format!(
            "the Ion writer does not encode this v4 value yet: {other:?}"
        ))),
    }
}

fn read_hole_reason(element: &Element) -> Result<v4::HoleReason, TransportDiagnostic> {
    match annotation_names(element)?.as_slice() {
        ["unresolvedReference"] => {
            let fields = struct_fields(element, "unresolvedReference")?;
            Ok(v4::HoleReason::UnresolvedReference {
                target: fq_name(required_string(&fields, "target")?)?,
            })
        }
        names => Err(member(format!(
            "unsupported hole reason {}",
            names.join("::")
        ))),
    }
}

fn write_hole_reason(reason: &v4::HoleReason) -> Result<Element, TransportDiagnostic> {
    match reason {
        v4::HoleReason::UnresolvedReference { target } => Ok(Element::from(
            ion_rs::Struct::builder()
                .with_field("target", target.to_canonical_string())
                .build(),
        )
        .with_annotations(["unresolvedReference"])),
        other => Err(member(format!("unsupported hole reason {other:?}"))),
    }
}

fn read_hint(element: &Element) -> Result<v4::NativeHint, TransportDiagnostic> {
    if let Some(text) = element.as_symbol().and_then(|symbol| symbol.text()) {
        return match text {
            "arithmetic" => Ok(v4::NativeHint::Arithmetic),
            "comparison" => Ok(v4::NativeHint::Comparison),
            "stringOp" => Ok(v4::NativeHint::StringOp),
            "collectionOp" => Ok(v4::NativeHint::CollectionOp),
            other => Err(member(format!("unknown native hint '{other}'"))),
        };
    }
    Err(member("hint is a symbol"))
}

fn write_hint(hint: &v4::NativeHint) -> Result<Element, TransportDiagnostic> {
    let text = match hint {
        v4::NativeHint::Arithmetic => "arithmetic",
        v4::NativeHint::Comparison => "comparison",
        v4::NativeHint::StringOp => "stringOp",
        v4::NativeHint::CollectionOp => "collectionOp",
        v4::NativeHint::PlatformSpecific { .. } => {
            return Err(member("platform-specific hints are not written yet"));
        }
    };
    Ok(Element::symbol(text))
}

fn read_incompleteness(element: &Element) -> Result<v4::Incompleteness, TransportDiagnostic> {
    if element.as_symbol().and_then(|symbol| symbol.text()) == Some("draft") {
        return Ok(v4::Incompleteness::Draft);
    }
    Err(member("incompleteness is draft"))
}

fn read_record_fields(element: Option<&Element>) -> Result<Vec<v4::Field>, TransportDiagnostic> {
    let Some(element) = element else {
        return Ok(Vec::new());
    };
    let list = element
        .as_list()
        .ok_or_else(|| member("fields is a list"))?;
    let mut fields = Vec::new();
    for item in list.iter() {
        let members = struct_fields(item, "field")?;
        fields.push(v4::Field {
            name: local_name(required_string(&members, "name")?)?,
            tpe: read_type(required_field(&members, "type")?)?,
        });
    }
    Ok(fields)
}

fn read_input_struct(
    element: Option<&Element>,
) -> Result<IndexMap<String, v4::Type>, TransportDiagnostic> {
    let Some(element) = element else {
        return Ok(IndexMap::new());
    };
    let value = element
        .as_struct()
        .ok_or_else(|| member("inputs are a struct"))?;
    let mut inputs = IndexMap::new();
    for (symbol, field) in value.fields() {
        let name = symbol
            .text()
            .ok_or_else(|| member("an input name is a symbol"))?
            .to_owned();
        inputs.insert(name, read_type(field)?);
    }
    Ok(inputs)
}

fn write_input_struct(inputs: &IndexMap<String, v4::Type>) -> Result<Element, TransportDiagnostic> {
    let mut builder = ion_rs::Struct::builder();
    for (name, ty) in inputs {
        builder = builder.with_field(name.as_str(), write_type(ty)?);
    }
    Ok(Element::from(builder.build()))
}

fn header(package: &PackageName) -> Element {
    Element::from(ion_rs::ion_struct! {
        "ionVersion": ION_CONTRACT,
        "formatVersion": "4.0.0",
        "kind": Element::symbol("library"),
        "packageName": package.to_canonical_string(),
    })
    .with_annotations(["morphir"])
}

fn footer() -> Element {
    Element::from(ion_rs::Struct::builder().build()).with_annotations(["morphir_footer"])
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
    Ok(Element::from(
        ion_rs::Struct::builder()
            .with_field("name", name)
            .with_field("modules", list(modules))
            .build(),
    )
    .with_annotations(["package", "spec"]))
}

fn module_spec(name: &str, spec: &v4::ModuleSpecification) -> Result<Element, TransportDiagnostic> {
    let mut builder = ion_rs::Struct::builder().with_field("name", name);
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
    if let Some(doc) = &spec.doc {
        builder = builder.with_field("doc", doc.text());
    }
    Ok(Element::from(builder.build()).with_annotations(["module", "spec"]))
}

fn write_type_spec(
    name: &str,
    documented: &v4::Documented<v4::TypeSpecification>,
) -> Result<Element, TransportDiagnostic> {
    let (annotation, mut builder) = match &documented.value {
        v4::TypeSpecification::OpaqueTypeSpecification { type_params, .. } => {
            let mut builder = ion_rs::Struct::builder().with_field("name", name);
            if !type_params.is_empty() {
                builder = builder.with_field("typeParams", name_elements(type_params));
            }
            ("opaque", builder)
        }
        v4::TypeSpecification::TypeAliasSpecification {
            type_params,
            type_expr,
            ..
        } => {
            let mut builder = ion_rs::Struct::builder()
                .with_field("name", name)
                .with_field("typeExp", write_type(type_expr)?);
            if !type_params.is_empty() {
                builder = builder.with_field("typeParams", name_elements(type_params));
            }
            ("alias", builder)
        }
        other => return Err(member(format!("unsupported type spec {other:?}"))),
    };
    if let Some(doc) = &documented.doc {
        builder = builder.with_field("doc", doc.text());
    }
    Ok(Element::from(builder.build()).with_annotations(["public", "spec", annotation, "type"]))
}

fn write_value_spec(
    name: &str,
    documented: &v4::Documented<v4::ValueSpecification>,
) -> Result<Element, TransportDiagnostic> {
    let mut builder = ion_rs::Struct::builder()
        .with_field("name", name)
        .with_field("output", write_type(&documented.value.output)?);
    if !documented.value.inputs.is_empty() {
        builder = builder.with_field("inputs", write_input_struct(&documented.value.inputs)?);
    }
    if let Some(doc) = &documented.doc {
        builder = builder.with_field("doc", doc.text());
    }
    Ok(Element::from(builder.build()).with_annotations(["public", "spec", "value"]))
}

fn def_module(
    name: &str,
    module: &v4::AccessControlled<v4::ModuleDefinition>,
) -> Result<Element, TransportDiagnostic> {
    let access = match module.access {
        Access::Public => "public",
        Access::Private => "private",
    };
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
    Ok(Element::from(builder.build()).with_annotations([access, "def", "module"]))
}

fn write_type_def(
    name: &str,
    defined: &v4::AccessControlled<v4::Documented<v4::TypeDefinition>>,
) -> Result<Element, TransportDiagnostic> {
    let access = match defined.access {
        Access::Public => "public",
        Access::Private => "private",
    };
    let (kind, mut builder) = match &defined.value.value {
        v4::TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr,
        } => {
            let mut builder = ion_rs::Struct::builder()
                .with_field("name", name)
                .with_field("typeExp", write_type(type_expr)?);
            if !type_params.is_empty() {
                builder = builder.with_field("typeParams", name_elements(type_params));
            }
            ("alias", builder)
        }
        v4::TypeDefinition::IncompleteTypeDefinition {
            type_params,
            incompleteness,
            partial_type_expr,
        } => {
            let mut builder = ion_rs::Struct::builder()
                .with_field("name", name)
                .with_field("incompleteness", Element::symbol("draft"));
            if !matches!(incompleteness, v4::Incompleteness::Draft) {
                return Err(member("only a draft incomplete type is written"));
            }
            if !type_params.is_empty() {
                builder = builder.with_field("typeParams", name_elements(type_params));
            }
            if let Some(partial) = partial_type_expr {
                builder = builder.with_field("partialTypeExp", write_type(partial)?);
            }
            ("incomplete", builder)
        }
        other => return Err(member(format!("unsupported type definition {other:?}"))),
    };
    if let Some(doc) = &defined.value.doc {
        builder = builder.with_field("doc", doc.text());
    }
    Ok(Element::from(builder.build()).with_annotations([access, "def", kind, "type"]))
}

fn write_value_def(
    name: &str,
    defined: &v4::AccessControlled<v4::Documented<v4::ValueDefinition>>,
) -> Result<Element, TransportDiagnostic> {
    let access = match defined.access {
        Access::Public => "public",
        Access::Private => "private",
    };
    let definition = &defined.value.value;
    let (kind, mut builder) = match &definition.body {
        v4::ValueBody::Expression(body) => (
            "value",
            ion_rs::Struct::builder()
                .with_field("name", name)
                .with_field("body", write_value(body)?),
        ),
        v4::ValueBody::Native { native_info } => {
            let mut builder = ion_rs::Struct::builder()
                .with_field("name", name)
                .with_field("hint", write_hint(&native_info.hint)?);
            if let Some(description) = &native_info.description {
                builder = builder.with_field("description", description.as_str());
            }
            ("native", builder)
        }
        other => return Err(member(format!("unsupported value body {other:?}"))),
    };
    if !definition.input_types.is_empty() {
        builder = builder.with_field("inputTypes", write_input_struct(&definition.input_types)?);
    }
    if let Some(output) = &definition.output_type {
        builder = builder.with_field("outputType", write_type(output)?);
    }
    if let Some(doc) = &defined.value.doc {
        builder = builder.with_field("doc", doc.text());
    }
    let annotations: &[&str] = if kind == "native" {
        &[access, "def", "native", "value"]
    } else {
        &[access, "def", "value"]
    };
    Ok(Element::from(builder.build()).with_annotations(annotations.iter().copied()))
}

fn name_list(
    fields: &BTreeMap<&str, &Element>,
    key: &str,
) -> Result<Vec<Name>, TransportDiagnostic> {
    let Some(element) = fields.get(key) else {
        return Ok(Vec::new());
    };
    let list = element
        .as_list()
        .ok_or_else(|| member(format!("{key} is a list")))?;
    list.iter()
        .map(|item| {
            let text = item
                .as_string()
                .ok_or_else(|| member("a type parameter is a name"))?;
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

fn leading_access(element: &Element) -> Result<Access, TransportDiagnostic> {
    match annotation_names(element)?.first().copied() {
        Some("public") => Ok(Access::Public),
        Some("private") => Ok(Access::Private),
        _ => Err(member("a definition starts with public or private")),
    }
}

fn package_name(text: &str) -> Result<PackageName, TransportDiagnostic> {
    PackageName::from_canonical_string(text).map_err(member)
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

fn sexp(items: Vec<Element>) -> Element {
    Element::from(
        items
            .into_iter()
            .fold(ion_rs::Sequence::builder(), |builder, element| {
                builder.push(element)
            })
            .build_sexp(),
    )
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

/// A repeated `package::spec` is a fragment of the same dependency, so its modules merge.
fn merge_dependency(
    dependencies: &mut IndexMap<String, v4::PackageSpecification>,
    name: String,
    spec: v4::PackageSpecification,
) -> Result<(), TransportDiagnostic> {
    let Some(existing) = dependencies.get_mut(&name) else {
        dependencies.insert(name, spec);
        return Ok(());
    };
    for (module_name, module) in spec.modules {
        insert_new(&mut existing.modules, "module", module_name, module)?;
    }
    Ok(())
}

/// The writer does not encode v4 attributes yet. Refusing them keeps a round trip from dropping
/// them without a word.
fn unwritten_attributes(node: &str) -> TransportDiagnostic {
    IonCodec::error(
        "morphir::ir::ion::unsupported_node",
        Stage::Encoding,
        format!("the Ion writer does not encode v4 {node} attributes yet"),
    )
}

fn member(message: impl Into<String>) -> TransportDiagnostic {
    IonCodec::error(
        "morphir::ir::ion::invalid_member",
        Stage::Normalization,
        message,
    )
}
