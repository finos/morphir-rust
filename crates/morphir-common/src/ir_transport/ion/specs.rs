//! Ion spelling of v3 dependency specifications.
//!
//! A dependency is `package::spec::{ name, modules }`. Each module is
//! `module::spec::{ name, doc, types, values }`. A type specification is
//! `public::spec::<kind>::type` with kind `opaque`, `alias`, `custom` or `derived`. A value
//! specification is `public::spec::value::{ name, inputs: [ { name, type } ], output }`. Several
//! `package::spec` values with the same name are one package, and their modules merge.

use std::collections::BTreeMap;

use ion_rs::Element;
use morphir_core::ir::classic;

use super::type_expr::{
    canonical_fq, canonical_fq_name, local_name, read_constructors, read_type, write_constructors,
    write_type,
};
use super::{
    IonCodec, annotation_names, canonical_name, canonical_package, classic_path,
    display_annotations, duplicate_name, optional_string, required_field, required_string,
    struct_fields,
};
use crate::ir_transport::{Stage, TransportDiagnostic};

type Spec = classic::PackageSpecification<classic::Attrs>;
pub(super) type Dependency = (classic::Path, Spec);
pub(super) type OwnModule = classic::package::ModuleSpecEntry<classic::Attrs>;

/// Adds one `package::spec` to the dependencies. A repeated package merges its modules, and a
/// repeated module is refused.
pub(super) fn read_package_spec(
    element: &Element,
    distribution: &classic::Path,
    dependencies: &mut Vec<Dependency>,
) -> Result<(), TransportDiagnostic> {
    let fields = struct_fields(element, "package::spec")?;
    let package = classic_path(required_string(&fields, "name")?)?;
    if package == *distribution {
        return Err(IonCodec::error(
            "morphir::ir::ion::unexpected_member",
            Stage::Normalization,
            "a v3 package::spec cannot name the distribution package, whose modules are top-level values",
        ));
    }
    let index = match dependencies.iter().position(|(name, _)| *name == package) {
        Some(index) => index,
        None => {
            dependencies.push((
                package,
                Spec {
                    modules: Vec::new(),
                },
            ));
            dependencies.len() - 1
        }
    };
    let modules = &mut dependencies[index].1.modules;
    if let Some(list) = fields.get("modules") {
        let list = list.as_list().ok_or_else(|| member("modules is a list"))?;
        for module in list.iter() {
            let entry = read_module_spec(module)?;
            if modules.iter().any(|existing| existing.path == entry.path) {
                return Err(duplicate_name("module", &entry.path));
            }
            modules.push(entry);
        }
    }
    Ok(())
}

pub(super) fn write_package_spec(
    (package, spec): &Dependency,
) -> Result<Element, TransportDiagnostic> {
    let mut builder = ion_rs::Struct::builder().with_field("name", canonical_package(package));
    if !spec.modules.is_empty() {
        let modules = spec
            .modules
            .iter()
            .map(write_module_spec)
            .collect::<Result<Vec<_>, _>>()?;
        builder = builder.with_field("modules", list(modules));
    }
    Ok(Element::from(builder.build()).with_annotations(["package", "spec"]))
}

pub(super) fn read_module_spec(element: &Element) -> Result<OwnModule, TransportDiagnostic> {
    let names = annotation_names(element)?;
    if names != ["module", "spec"] {
        return Err(member(format!(
            "expected module::spec, found {}",
            display_annotations(&names)
        )));
    }
    let fields = struct_fields(element, "module::spec")?;
    let mut types: Vec<classic::module::ModuleTypeSpecification<classic::Attrs>> = Vec::new();
    for item in items(&fields, "types")? {
        let (name, spec) = read_type_spec(item)?;
        if types.iter().any(|(existing, _)| *existing == name) {
            return Err(duplicate_name("type", &classic::Path::new(vec![name])));
        }
        types.push((name, spec));
    }
    let mut values: Vec<classic::module::ModuleValueSpecification<classic::Attrs>> = Vec::new();
    for item in items(&fields, "values")? {
        let (name, spec) = read_value_spec(item)?;
        if values.iter().any(|(existing, _)| *existing == name) {
            return Err(duplicate_name("value", &classic::Path::new(vec![name])));
        }
        values.push((name, spec));
    }
    Ok(classic::package::ModuleSpecEntry {
        path: classic_path(required_string(&fields, "name")?)?,
        specification: classic::ModuleSpecification {
            types,
            values,
            doc: optional_string(&fields, "doc")?.map(str::to_owned),
        },
    })
}

pub(super) fn write_module_spec(entry: &OwnModule) -> Result<Element, TransportDiagnostic> {
    let spec = &entry.specification;
    let mut builder = ion_rs::Struct::builder().with_field("name", canonical_package(&entry.path));
    if let Some(doc) = spec.doc.as_deref() {
        builder = builder.with_field("doc", doc);
    }
    if !spec.types.is_empty() {
        let types = spec
            .types
            .iter()
            .map(|(name, spec)| write_type_spec(name, spec))
            .collect::<Result<Vec<_>, _>>()?;
        builder = builder.with_field("types", list(types));
    }
    if !spec.values.is_empty() {
        let values = spec
            .values
            .iter()
            .map(|(name, spec)| write_value_spec(name, spec))
            .collect();
        builder = builder.with_field("values", list(values));
    }
    Ok(Element::from(builder.build()).with_annotations(["module", "spec"]))
}

fn read_type_spec(
    element: &Element,
) -> Result<
    (
        classic::Name,
        classic::Documented<classic::TypeSpecification<classic::Attrs>>,
    ),
    TransportDiagnostic,
> {
    let names = annotation_names(element)?;
    let fields = struct_fields(element, "type spec")?;
    let params = name_list(&fields, "typeParams")?;
    let spec = match names.as_slice() {
        ["public", "spec", "opaque", "type"] => classic::TypeSpecification::Opaque(params),
        ["public", "spec", "alias", "type"] => classic::TypeSpecification::Alias(
            params,
            read_type(required_field(&fields, "typeExp")?)?,
        ),
        ["public", "spec", "custom", "type"] => classic::TypeSpecification::Custom(
            params,
            match fields.get("constructors") {
                Some(element) => read_constructors(element)?,
                None => Vec::new(),
            },
        ),
        ["public", "spec", "derived", "type"] => classic::TypeSpecification::Derived(
            params,
            classic::DerivedTypeConfig {
                base_type: read_type(required_field(&fields, "baseType")?)?,
                from_base_type: canonical_fq_name(required_string(&fields, "fromBaseType")?)?,
                to_base_type: canonical_fq_name(required_string(&fields, "toBaseType")?)?,
            },
        ),
        names => {
            return Err(member(format!(
                "expected a v3 type specification, found {}",
                display_annotations(names)
            )));
        }
    };
    Ok((
        local_name(required_string(&fields, "name")?)?,
        classic::Documented::new(optional_string(&fields, "doc")?.unwrap_or(""), spec),
    ))
}

fn write_type_spec(
    name: &classic::Name,
    spec: &classic::Documented<classic::TypeSpecification<classic::Attrs>>,
) -> Result<Element, TransportDiagnostic> {
    let mut builder = ion_rs::Struct::builder().with_field("name", canonical_name(name));
    let (kind, params) = match &spec.value {
        classic::TypeSpecification::Opaque(params) => ("opaque", params),
        classic::TypeSpecification::Alias(params, ty) => {
            builder = builder.with_field("typeExp", write_type(ty));
            ("alias", params)
        }
        classic::TypeSpecification::Custom(params, constructors) => {
            if !constructors.is_empty() {
                builder = builder.with_field("constructors", write_constructors(constructors));
            }
            ("custom", params)
        }
        classic::TypeSpecification::Derived(params, config) => {
            builder = builder
                .with_field("baseType", write_type(&config.base_type))
                .with_field("fromBaseType", canonical_fq(&config.from_base_type))
                .with_field("toBaseType", canonical_fq(&config.to_base_type));
            ("derived", params)
        }
    };
    if !params.is_empty() {
        builder = builder.with_field("typeParams", names(params));
    }
    if !spec.doc.is_empty() {
        builder = builder.with_field("doc", spec.doc.as_str());
    }
    Ok(Element::from(builder.build()).with_annotations(["public", "spec", kind, "type"]))
}

fn read_value_spec(
    element: &Element,
) -> Result<
    (
        classic::Name,
        classic::Documented<classic::ValueSpecification<classic::Attrs>>,
    ),
    TransportDiagnostic,
> {
    let annotations = annotation_names(element)?;
    if annotations != ["public", "spec", "value"] {
        return Err(member(format!(
            "expected public::spec::value, found {}",
            display_annotations(&annotations)
        )));
    }
    let fields = struct_fields(element, "value spec")?;
    let mut inputs = Vec::new();
    for input in items(&fields, "inputs")? {
        let input_fields = struct_fields(input, "input")?;
        inputs.push(classic::value::ValueParameter {
            name: local_name(required_string(&input_fields, "name")?)?,
            ty: read_type(required_field(&input_fields, "type")?)?,
        });
    }
    Ok((
        local_name(required_string(&fields, "name")?)?,
        classic::Documented::new(
            optional_string(&fields, "doc")?.unwrap_or(""),
            classic::ValueSpecification {
                inputs,
                output: read_type(required_field(&fields, "output")?)?,
            },
        ),
    ))
}

fn write_value_spec(
    name: &classic::Name,
    spec: &classic::Documented<classic::ValueSpecification<classic::Attrs>>,
) -> Element {
    let mut builder = ion_rs::Struct::builder().with_field("name", canonical_name(name));
    if !spec.value.inputs.is_empty() {
        let inputs = spec
            .value
            .inputs
            .iter()
            .map(|input| {
                Element::from(
                    ion_rs::Struct::builder()
                        .with_field("name", canonical_name(&input.name))
                        .with_field("type", write_type(&input.ty))
                        .build(),
                )
            })
            .collect();
        builder = builder.with_field("inputs", list(inputs));
    }
    builder = builder.with_field("output", write_type(&spec.value.output));
    if !spec.doc.is_empty() {
        builder = builder.with_field("doc", spec.doc.as_str());
    }
    Element::from(builder.build()).with_annotations(["public", "spec", "value"])
}

fn items<'a>(
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
) -> Result<Vec<classic::Name>, TransportDiagnostic> {
    items(fields, key)?
        .into_iter()
        .map(|item| {
            local_name(
                item.as_string()
                    .ok_or_else(|| member(format!("{key} holds canonical names")))?,
            )
        })
        .collect()
}

fn names(params: &[classic::Name]) -> ion_rs::List {
    list(
        params
            .iter()
            .map(|name| Element::string(canonical_name(name)))
            .collect(),
    )
}

fn list(elements: Vec<Element>) -> ion_rs::List {
    elements
        .into_iter()
        .fold(ion_rs::Sequence::builder(), |builder, element| {
            builder.push(element)
        })
        .build_list()
}

fn member(message: impl Into<String>) -> TransportDiagnostic {
    IonCodec::error(
        "morphir::ir::ion::invalid_member",
        Stage::Normalization,
        message,
    )
}
