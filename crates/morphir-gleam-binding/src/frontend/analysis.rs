//! Upstream Gleam type analysis before V3 value lowering.

use ecow::EcoString;
use gleam_core::{
    analyse::{ModuleAnalyzerConstructor, TargetSupport},
    ast::{self, BinOp, Publicity, SrcSpan, TypedExpr, TypedModule, TypedPattern, TypedStatement},
    build::{Origin, Outcome, Target},
    config::PackageConfig,
    line_numbers::LineNumbers,
    type_::{
        ModuleInterface, Type as GleamType, TypeConstructor, TypeVar, ValueConstructorVariant,
        build_prelude, generic_var, named,
    },
    uid::UniqueIdGenerator,
    warning::{TypeWarningEmitter, WarningEmitter},
};
use im::HashMap;
use indexmap::IndexMap;
use morphir_core::ir::v4::PackageSpecification;
use morphir_core::ir::v4::{self as v4, AccessControlled, Documented, ValueDefinition};
use morphir_core::{
    ir::v4::{Type as MorphirType, TypeAttributes},
    naming::{FQName, ModuleName, Name, PackageName},
};
use std::collections::{HashMap as StdHashMap, HashSet};
use std::sync::Arc;

pub(crate) fn analyze_module(
    module_name: &str,
    package_name: &str,
    source: &str,
    interfaces: &HashMap<EcoString, ModuleInterface>,
) -> Result<TypedModule, String> {
    let path = format!("{module_name}.gleam");
    let parsed =
        gleam_core::parse::parse_module(path.clone().into(), source, &WarningEmitter::null())
            .map_err(|error| format!("Gleam parse failed: {error:?}"))?;
    let mut module = parsed.module;
    module.name = module_name.into();
    let ids = UniqueIdGenerator::new();
    let mut importable = interfaces.clone();
    importable.insert("gleam".into(), build_prelude(&ids));
    let config = PackageConfig {
        name: package_name.into(),
        ..Default::default()
    };
    let direct_dependencies: StdHashMap<EcoString, ()> = importable
        .values()
        .filter(|interface| !interface.package.is_empty() && interface.package != config.name)
        .map(|interface| (interface.package.clone(), ()))
        .collect();
    let result = ModuleAnalyzerConstructor::<()> {
        target: Target::JavaScript,
        ids: &ids,
        origin: Origin::Src,
        importable_modules: &importable,
        warnings: &TypeWarningEmitter::null(),
        direct_dependencies: &direct_dependencies,
        dev_dependencies: &HashSet::new(),
        target_support: TargetSupport::Enforced,
        package_config: &config,
    }
    .infer_module(module, LineNumbers::new(source), path.into());
    match result {
        Outcome::Ok(module) => Ok(module),
        Outcome::PartialFailure(_, errors) | Outcome::TotalFailure(errors) => {
            Err(format!("Gleam type analysis failed: {errors:?}"))
        }
    }
}

pub(crate) fn sdk_type_interface(
    source_module: &str,
    specification: &PackageSpecification,
) -> Result<ModuleInterface, String> {
    let (morphir_module, morphir_type, gleam_type, required_arity) = match source_module {
        "gleam/dict" => ("dict", "dict", "Dict", 2),
        "gleam/option" => ("maybe", "maybe", "Option", 1),
        "gleam/set" => ("set", "set", "Set", 1),
        _ => return Err(format!("Unsupported Gleam SDK module '{source_module}'")),
    };
    let declaration = specification
        .modules
        .get(morphir_module)
        .and_then(|module| module.types.get(morphir_type))
        .ok_or_else(|| {
            format!("Missing morphir/SDK:{morphir_module}#{morphir_type} type specification")
        })?;
    let parameters = match &declaration.value {
        morphir_core::ir::v4::TypeSpecification::OpaqueTypeSpecification {
            annotations,
            type_params,
        } if annotations.is_empty() => type_params,
        morphir_core::ir::v4::TypeSpecification::CustomTypeSpecification {
            annotations,
            type_params,
            ..
        } if annotations.is_empty() => type_params,
        _ => {
            return Err(format!(
                "SDK type '{morphir_type}' is not a plain opaque or custom specification"
            ));
        }
    };
    if parameters.len() != required_arity {
        return Err(format!(
            "SDK type '{morphir_type}' expects {required_arity} parameters, specification has {}",
            parameters.len()
        ));
    }
    let arguments = (0..required_arity)
        .map(|id| generic_var(id as u64))
        .collect::<Vec<_>>();
    let mut interface = build_prelude(&UniqueIdGenerator::new());
    interface.name = source_module.into();
    interface.package = "gleam_stdlib".into();
    interface.types.clear();
    interface.types_value_constructors.clear();
    interface.values.clear();
    interface.accessors.clear();
    interface.type_aliases.clear();
    interface.types.insert(
        gleam_type.into(),
        TypeConstructor {
            publicity: Publicity::Public,
            origin: SrcSpan::default(),
            module: source_module.into(),
            parameters: arguments.clone(),
            type_: named(
                "gleam_stdlib",
                source_module,
                gleam_type,
                Publicity::Public,
                arguments,
            ),
            deprecation: Default::default(),
            documentation: None,
        },
    );
    Ok(interface)
}

pub(crate) fn morphir_type(
    value: &Arc<GleamType>,
    package: &PackageName,
) -> Result<MorphirType, String> {
    let attrs = TypeAttributes::default();
    match value.as_ref() {
        GleamType::Named {
            package: source_package,
            module,
            name,
            arguments,
            ..
        } => {
            if module == "gleam" && name == "Nil" {
                return Ok(MorphirType::Unit(attrs));
            }
            let (target_package, target_module, target_name) = if module == "gleam" {
                let (module, name) = match name.as_str() {
                    "Int" => ("basics", "int"),
                    "Bool" => ("basics", "bool"),
                    "Float" => ("basics", "float"),
                    "String" => ("string", "string"),
                    "List" => ("list", "list"),
                    "Result" => ("result", "result"),
                    _ => return Err(format!("Unsupported inferred Gleam prelude type '{name}'")),
                };
                (
                    PackageName::parse("morphir/SDK"),
                    module.to_owned(),
                    name.to_owned(),
                )
            } else if source_package == "gleam_stdlib" {
                let (target_module, target_name) = match (module.as_str(), name.as_str()) {
                    ("gleam/dict", "Dict") => ("dict", "dict"),
                    ("gleam/option", "Option") => ("maybe", "maybe"),
                    ("gleam/set", "Set") => ("set", "set"),
                    _ => {
                        return Err(format!(
                            "Unsupported inferred Gleam stdlib type '{module}.{name}'"
                        ));
                    }
                };
                (
                    PackageName::parse("morphir/SDK"),
                    target_module.to_owned(),
                    target_name.to_owned(),
                )
            } else if source_package.as_str() == package.to_string() {
                (package.clone(), module.to_string(), name.to_string())
            } else {
                return Err(format!(
                    "Missing Morphir package mapping for inferred Gleam type '{source_package}:{module}.{name}'"
                ));
            };
            Ok(MorphirType::Reference(
                attrs,
                FQName {
                    package_path: target_package.into(),
                    module_path: ModuleName::parse(&target_module).into(),
                    local_name: Name::from(target_name.as_str()),
                },
                arguments
                    .iter()
                    .map(|argument| morphir_type(argument, package))
                    .collect::<Result<_, _>>()?,
            ))
        }
        GleamType::Fn { arguments, return_ } => {
            let mut result = morphir_type(return_, package)?;
            for argument in arguments.iter().rev() {
                result = MorphirType::Function(
                    attrs.clone(),
                    Box::new(morphir_type(argument, package)?),
                    Box::new(result),
                );
            }
            Ok(result)
        }
        GleamType::Tuple { elements } => Ok(MorphirType::Tuple(
            attrs,
            elements
                .iter()
                .map(|element| morphir_type(element, package))
                .collect::<Result<_, _>>()?,
        )),
        GleamType::Var { type_ } => match &*type_.borrow() {
            TypeVar::Link { type_ } => morphir_type(type_, package),
            TypeVar::Generic { id } => Ok(MorphirType::Variable(
                attrs,
                Name::from(type_var_name(*id).as_str()),
            )),
            TypeVar::Unbound { id } => Err(format!("Unbound inferred Gleam type variable {id}")),
        },
    }
}

fn type_var_name(mut id: u64) -> String {
    let mut letters = String::new();
    loop {
        letters.insert(0, char::from(b'a' + (id % 26) as u8));
        id /= 26;
        if id == 0 {
            break;
        }
    }
    format!("t{letters}")
}

pub(crate) fn lower_typed_functions(
    module: &TypedModule,
    package: &PackageName,
) -> Result<IndexMap<String, AccessControlled<Documented<ValueDefinition>>>, String> {
    module
        .definitions
        .functions
        .iter()
        .map(|function| {
            let source_name = function
                .name
                .as_ref()
                .ok_or("Anonymous top-level function")?
                .1
                .as_str();
            let name = Name::from(source_name).to_string();
            let input_types = function
                .arguments
                .iter()
                .map(|argument| {
                    let name = argument
                        .get_variable_name()
                        .ok_or("Discarded public function parameter is not supported in V3")?;
                    Ok((
                        Name::from(name.as_str()).to_string(),
                        morphir_type(&argument.type_, package)?,
                    ))
                })
                .collect::<Result<IndexMap<_, _>, String>>()?;
            let body = lower_statements(&function.body, package, &module.name)?;
            let definition = ValueDefinition {
                input_types,
                output_type: Some(morphir_type(&function.return_type, package)?),
                body: v4::ValueBody::Expression(body),
            };
            let access = if function.publicity.is_importable() {
                v4::Access::Public
            } else {
                v4::Access::Private
            };
            Ok((
                name,
                AccessControlled {
                    access,
                    value: Documented::new(None, definition),
                },
            ))
        })
        .collect()
}

fn attrs(ty: &Arc<GleamType>, package: &PackageName) -> Result<v4::ValueAttributes, String> {
    Ok(v4::ValueAttributes {
        inferred_type: Some(Box::new(morphir_type(ty, package)?)),
        ..Default::default()
    })
}

fn local_name(package: &PackageName, module: &str, name: &str) -> FQName {
    FQName {
        package_path: package.clone().into(),
        module_path: ModuleName::parse(module).into(),
        local_name: Name::from(name),
    }
}

fn sdk_name(module: &str, name: &str) -> FQName {
    local_name(&PackageName::parse("morphir/SDK"), module, name)
}

fn lower_statements(
    statements: &[TypedStatement],
    package: &PackageName,
    module: &str,
) -> Result<v4::Value, String> {
    match statements {
        [ast::Statement::Expression(expr)] => lower_expr(expr, package, module),
        _ => Err("V3 pilot lowering requires one expression in this function body".into()),
    }
}

fn lower_pattern(
    pattern: &TypedPattern,
    package: &PackageName,
    module: &str,
) -> Result<v4::Pattern, String> {
    let attributes = attrs(&pattern.type_(), package)?;
    Ok(match pattern {
        ast::Pattern::Discard { .. } => v4::Pattern::WildcardPattern(attributes),
        ast::Pattern::Variable { name, .. } => v4::Pattern::AsPattern(
            attributes.clone(),
            Box::new(v4::Pattern::WildcardPattern(attributes)),
            Name::from(name.as_str()),
        ),
        ast::Pattern::Assign { pattern, name, .. } => v4::Pattern::AsPattern(
            attributes,
            Box::new(lower_pattern(pattern, package, module)?),
            Name::from(name.as_str()),
        ),
        ast::Pattern::List { elements, tail, .. } => {
            let tail = match tail {
                Some(tail) => lower_pattern(&tail.pattern, package, module)?,
                None => v4::Pattern::EmptyListPattern(attributes.clone()),
            };
            elements.iter().rev().try_fold(tail, |tail, head| {
                Ok::<_, String>(v4::Pattern::HeadTailPattern(
                    attributes.clone(),
                    Box::new(lower_pattern(head, package, module)?),
                    Box::new(tail),
                ))
            })?
        }
        ast::Pattern::Constructor {
            name,
            constructor,
            arguments,
            ..
        } => {
            if name == "True" || name == "False" {
                v4::Pattern::LiteralPattern(attributes, v4::Literal::Bool(name == "True"))
            } else {
                let source_module = match constructor {
                    gleam_core::analyse::Inferred::Known(known) => known.module.as_str(),
                    gleam_core::analyse::Inferred::Unknown => {
                        return Err(format!("Unresolved constructor pattern '{name}'"));
                    }
                };
                if source_module != module {
                    return Err(format!(
                        "External constructor pattern '{source_module}.{name}' is not supported"
                    ));
                }
                v4::Pattern::ConstructorPattern(
                    attributes,
                    local_name(package, source_module, name),
                    arguments
                        .iter()
                        .map(|argument| lower_pattern(&argument.value, package, module))
                        .collect::<Result<_, _>>()?,
                )
            }
        }
        _ => return Err("Unsupported typed Gleam pattern in V3 lowering".into()),
    })
}

fn lower_expr(expr: &TypedExpr, package: &PackageName, module: &str) -> Result<v4::Value, String> {
    let attributes = attrs(&expr.type_(), package)?;
    Ok(match expr {
        TypedExpr::Int { int_value, .. } => v4::Value::Literal(
            attributes,
            v4::Literal::Integer(
                int_value
                    .to_string()
                    .parse()
                    .map_err(|_| "Cannot convert inferred Gleam integer")?,
            ),
        ),
        TypedExpr::String { value, .. } => {
            v4::Value::Literal(attributes, v4::Literal::String(value.to_string()))
        }
        TypedExpr::Var {
            constructor, name, ..
        } => match &constructor.variant {
            ValueConstructorVariant::LocalVariable { .. } => {
                v4::Value::Variable(attributes, Name::from(name.as_str()))
            }
            ValueConstructorVariant::ModuleFn {
                module: source_module,
                ..
            } => {
                if source_module.as_str() != module {
                    return Err(format!(
                        "External function '{source_module}.{name}' is not supported"
                    ));
                }
                v4::Value::Reference(attributes, local_name(package, source_module, name))
            }
            ValueConstructorVariant::Record {
                module: source_module,
                ..
            } if source_module == "gleam" && (name == "True" || name == "False") => {
                v4::Value::Literal(attributes, v4::Literal::Bool(name == "True"))
            }
            ValueConstructorVariant::Record {
                module: source_module,
                ..
            } => {
                if source_module.as_str() != module {
                    return Err(format!(
                        "External constructor '{source_module}.{name}' is not supported"
                    ));
                }
                v4::Value::Constructor(attributes, local_name(package, source_module, name))
            }
            _ => return Err(format!("Unsupported typed Gleam variable '{name}'")),
        },
        TypedExpr::Call {
            fun,
            arguments,
            type_,
            ..
        } => {
            let mut result = lower_expr(fun, package, module)?;
            let function_type = fun.type_();
            let GleamType::Fn {
                arguments: parameter_types,
                return_,
            } = function_type.as_ref()
            else {
                return Err("Typed Gleam call has no function type".into());
            };
            for (index, argument) in arguments.iter().enumerate() {
                let result_type = if index + 1 == arguments.len() {
                    morphir_type(type_, package)?
                } else {
                    parameter_types
                        .get(index + 1..)
                        .ok_or("Call arity exceeds inferred function type")?
                        .iter()
                        .rev()
                        .try_fold(morphir_type(return_, package)?, |result, argument| {
                            Ok::<_, String>(MorphirType::Function(
                                TypeAttributes::default(),
                                Box::new(morphir_type(argument, package)?),
                                Box::new(result),
                            ))
                        })?
                };
                result = v4::Value::Apply(
                    v4::ValueAttributes {
                        inferred_type: Some(Box::new(result_type)),
                        ..Default::default()
                    },
                    Box::new(result),
                    Box::new(lower_expr(&argument.value, package, module)?),
                );
            }
            result
        }
        TypedExpr::BinOp {
            operator,
            left,
            right,
            ..
        } => {
            let name = match operator {
                BinOp::AddInt => "add",
                BinOp::Eq => "equal",
                _ => return Err(format!("Unsupported Gleam operator '{operator:?}'")),
            };
            let left_type = morphir_type(&left.type_(), package)?;
            let right_type = morphir_type(&right.type_(), package)?;
            let result_type = morphir_type(&expr.type_(), package)?;
            let after_left = MorphirType::Function(
                TypeAttributes::default(),
                Box::new(right_type.clone()),
                Box::new(result_type.clone()),
            );
            let function_type = MorphirType::Function(
                TypeAttributes::default(),
                Box::new(left_type),
                Box::new(after_left.clone()),
            );
            let typed_attrs = |ty| v4::ValueAttributes {
                inferred_type: Some(Box::new(ty)),
                ..Default::default()
            };
            let function =
                v4::Value::Reference(typed_attrs(function_type), sdk_name("basics", name));
            let first = v4::Value::Apply(
                typed_attrs(after_left),
                Box::new(function),
                Box::new(lower_expr(left, package, module)?),
            );
            v4::Value::Apply(
                attributes,
                Box::new(first),
                Box::new(lower_expr(right, package, module)?),
            )
        }
        TypedExpr::Case {
            subjects, clauses, ..
        } => {
            let [subject] = subjects.as_slice() else {
                return Err("V3 pilot supports one case subject".into());
            };
            let cases = clauses
                .iter()
                .map(|clause| -> Result<v4::PatternCase, String> {
                    if clause.guard.is_some() || !clause.alternative_patterns.is_empty() {
                        return Err("Guarded or alternative case clauses are not supported".into());
                    }
                    let [pattern] = clause.pattern.as_slice() else {
                        return Err("V3 pilot supports one case pattern".into());
                    };
                    Ok(v4::PatternCase(
                        lower_pattern(pattern, package, module)?,
                        lower_expr(&clause.then, package, module)?,
                    ))
                })
                .collect::<Result<_, _>>()?;
            v4::Value::PatternMatch(
                attributes,
                Box::new(lower_expr(subject, package, module)?),
                cases,
            )
        }
        TypedExpr::Block { statements, .. } => lower_statements(statements, package, module)?,
        _ => return Err("Unsupported typed Gleam expression in V3 lowering".into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gleam_core::type_::int;
    use morphir_core::{
        ir::v4::{Documented, ModuleSpecification, TypeSpecification},
        naming::Name,
    };

    fn sdk_spec(module: &str, name: &str, arity: usize) -> PackageSpecification {
        PackageSpecification {
            modules: [(
                module.into(),
                ModuleSpecification {
                    annotations: vec![],
                    types: [(
                        name.into(),
                        Documented::new(
                            None,
                            TypeSpecification::OpaqueTypeSpecification {
                                annotations: vec![],
                                type_params: (0..arity)
                                    .map(|i| Name::from(format!("t{i}").as_str()))
                                    .collect(),
                            },
                        ),
                    )]
                    .into(),
                    values: Default::default(),
                    doc: None,
                },
            )]
            .into(),
        }
    }

    #[test]
    fn sdk_dict_interface_uses_supplied_two_parameter_specification() {
        let interface = sdk_type_interface("gleam/dict", &sdk_spec("dict", "dict", 2))
            .expect("explicit SDK Dict type");
        assert_eq!(interface.name, "gleam/dict");
        assert_eq!(interface.types["Dict"].parameters.len(), 2);
        let source = "import gleam/dict.{type Dict}\npub fn id(value: Dict(Int, String)) -> Dict(Int, String) { value }";
        let analyzed = analyze_module(
            "validation/dict",
            "example/arity",
            source,
            &vec![("gleam/dict".into(), interface)].into(),
        )
        .expect("analyze imported explicit SDK type");
        assert_eq!(analyzed.definitions.functions.len(), 1);
    }

    #[test]
    fn sdk_dict_interface_rejects_wrong_arity_and_missing_type() {
        assert!(sdk_type_interface("gleam/dict", &sdk_spec("dict", "dict", 1)).is_err());
        assert!(sdk_type_interface("gleam/dict", &sdk_spec("dict", "other", 2)).is_err());
    }

    #[test]
    fn sdk_option_custom_type_can_be_imported_for_type_only_use() {
        let mut specification = sdk_spec("maybe", "maybe", 1);
        specification
            .modules
            .get_mut("maybe")
            .unwrap()
            .types
            .get_mut("maybe")
            .unwrap()
            .value = TypeSpecification::CustomTypeSpecification {
            annotations: vec![],
            type_params: vec![Name::from("a")],
            constructors: vec![],
        };
        let interface = sdk_type_interface("gleam/option", &specification).unwrap();
        assert_eq!(interface.types["Option"].parameters.len(), 1);
    }

    #[test]
    fn local_module_interface_preserves_imported_constructor_type() {
        let model = analyze_module(
            "morphir/ir/type_",
            "morphir_ir_specification",
            "pub type Type(a) { Unit(a) }",
            &HashMap::new(),
        )
        .unwrap();
        let interfaces: HashMap<_, _> = vec![("morphir/ir/type_".into(), model.type_info)].into();
        let rule = analyze_module(
            "morphir/validation/arity",
            "morphir_ir_specification",
            "import morphir/ir/type_.{type Type}\npub fn check_arity(values: List(Type(Nil))) -> Int { 0 }",
            &interfaces,
        ).unwrap();
        assert_eq!(rule.definitions.functions.len(), 1);
        assert_eq!(rule.definitions.functions[0].return_type, int());
    }

    #[test]
    fn inferred_prelude_int_is_morphir_sdk_int() {
        let actual = morphir_type(&int(), &PackageName::parse("example/arity")).unwrap();
        assert_eq!(
            actual,
            MorphirType::Reference(
                TypeAttributes::default(),
                FQName {
                    package_path: PackageName::parse("morphir/SDK").into(),
                    module_path: ModuleName::parse("basics").into(),
                    local_name: Name::from("int"),
                },
                vec![],
            )
        );
    }

    #[test]
    fn typed_recursive_count_lowers_to_v4_pattern_match_with_inferred_type() {
        let source = "pub fn count(items: List(Int)) -> Int { case items { [] -> 0 [_, ..rest] -> 1 + count(rest) } }";
        let typed =
            analyze_module("validation/arity", "example/arity", source, &HashMap::new()).unwrap();
        let values = lower_typed_functions(&typed, &PackageName::parse("example/arity")).unwrap();
        let definition = &values["count"].value.value;
        assert_eq!(
            definition.output_type,
            Some(morphir_type(&int(), &PackageName::parse("example/arity")).unwrap())
        );
        let morphir_core::ir::v4::ValueBody::Expression(morphir_core::ir::v4::Value::PatternMatch(
            attrs,
            _,
            cases,
        )) = &definition.body
        else {
            panic!("compiled count body must be a case expression")
        };
        assert_eq!(
            attrs.inferred_type.as_deref(),
            definition.output_type.as_ref()
        );
        assert_eq!(cases.len(), 2);
    }

    #[test]
    fn infers_recursive_function_with_real_integer_type() {
        let source = "pub fn count(items: List(Int)) -> Int { case items { [] -> 0 [_, ..rest] -> 1 + count(rest) } }";
        let analyzed = analyze_module("validation/arity", "example/arity", source, &HashMap::new())
            .expect("analyze recursive function");
        assert_eq!(analyzed.definitions.functions.len(), 1);
        assert_eq!(analyzed.definitions.functions[0].return_type, int());
    }

    #[test]
    fn missing_import_is_an_error() {
        let source =
            "import missing/module.{type Thing}\npub fn id(value: Thing) -> Thing { value }";
        assert!(
            analyze_module("validation/arity", "example/arity", source, &HashMap::new()).is_err()
        );
    }
}
