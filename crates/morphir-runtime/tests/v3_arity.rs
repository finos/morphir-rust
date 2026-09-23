use morphir_core::ir::classic as ir;
use morphir_runtime::{EvaluationLimits, RuntimeValue, evaluate_v3, evaluate_v3_with_deadline};
use std::time::{Duration, Instant};

type Expr = ir::Value<ir::Attrs, ir::Type<ir::Attrs>>;

fn name(text: &str) -> ir::Name {
    ir::Name::from_str(text)
}
fn path(text: &str) -> ir::Path {
    ir::Path::new(text.split('/').map(name).collect())
}
fn reference(module: &str, local: &str) -> ir::FQName {
    ir::FQName::new(path("example/arity"), path(module), name(local))
}
fn attr() -> ir::Type<ir::Attrs> {
    ir::Type::Unit(ir::Attrs::None)
}
fn literal(number: i64) -> Expr {
    ir::Value::Literal(attr(), ir::Literal::WholeNumber(number))
}
fn variable(local: &str) -> Expr {
    ir::Value::Variable(attr(), name(local))
}
fn value_ref(module: &str, local: &str) -> Expr {
    ir::Value::Reference(attr(), reference(module, local))
}
fn apply(function: Expr, argument: Expr) -> Expr {
    ir::Value::Apply(attr(), Box::new(function), Box::new(argument))
}
fn sdk(local: &str) -> Expr {
    ir::Value::Reference(
        attr(),
        ir::FQName::new(path("morphir/SDK"), path("basics"), name(local)),
    )
}
fn function(arguments: &[&str], body: Expr) -> ir::ValueDefinition<ir::Attrs, ir::Type<ir::Attrs>> {
    ir::ValueDefinition {
        input_types: arguments
            .iter()
            .map(|argument| ir::value::ValueArgument {
                name: name(argument),
                annotation: attr(),
                ty: attr(),
            })
            .collect(),
        output_type: attr(),
        body,
    }
}
fn defined(
    local: &str,
    arguments: &[&str],
    body: Expr,
) -> ir::module::ModuleValueDefinition<ir::Attrs, ir::Type<ir::Attrs>> {
    (
        name(local),
        ir::AccessControlled {
            access: ir::Access::Public,
            value: ir::Documented::new("", function(arguments, body)),
        },
    )
}

fn independent_arity_program(with_sdk: bool) -> ir::Distribution {
    let count = ir::Value::PatternMatch(
        attr(),
        Box::new(variable("arguments")),
        vec![
            (ir::Pattern::EmptyList(attr()), literal(0)),
            (
                ir::Pattern::HeadTail(
                    attr(),
                    Box::new(ir::Pattern::Wildcard(attr())),
                    Box::new(ir::Pattern::As(
                        attr(),
                        Box::new(ir::Pattern::Wildcard(attr())),
                        name("rest"),
                    )),
                ),
                apply(
                    apply(sdk("add"), literal(1)),
                    apply(value_ref("rules", "count"), variable("rest")),
                ),
            ),
        ],
    );
    let compare = ir::Value::PatternMatch(
        attr(),
        Box::new(apply(
            apply(sdk("equal"), variable("expected")),
            variable("actual"),
        )),
        vec![
            (
                ir::Pattern::Literal(attr(), ir::Literal::Bool(true)),
                ir::Value::Constructor(attr(), reference("rules", "valid")),
            ),
            (
                ir::Pattern::Literal(attr(), ir::Literal::Bool(false)),
                apply(
                    apply(
                        ir::Value::Constructor(attr(), reference("rules", "mismatch")),
                        variable("expected"),
                    ),
                    variable("actual"),
                ),
            ),
        ],
    );
    let check = apply(
        apply(value_ref("rules", "compare-count"), variable("expected")),
        apply(value_ref("rules", "count"), variable("arguments")),
    );
    let rule_type = (
        name("arity-outcome"),
        ir::AccessControlled {
            access: ir::Access::Public,
            value: ir::Documented::new(
                "",
                ir::TypeDefinition::Custom(
                    vec![],
                    ir::AccessControlled {
                        access: ir::Access::Public,
                        value: vec![
                            ir::Constructor {
                                name: name("valid"),
                                args: vec![],
                            },
                            ir::Constructor {
                                name: name("mismatch"),
                                args: vec![(name("expected"), attr()), (name("actual"), attr())],
                            },
                        ],
                    },
                ),
            ),
        },
    );
    ir::Distribution {
        format_version: 3,
        distribution: ir::DistributionBody::Library(
            path("example/arity"),
            if with_sdk {
                vec![(
                    path("morphir/SDK"),
                    ir::PackageSpecification { modules: vec![] },
                )]
            } else {
                vec![]
            },
            ir::PackageDefinition {
                modules: vec![ir::ModuleEntry {
                    path: path("rules"),
                    definition: ir::AccessControlled {
                        access: ir::Access::Public,
                        value: ir::ModuleDefinition {
                            types: vec![rule_type],
                            values: vec![
                                defined("count", &["arguments"], count),
                                defined("compare-count", &["expected", "actual"], compare),
                                defined("check-arity", &["expected", "arguments"], check),
                            ],
                            doc: None,
                        },
                    },
                }],
            },
        ),
    }
}

#[test]
fn independent_v3_arity_vectors_cover_all_five_fixed_outcomes() {
    let program = independent_arity_program(true);
    let valid = RuntimeValue::Constructor(reference("rules", "valid"), vec![]);
    for (expected, supplied, outcome) in [
        (0, 0, valid.clone()),
        (1, 1, valid.clone()),
        (2, 2, valid),
        (
            1,
            0,
            RuntimeValue::Constructor(
                reference("rules", "mismatch"),
                vec![RuntimeValue::Integer(1), RuntimeValue::Integer(0)],
            ),
        ),
        (
            1,
            2,
            RuntimeValue::Constructor(
                reference("rules", "mismatch"),
                vec![RuntimeValue::Integer(1), RuntimeValue::Integer(2)],
            ),
        ),
    ] {
        let units = RuntimeValue::List(
            (0..supplied)
                .map(|_| {
                    RuntimeValue::Constructor(
                        reference("morphir/ir/type", "unit"),
                        vec![RuntimeValue::Unit],
                    )
                })
                .collect(),
        );
        let actual = evaluate_v3(
            &program,
            &reference("rules", "check-arity"),
            vec![RuntimeValue::Integer(expected), units],
            EvaluationLimits::default(),
        )
        .unwrap();
        assert_eq!(actual, outcome, "expected={expected}, supplied={supplied}");
    }
}

#[test]
fn missing_sdk_dependency_and_exhausted_budget_are_distinct_failures() {
    let input = vec![
        RuntimeValue::Integer(1),
        RuntimeValue::List(vec![RuntimeValue::Unit]),
    ];
    let missing = evaluate_v3(
        &independent_arity_program(false),
        &reference("rules", "check-arity"),
        input.clone(),
        EvaluationLimits::default(),
    );
    assert_eq!(
        missing,
        Err(morphir_runtime::EvaluationError::MissingDependency(path(
            "morphir/SDK"
        )))
    );
    let exhausted = evaluate_v3(
        &independent_arity_program(true),
        &reference("rules", "check-arity"),
        input,
        EvaluationLimits {
            fuel: 1,
            max_call_depth: 256,
        },
    );
    assert_eq!(
        exhausted,
        Err(morphir_runtime::EvaluationError::FuelExhausted)
    );
    let depth_limited = evaluate_v3(
        &independent_arity_program(true),
        &reference("rules", "check-arity"),
        vec![
            RuntimeValue::Integer(2),
            RuntimeValue::List(vec![RuntimeValue::Unit, RuntimeValue::Unit]),
        ],
        EvaluationLimits {
            fuel: 10_000,
            max_call_depth: 1,
        },
    );
    assert_eq!(
        depth_limited,
        Err(morphir_runtime::EvaluationError::CallDepthExceeded)
    );
}

#[test]
fn independent_literal_program_evaluates_without_a_compiler() {
    let distribution = ir::Distribution {
        format_version: 3,
        distribution: ir::DistributionBody::Library(
            path("example/arity"),
            vec![],
            ir::PackageDefinition {
                modules: vec![ir::ModuleEntry {
                    path: path("rules"),
                    definition: ir::AccessControlled {
                        access: ir::Access::Public,
                        value: ir::ModuleDefinition {
                            types: vec![],
                            values: vec![(
                                name("answer"),
                                ir::AccessControlled {
                                    access: ir::Access::Public,
                                    value: ir::Documented::new(
                                        "",
                                        ir::ValueDefinition {
                                            input_types: vec![],
                                            output_type: attr(),
                                            body: literal(42),
                                        },
                                    ),
                                },
                            )],
                            doc: None,
                        },
                    },
                }],
            },
        ),
    };
    let actual = evaluate_v3(
        &distribution,
        &reference("rules", "answer"),
        vec![],
        EvaluationLimits::default(),
    )
    .unwrap();
    assert_eq!(actual, RuntimeValue::Integer(42));
}

#[test]
fn zero_argument_definition_reference_evaluates_the_named_value() {
    let program = ir::Distribution {
        format_version: 3,
        distribution: ir::DistributionBody::Library(
            path("example/arity"),
            vec![],
            ir::PackageDefinition {
                modules: vec![ir::ModuleEntry {
                    path: path("rules"),
                    definition: ir::AccessControlled {
                        access: ir::Access::Public,
                        value: ir::ModuleDefinition {
                            types: vec![],
                            values: vec![
                                defined("answer", &[], literal(42)),
                                defined("proxy", &[], value_ref("rules", "answer")),
                            ],
                            doc: None,
                        },
                    },
                }],
            },
        ),
    };
    assert_eq!(
        evaluate_v3(
            &program,
            &reference("rules", "proxy"),
            vec![],
            EvaluationLimits::default()
        ),
        Ok(RuntimeValue::Integer(42)),
    );
}

#[test]
fn lambda_application_obeys_call_depth_limit() {
    let lambda = ir::Value::Lambda(
        attr(),
        ir::Pattern::As(
            attr(),
            Box::new(ir::Pattern::Wildcard(attr())),
            name("value"),
        ),
        Box::new(variable("value")),
    );
    let program = ir::Distribution {
        format_version: 3,
        distribution: ir::DistributionBody::Library(
            path("example/arity"),
            vec![],
            ir::PackageDefinition {
                modules: vec![ir::ModuleEntry {
                    path: path("rules"),
                    definition: ir::AccessControlled {
                        access: ir::Access::Public,
                        value: ir::ModuleDefinition {
                            types: vec![],
                            values: vec![defined("apply-lambda", &[], apply(lambda, literal(3)))],
                            doc: None,
                        },
                    },
                }],
            },
        ),
    };
    assert_eq!(
        evaluate_v3(
            &program,
            &reference("rules", "apply-lambda"),
            vec![],
            EvaluationLimits {
                fuel: 100,
                max_call_depth: 1
            }
        ),
        Err(morphir_runtime::EvaluationError::CallDepthExceeded),
    );
}

#[test]
fn lexical_definition_and_recursive_local_function_are_evaluated() {
    let local_count = ir::Value::PatternMatch(
        attr(),
        Box::new(variable("items")),
        vec![
            (ir::Pattern::EmptyList(attr()), literal(0)),
            (
                ir::Pattern::HeadTail(
                    attr(),
                    Box::new(ir::Pattern::Wildcard(attr())),
                    Box::new(ir::Pattern::As(
                        attr(),
                        Box::new(ir::Pattern::Wildcard(attr())),
                        name("rest"),
                    )),
                ),
                apply(
                    apply(sdk("add"), literal(1)),
                    apply(variable("count"), variable("rest")),
                ),
            ),
        ],
    );
    let body = ir::Value::LetDefinition(
        attr(),
        name("offset"),
        Box::new(function(&[], literal(1))),
        Box::new(ir::Value::LetRecursion(
            attr(),
            vec![(name("count"), Box::new(function(&["items"], local_count)))],
            Box::new(apply(
                apply(sdk("add"), variable("offset")),
                apply(variable("count"), variable("values")),
            )),
        )),
    );
    let program = ir::Distribution {
        format_version: 3,
        distribution: ir::DistributionBody::Library(
            path("example/arity"),
            vec![(
                path("morphir/SDK"),
                ir::PackageSpecification { modules: vec![] },
            )],
            ir::PackageDefinition {
                modules: vec![ir::ModuleEntry {
                    path: path("rules"),
                    definition: ir::AccessControlled {
                        access: ir::Access::Public,
                        value: ir::ModuleDefinition {
                            types: vec![],
                            values: vec![defined("local-count", &["values"], body)],
                            doc: None,
                        },
                    },
                }],
            },
        ),
    };
    let actual = evaluate_v3(
        &program,
        &reference("rules", "local-count"),
        vec![RuntimeValue::List(vec![
            RuntimeValue::Unit,
            RuntimeValue::Unit,
        ])],
        EvaluationLimits::default(),
    );
    assert_eq!(actual, Ok(RuntimeValue::Integer(3)));
}

#[test]
fn expired_host_deadline_stops_before_reducing_the_rule() {
    let actual = evaluate_v3_with_deadline(
        &independent_arity_program(true),
        &reference("rules", "check-arity"),
        vec![RuntimeValue::Integer(0), RuntimeValue::List(vec![])],
        EvaluationLimits::default(),
        Some(Instant::now() - Duration::from_millis(1)),
    );
    assert_eq!(
        actual,
        Err(morphir_runtime::EvaluationError::DeadlineExceeded)
    );
}
