use indexmap::IndexMap;
use morphir_core::ir::v4::*;
use morphir_core::naming::{FQName, Name};
use morphir_extension_sdk::prelude::*;
use morphir_gleam_binding::{GleamExtension, frontend::parse_gleam};
use std::collections::HashMap;

fn integer(n: i64) -> Value {
    Value::Literal(Default::default(), Literal::integer(n))
}
fn variable(name: &str) -> Value {
    Value::Variable(Default::default(), Name::from(name))
}
fn binding(name: &str) -> Pattern {
    Pattern::AsPattern(
        Default::default(),
        Box::new(Pattern::WildcardPattern(Default::default())),
        Name::from(name),
    )
}
fn destructure() -> Value {
    Value::Destructure(
        Default::default(),
        Pattern::TuplePattern(Default::default(), vec![binding("left"), binding("right")]),
        Box::new(Value::Tuple(
            Default::default(),
            vec![integer(1), integer(2)],
        )),
        Box::new(variable("left")),
    )
}
fn cons(head: Value, tail: Value) -> Value {
    let function = Value::Reference(
        Default::default(),
        FQName::new(
            morphir_core::naming::Path::new("morphir/SDK"),
            morphir_core::naming::Path::new("list"),
            Name::from("cons"),
        ),
    );
    Value::Apply(
        Default::default(),
        Box::new(Value::Apply(
            Default::default(),
            Box::new(function),
            Box::new(head),
        )),
        Box::new(tail),
    )
}
fn generate_body(body: ValueBody) -> GenerateResult {
    let package = PackageDefinition {
        modules: IndexMap::from([(
            "main".into(),
            AccessControlled {
                access: Access::Public,
                value: ModuleDefinition {
                    types: IndexMap::new(),
                    doc: None,
                    values: IndexMap::from([(
                        "run".into(),
                        AccessControlled {
                            access: Access::Public,
                            value: Documented::new(
                                None,
                                ValueDefinition {
                                    input_types: IndexMap::new(),
                                    output_type: Some(Type::Unit(Default::default())),
                                    body,
                                },
                            ),
                        },
                    )]),
                },
            },
        )]),
    };
    GleamExtension
        .generate(GenerateRequest {
            ir: serde_json::to_value(package).unwrap(),
            target: "gleam".into(),
            options: HashMap::new(),
        })
        .unwrap()
}
fn generated(value: Value) -> String {
    let result = generate_body(ValueBody::Expression(value));
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.artifacts.len(), 1);
    let code = result.artifacts[0].content.clone();
    parse_gleam("main.gleam", &code).expect("generated module must parse with the official parser");
    code
}
fn local_value() -> Value {
    Value::LetDefinition(
        Default::default(),
        Name::from("inner"),
        Box::new(ValueDefinition {
            input_types: IndexMap::new(),
            output_type: Some(Type::Reference(
                Default::default(),
                FQName::new(
                    morphir_core::naming::Path::new("morphir/SDK"),
                    morphir_core::naming::Path::new("basics"),
                    Name::from("int"),
                ),
                vec![],
            )),
            body: ValueBody::Expression(integer(42)),
        }),
        Box::new(variable("inner")),
    )
}

fn nested_local_values() -> Value {
    Value::Tuple(
        Default::default(),
        vec![
            local_value(),
            Value::Destructure(
                Default::default(),
                binding("outer"),
                Box::new(local_value()),
                Box::new(variable("outer")),
            ),
        ],
    )
}

#[test]
fn local_value_definitions_remain_expressions_when_nested() {
    assert_eq!(
        generated(nested_local_values()),
        include_str!("goldens/nested_local_values.gleam")
    );
}

fn representative() -> Value {
    Value::Tuple(
        Default::default(),
        vec![
            destructure(),
            cons(
                integer(1),
                Value::List(Default::default(), vec![integer(2), integer(3)]),
            ),
            Value::IfThenElse(
                Default::default(),
                Box::new(Value::Literal(Default::default(), Literal::Bool(true))),
                Box::new(destructure()),
                Box::new(integer(0)),
            ),
            Value::List(Default::default(), vec![destructure()]),
            cons(destructure(), Value::List(Default::default(), vec![])),
            Value::PatternMatch(
                Default::default(),
                Box::new(Value::Unit(Default::default())),
                vec![PatternCase(
                    Pattern::UnitPattern(Default::default()),
                    destructure(),
                )],
            ),
        ],
    )
}
#[test]
fn structural_values_match_the_complete_golden_and_parse() {
    assert_eq!(
        generated(representative()),
        include_str!("goldens/structural_values.gleam")
    );
}
#[test]
fn unsupported_values_fail_without_artifacts() {
    for (name, value) in [
        (
            "FieldFunction",
            Value::FieldFunction(Default::default(), Name::from("field")),
        ),
        (
            "LetRecursion",
            Value::LetRecursion(Default::default(), vec![], Box::new(integer(0))),
        ),
        (
            "UpdateRecord",
            Value::UpdateRecord(Default::default(), Box::new(variable("record")), vec![]),
        ),
        (
            "Hole",
            Value::Hole(
                Default::default(),
                HoleReason::DeletedDuringRefactor {
                    tx_id: "test".into(),
                },
                None,
            ),
        ),
        ("Record", Value::Record(Default::default(), vec![])),
    ] {
        let result = generate_body(ValueBody::Expression(Value::Tuple(
            Default::default(),
            vec![value],
        )));
        assert!(
            !result.success,
            "{name} must reject instead of generating a placeholder"
        );
        assert!(result.artifacts.is_empty());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains(name)),
            "{:?}",
            result.diagnostics
        );
    }
}
#[test]
fn unsupported_definition_bodies_fail_without_artifacts() {
    for (name, body) in [
        (
            "Native",
            ValueBody::Native {
                native_info: NativeInfo {
                    hint: NativeHint::Arithmetic,
                    description: None,
                },
            },
        ),
        (
            "External",
            ValueBody::External {
                externals: vec![ExternalBinding {
                    target_platform: "javascript".into(),
                    external_name: "run".into(),
                }],
                fallback: None,
            },
        ),
        (
            "Incomplete",
            ValueBody::Incomplete {
                incompleteness: Incompleteness::Draft,
                partial_body: None,
            },
        ),
    ] {
        let result = generate_body(body);
        assert!(!result.success, "{name} must reject");
        assert!(result.artifacts.is_empty());
        assert!(
            result.diagnostics.iter().any(|d| d.message.contains(name)),
            "{:?}",
            result.diagnostics
        );
    }
}
#[test]
#[ignore = "requires a locally installed Gleam compiler"]
fn structural_values_pass_the_gleam_compiler() {
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join("src")).unwrap();
    std::fs::write(
        project.path().join("gleam.toml"),
        "name = \"backend_values\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(
        project.path().join("src/main.gleam"),
        generated(representative()),
    )
    .unwrap();
    std::fs::write(
        project.path().join("src/refutable.gleam"),
        generated(refutable_destructure()),
    )
    .unwrap();
    std::fs::write(
        project.path().join("src/nested_local_values.gleam"),
        generated(nested_local_values()),
    )
    .unwrap();
    let output = std::process::Command::new(
        std::env::var_os("MORPHIR_TEST_GLEAM").unwrap_or_else(|| "gleam".into()),
    )
    .args(["check", "--target", "javascript"])
    .current_dir(project.path())
    .output()
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn classic_v3_destructure_generates_the_same_expression_block() {
    let attrs = serde_json::json!([
        "Reference",
        {},
        [[["morphir"], ["s", "d", "k"]], [["basics"]], ["int"]],
        []
    ]);
    let integer = |n| serde_json::json!(["Literal", attrs, ["WholeNumberLiteral", n]]);
    let value = serde_json::json!([
        "Destructure",
        attrs,
        ["AsPattern", attrs, ["WildcardPattern", attrs], ["value"]],
        integer(42),
        ["Variable", attrs, ["value"]]
    ]);
    let ir = serde_json::json!({"formatVersion":3,"distribution":["Library",[["example"],["package"]],[],{"modules":[[[["main"]],{"access":"Public","value":{"types":[],"values":[[["run"],{"access":"Public","value":{"doc":"","value":{"inputTypes":[],"outputType":["Reference",{},[[["morphir"],["s","d","k"]],[["basics"]],["int"]],[]],"body":value}}}]]}}]]}]});
    let result = GleamExtension
        .generate(GenerateRequest {
            ir,
            target: "gleam".into(),
            options: HashMap::new(),
        })
        .unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(
        result.artifacts[0].content,
        include_str!("goldens/v3_destructure.gleam")
    );
    parse_gleam("main.gleam", &result.artifacts[0].content).unwrap();
}

fn refutable_destructure() -> Value {
    Value::Destructure(
        Default::default(),
        Pattern::HeadTailPattern(
            Default::default(),
            Box::new(binding("head")),
            Box::new(Pattern::EmptyListPattern(Default::default())),
        ),
        Box::new(Value::List(Default::default(), vec![integer(7)])),
        Box::new(variable("head")),
    )
}

#[test]
fn refutable_destructure_preserves_the_pattern_assertion() {
    assert_eq!(
        generated(refutable_destructure()),
        include_str!("goldens/list_destructure.gleam")
    );
}

#[test]
fn local_function_definitions_fail_instead_of_discarding_parameters() {
    let definition = ValueDefinition {
        input_types: IndexMap::from([("argument".into(), Type::Unit(Default::default()))]),
        output_type: Some(Type::Unit(Default::default())),
        body: ValueBody::Expression(variable("argument")),
    };
    let result = generate_body(ValueBody::Expression(Value::LetDefinition(
        Default::default(),
        Name::from("identity"),
        Box::new(definition),
        Box::new(variable("identity")),
    )));
    assert!(
        !result.success,
        "local function parameters must not be dropped"
    );
    assert!(result.artifacts.is_empty());
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("LetDefinition") && d.message.contains("parameters")),
        "{:?}",
        result.diagnostics
    );
}
