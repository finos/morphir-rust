use crate::{Outcome, error, names};
use morphir_core::{ir::v4::*, naming::FQName};
use morphir_extension_sdk::{
    CompileRequest, Diagnostic, SourceLocation, SourcePosition, SourceRange,
};
use ruff_python_ast::{Expr, Operator, Stmt, StmtClassDef};
use ruff_text_size::TextRange;
use std::collections::{BTreeMap, BTreeSet};

mod functions;
mod imports;
mod project;

#[derive(Clone)]
enum Symbol {
    Type(FQName),
    Function(FQName),
    Callable,
}
type TypeScope = BTreeMap<String, Symbol>;

pub(crate) fn compile(request: &CompileRequest) -> Outcome<(IRFile, Vec<String>)> {
    crate::ir::Version::parse(&request.options.ir_version)?;
    if request.language_id != "python" || !request.dependencies.is_empty() {
        return Err(error("PY001", "Expected Python and no dependencies"));
    }
    for (key, value) in &request.options.extra {
        let valid = match key.as_str() {
            "outputDir" => value.is_string(),
            "emitParseStage" | "emitParseStageFatal" => value.is_boolean(),
            _ => false,
        };
        if !valid {
            return Err(error(
                "PY001",
                format!("Unknown or invalid compile option: {key}"),
            ));
        }
    }
    if parse_stage_requested(request)
        && request
            .options
            .extra
            .get("emitParseStageFatal")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
    {
        return Err(error(
            "PY004",
            "Python parse-stage output is not implemented",
        ));
    }
    project::compile(request)
}

pub(crate) fn parse_stage_requested(request: &CompileRequest) -> bool {
    request
        .options
        .extra
        .get("emitParseStage")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

fn lower(
    statements: &[Stmt],
    types: &TypeScope,
    aliases: Option<&crate::values::TupleAliases>,
    signatures: &crate::values::Signatures,
) -> Outcome<ModuleDefinition> {
    let mut classes = BTreeMap::new();
    let mut sums = BTreeMap::new();
    let mut tuple_aliases = BTreeMap::new();
    let mut declared = BTreeSet::new();
    let mut functions = BTreeMap::new();
    let mut dataclass_imported = false;
    for statement in statements {
        match statement {
            Stmt::ImportFrom(import) => {
                // The package resolver validates all imports and their binding names.
                if import.level == 0
                    && import.module.as_ref().map(|m| m.as_str()) == Some("dataclasses")
                {
                    dataclass_imported = true;
                }
            }
            Stmt::Import(_) => {}
            Stmt::ClassDef(class) => {
                if !dataclass_imported {
                    return Err(error(
                        "PY004",
                        "Import dataclass before declaring a product or variant",
                    ));
                }
                declare(&mut declared, class.name.as_str())?;
                names::type_name(&names::identifier(class.name.as_str())?)?;
                validate_class(class)?;
                classes.insert(class.name.to_string(), class);
            }
            Stmt::TypeAlias(alias) if alias.type_params.is_none() => {
                let name = expr_name(&alias.name)?;
                declare(&mut declared, name)?;
                names::type_name(&names::identifier(name)?)?;
                if matches!(alias.value.as_ref(), Expr::Subscript(subscript) if matches!(subscript.value.as_ref(), Expr::Name(name) if name.id.as_str() == "tuple"))
                {
                    tuple_aliases.insert(name.to_owned(), alias.value.as_ref());
                    continue;
                }
                let mut variants = vec![];
                union_members(&alias.value, &mut variants)?;
                sums.insert(name.to_owned(), variants);
            }
            Stmt::FunctionDef(function) => {
                declare(&mut declared, function.name.as_str())?;
                names::field_name(&names::identifier(function.name.as_str())?)?;
                functions.insert(function.name.to_string(), function);
            }
            _ => {
                return Err(error(
                    "PY004",
                    "Only frozen dataclasses, non-generic sum or tuple aliases and annotated pure functions are supported",
                ));
            }
        }
    }
    let mut owned_variants = BTreeSet::new();
    for variants in sums.values() {
        for variant in variants {
            if !classes.contains_key(*variant) || !owned_variants.insert(*variant) {
                return Err(error(
                    "PY004",
                    format!(
                        "Variant {variant} must name a dataclass and belong to exactly one sum"
                    ),
                ));
            }
        }
    }
    let mut definitions = BTreeMap::new();

    for (name, expression) in &tuple_aliases {
        let canonical = names::identifier(name)?.to_canonical_string();
        let type_expr = annotation(expression, types)?;
        definitions.insert(
            canonical,
            TypeDefinition::TypeAliasDefinition {
                type_params: vec![],
                type_expr,
            },
        );
    }
    for (name, class) in &classes {
        if !owned_variants.contains(name.as_str()) {
            definitions.insert(
                names::identifier(name)?.to_canonical_string(),
                TypeDefinition::TypeAliasDefinition {
                    type_params: vec![],
                    type_expr: Type::Record(Default::default(), fields(class, types)?),
                },
            );
        }
    }
    for (name, variants) in &sums {
        let constructors = variants
            .iter()
            .map(|variant| {
                Ok(ConstructorDefinition {
                    name: names::identifier(variant)?,
                    args: fields(classes[*variant], types)?
                        .into_iter()
                        .map(|field| ConstructorArg {
                            name: field.name,
                            arg_type: field.tpe,
                        })
                        .collect(),
                })
            })
            .collect::<Outcome<Vec<_>>>()?;
        definitions.insert(
            names::identifier(name)?.to_canonical_string(),
            TypeDefinition::CustomTypeDefinition {
                type_params: vec![],
                constructors: public(constructors),
            },
        );
    }
    Ok(ModuleDefinition {
        types: definitions
            .into_iter()
            .map(|(name, definition)| (name, public(Documented::new(None, definition))))
            .collect(),
        values: functions
            .into_iter()
            .filter_map(|(name, function)| aliases.map(|aliases| (name, function, aliases)))
            .map(|(name, function, aliases)| {
                Ok((
                    names::identifier(&name)?.to_canonical_string(),
                    public(Documented::new(
                        None,
                        functions::lower(function, types, aliases, signatures)?,
                    )),
                ))
            })
            .collect::<Outcome<_>>()?,
        doc: None,
    })
}

fn declare(declared: &mut BTreeSet<String>, name: &str) -> Outcome<()> {
    if !declared.insert(names::identifier(name)?.to_canonical_string()) {
        return Err(error(
            "PY003",
            format!("Duplicate declaration after Morphir name normalization: {name}"),
        ));
    }
    Ok(())
}

fn validate_class(class: &StmtClassDef) -> Outcome<()> {
    if class.type_params.is_some() || class.arguments.is_some() || class.decorator_list.len() != 1 {
        return Err(error(
            "PY004",
            "Dataclasses cannot have type parameters, bases, metaclasses or additional decorators",
        ));
    }
    let Expr::Call(call) = &class.decorator_list[0].expression else {
        return Err(error("PY004", "Expected @dataclass(frozen=True)"));
    };
    if expr_name(&call.func)? != "dataclass"
        || !call.arguments.args.is_empty()
        || call.arguments.keywords.len() != 1
        || call.arguments.keywords[0].arg.as_ref().map(|s| s.as_str()) != Some("frozen")
        || !matches!(&call.arguments.keywords[0].value, Expr::BooleanLiteral(v) if v.value)
    {
        return Err(error("PY004", "Only @dataclass(frozen=True) is supported"));
    }
    Ok(())
}

fn fields(class: &StmtClassDef, types: &TypeScope) -> Outcome<Vec<Field>> {
    let mut seen = BTreeSet::new();
    class
        .body
        .iter()
        .filter(|s| !matches!(s, Stmt::Pass(_)))
        .map(|statement| {
            let Stmt::AnnAssign(field) = statement else {
                return Err(error(
                    "PY004",
                    "Dataclass bodies may contain only annotated fields without defaults, or pass",
                ));
            };
            if field.value.is_some() {
                return Err(error("PY004", "Field defaults are not supported"));
            }
            let name = expr_name(&field.target)?;
            declare(&mut seen, name)?;
            let name = names::identifier(name)?;
            names::field_name(&name)?;
            Ok(Field::new(name, annotation(&field.annotation, types)?))
        })
        .collect()
}

fn annotation(expr: &Expr, types: &TypeScope) -> Outcome<Type> {
    match expr {
        Expr::Name(name) => {
            if let Some(Symbol::Type(fq)) = types.get(name.id.as_str()) {
                return Ok(Type::Reference(Default::default(), fq.clone(), vec![]));
            }
            let fq = match name.id.as_str() {
                "int" => "morphir/SDK:basics#int",
                "float" => "morphir/SDK:basics#float",
                "bool" => "morphir/SDK:basics#bool",
                "str" => "morphir/SDK:string#string",
                other => {
                    return Err(error(
                        "PY004",
                        format!("Unknown type or constructor used as a type: {other}"),
                    ));
                }
            };
            Ok(Type::Reference(
                Default::default(),
                FQName::from_canonical_string(fq).map_err(|e| error("PY003", e))?,
                vec![],
            ))
        }
        Expr::Attribute(_) => {
            let name = imports::qualified_name(expr)?;
            let Some(Symbol::Type(fq)) = types.get(&name) else {
                return Err(error("PY004", format!("Unknown imported type: {name}")));
            };
            Ok(Type::Reference(Default::default(), fq.clone(), vec![]))
        }
        Expr::Subscript(subscript)
            if matches!(
                types.get(&imports::qualified_name(&subscript.value)?),
                Some(Symbol::Callable)
            ) =>
        {
            let Expr::Tuple(parts) = subscript.slice.as_ref() else {
                return Err(error("PY004", "Expected Callable[[input], output]"));
            };
            let [Expr::List(inputs), output] = parts.elts.as_slice() else {
                return Err(error("PY004", "Expected Callable[[input], output]"));
            };
            let [input] = inputs.elts.as_slice() else {
                return Err(error(
                    "PY004",
                    "Callable requires exactly one input; nest Callable for curried functions",
                ));
            };
            Ok(Type::Function(
                Default::default(),
                Box::new(annotation(input, types)?),
                Box::new(annotation(output, types)?),
            ))
        }
        Expr::Subscript(subscript) if expr_name(&subscript.value)? == "tuple" => {
            let elements: Vec<&Expr> = match subscript.slice.as_ref() {
                Expr::Tuple(tuple) => tuple.elts.iter().collect(),
                element => vec![element],
            };
            if elements.len() < 2 {
                return Err(error("PY004", "Tuples require at least two fixed elements"));
            }
            Ok(Type::Tuple(
                Default::default(),
                elements
                    .into_iter()
                    .map(|e| annotation(e, types))
                    .collect::<Outcome<_>>()?,
            ))
        }
        _ => Err(error(
            "PY004",
            "Expected int, float, bool, str, a local type name, or a fixed tuple annotation",
        )),
    }
}

fn union_members<'a>(expr: &'a Expr, members: &mut Vec<&'a str>) -> Outcome<()> {
    match expr {
        Expr::BinOp(binary) if binary.op == Operator::BitOr => {
            union_members(&binary.left, members)?;
            union_members(&binary.right, members)
        }
        Expr::Name(name) => {
            members.push(name.id.as_str());
            Ok(())
        }
        _ => Err(error(
            "PY004",
            "A sum alias must list dataclass variants separated by |",
        )),
    }
}

fn expr_name(expr: &Expr) -> Outcome<&str> {
    match expr {
        Expr::Name(name) => Ok(name.id.as_str()),
        _ => Err(error("PY004", "Expected a simple Python name")),
    }
}

fn public<T>(value: T) -> AccessControlled<T> {
    AccessControlled {
        access: Access::Public,
        value,
    }
}

fn at(mut diagnostic: Diagnostic, uri: &str, source: &str, range: TextRange) -> Diagnostic {
    let position = |offset: usize| {
        let prefix = &source[..offset.min(source.len())];
        SourcePosition::from_line_prefix(
            prefix.bytes().filter(|b| *b == b'\n').count() as u32,
            prefix.rsplit('\n').next().unwrap_or(""),
        )
    };
    diagnostic.location = Some(SourceLocation {
        uri: uri.into(),
        range: SourceRange {
            start: position(range.start().to_usize()),
            end: position(range.end().to_usize()),
        },
    });
    diagnostic
}
