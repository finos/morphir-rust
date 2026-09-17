use crate::{Outcome, error, names};
use morphir_core::{ir::v4::*, naming::FQName};
use morphir_extension_sdk::{Artifact, GenerateRequest};
use std::collections::{BTreeMap, BTreeSet};

mod functions;

pub(crate) fn generate(request: &GenerateRequest) -> Outcome<Artifact> {
    if request.target != "python" || !request.options.is_empty() {
        return Err(error(
            "PY001",
            "Expected target python without backend options",
        ));
    }
    let ir: IRFile =
        serde_json::from_value(request.ir.clone()).map_err(|e| error("PY005", e.to_string()))?;
    let Distribution::Library(library) = ir.distribution else {
        return Err(error("PY004", "Only Library distributions are supported"));
    };
    if !library.dependencies.is_empty() || library.def.modules.len() != 1 {
        return Err(error(
            "PY004",
            "Expected exactly one module with no dependencies",
        ));
    }
    let (module_name, module) = library
        .def
        .modules
        .iter()
        .next()
        .expect("one module checked above");
    require_public(module.access)?;
    let module_identifier =
        Name::from_canonical_string(module_name).map_err(|e| error("PY003", e))?;
    let filename = names::module_file_stem(&module_identifier)?;
    if module.value.doc.is_some() {
        return Err(error(
            "PY004",
            "Module documentation is not supported by the Python backend",
        ));
    }
    let mut symbols = BTreeSet::new();
    let mut type_names = BTreeMap::new();
    for name in module.value.types.keys() {
        let python =
            names::type_name(&Name::from_canonical_string(name).map_err(|e| error("PY003", e))?)?;
        reserve(&mut symbols, &python)?;
        type_names.insert(name.clone(), python);
    }
    let mut source =
        String::from("from __future__ import annotations\nfrom dataclasses import dataclass\n\n");
    let mut aliases = vec![];
    let mut tuple_aliases = crate::values::TupleAliases::new();
    for (name, definition) in &module.value.types {
        require_public(definition.access)?;
        if definition.value.doc.is_some() {
            return Err(error("PY004", "Type documentation is not supported yet"));
        }
        let python = &type_names[name];
        match &definition.value.value {
            TypeDefinition::TypeAliasDefinition {
                type_params,
                type_expr: tpe @ Type::Tuple(..),
            } if type_params.is_empty() => {
                aliases.push(format!(
                    "type {python} = {}\n",
                    annotation(tpe, &library.package_name, module_name, &type_names)?
                ));
                tuple_aliases.insert(
                    format!(
                        "{}:{module_name}#{name}",
                        library.package_name.to_canonical_string()
                    ),
                    tpe.clone(),
                );
            }
            TypeDefinition::TypeAliasDefinition {
                type_params,
                type_expr: Type::Record(attrs, fields),
            } if type_params.is_empty() => {
                require_empty_attributes(attrs)?;
                render_class(
                    &mut source,
                    python,
                    fields.iter().map(|f| (&f.name, &f.tpe)),
                    &library.package_name,
                    module_name,
                    &type_names,
                )?;
            }
            TypeDefinition::CustomTypeDefinition {
                type_params,
                constructors,
            } if type_params.is_empty() => {
                require_public(constructors.access)?;
                if constructors.value.is_empty() {
                    return Err(error(
                        "PY004",
                        "Empty custom types cannot be represented by a Python union",
                    ));
                }
                let mut variants = vec![];
                for constructor in &constructors.value {
                    let variant = names::type_name(&constructor.name)?;
                    reserve(&mut symbols, &variant)?;
                    render_class(
                        &mut source,
                        &variant,
                        constructor.args.iter().map(|a| (&a.name, &a.arg_type)),
                        &library.package_name,
                        module_name,
                        &type_names,
                    )?;
                    variants.push(variant);
                }
                aliases.push(format!("type {python} = {}\n", variants.join(" | ")));
            }
            _ => {
                return Err(error(
                    "PY004",
                    "Only non-generic record or tuple aliases and custom types with public constructors are supported",
                ));
            }
        }
    }
    source.push_str(&aliases.join("\n"));
    for alias in tuple_aliases.values() {
        crate::values::resolve_aliases(alias, &tuple_aliases)?;
    }
    for (name, definition) in &module.value.values {
        require_public(definition.access)?;
        if definition.value.doc.is_some() {
            return Err(error(
                "PY004",
                "Function documentation is not supported yet",
            ));
        }
        let name =
            names::field_name(&Name::from_canonical_string(name).map_err(|e| error("PY003", e))?)?;
        reserve(&mut symbols, &name)?;
        source.push_str(&functions::render(
            &name,
            &definition.value.value,
            &library.package_name,
            module_name,
            &type_names,
            &tuple_aliases,
        )?);
    }
    // Ruff validates the constructed declarations and owns AST-to-source rendering.
    // No Python interpreter or user imports run during generation.
    let content = ruff_python_codegen::round_trip(&source)
        .map_err(|e| error("PY005", format!("Generated Python is invalid: {e}")))?;
    Ok(Artifact {
        path: format!("{filename}.py"),
        content: format!("{}\n", content.trim_end()),
        binary: false,
    })
}

fn render_class<'a>(
    source: &mut String,
    name: &str,
    fields: impl Iterator<Item = (&'a Name, &'a Type)>,
    package: &PackageName,
    module: &str,
    types: &BTreeMap<String, String>,
) -> Outcome<()> {
    source.push_str(&format!("@dataclass(frozen=True)\nclass {name}:\n"));
    let mut seen = BTreeSet::new();
    for (name, tpe) in fields {
        let field = names::field_name(name)?;
        reserve(&mut seen, &field)?;
        source.push_str(&format!(
            "    {field}: {}\n",
            annotation(tpe, package, module, types)?
        ));
    }
    if seen.is_empty() {
        source.push_str("    pass\n");
    }
    source.push('\n');
    Ok(())
}

fn annotation(
    tpe: &Type,
    package: &PackageName,
    module: &str,
    types: &BTreeMap<String, String>,
) -> Outcome<String> {
    require_empty_attributes(tpe.attributes())?;
    match tpe {
        Type::Reference(_, fq, args) if args.is_empty() => reference(fq, package, module, types),
        Type::Tuple(_, items) if items.len() >= 2 => Ok(format!(
            "tuple[{}]",
            items
                .iter()
                .map(|t| annotation(t, package, module, types))
                .collect::<Outcome<Vec<_>>>()?
                .join(", ")
        )),
        _ => Err(error(
            "PY004",
            "Unsupported Python field type; expected scalar, local reference, or fixed tuple",
        )),
    }
}

fn reference(
    fq: &FQName,
    package: &PackageName,
    module: &str,
    types: &BTreeMap<String, String>,
) -> Outcome<String> {
    let canonical = fq.to_canonical_string();
    let scalar = match canonical.as_str() {
        "morphir/SDK:basics#int" => Some("int"),
        "morphir/SDK:basics#float" => Some("float"),
        "morphir/SDK:basics#bool" => Some("bool"),
        "morphir/SDK:string#string" => Some("str"),
        _ => None,
    };
    if let Some(scalar) = scalar {
        return Ok(scalar.into());
    }
    let prefix = format!("{}:{module}#", package.to_canonical_string());
    canonical
        .strip_prefix(&prefix)
        .and_then(|local| types.get(local))
        .cloned()
        .ok_or_else(|| {
            error(
                "PY004",
                format!("Unresolved or external type reference: {canonical}"),
            )
        })
}

fn reserve(seen: &mut BTreeSet<String>, name: &str) -> Outcome<()> {
    if !seen.insert(names::identifier(name)?.to_canonical_string()) {
        return Err(error("PY003", format!("Python name collision: {name}")));
    }
    Ok(())
}

fn require_public(access: Access) -> Outcome<()> {
    if access != Access::Public {
        return Err(error(
            "PY004",
            "Private definitions are not supported by this Python subset",
        ));
    }
    Ok(())
}

fn require_empty_attributes(attrs: &TypeAttributes) -> Outcome<()> {
    if *attrs != TypeAttributes::default() {
        return Err(error(
            "PY004",
            "Type attributes cannot yet be preserved in Python",
        ));
    }
    Ok(())
}
