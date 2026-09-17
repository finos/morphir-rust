use morphir_core::{ir::v4::*, naming::Name};
use morphir_extension_sdk::{Backend, GenerateRequest};
use morphir_rust_binding::RustExtension;
use serde_json::json;

fn variable(name: &str) -> Value {
    Value::Variable(
        Default::default(),
        Name::from_canonical_string(name).unwrap(),
    )
}
fn ty(text: &str) -> Type {
    serde_json::from_value(json!(text)).unwrap()
}
fn function(body: Value, output: Type, inputs: &[(&str, Type)]) -> ValueDefinition {
    ValueDefinition {
        input_types: inputs
            .iter()
            .map(|(n, t)| (n.to_string(), t.clone()))
            .collect(),
        output_type: Some(output),
        body: ValueBody::Expression(body),
    }
}
fn generate(def: ValueDefinition) -> morphir_extension_sdk::GenerateResult {
    let ir = json!({"formatVersion":4,"distribution":{"Library":{"packageName":"acme/example","dependencies":{},"def":{"modules":{"models":{"Public":{"types":{},"values":{"select":{"Public":serde_json::to_value(def).unwrap()}}}}}}}}});
    RustExtension
        .generate(GenerateRequest {
            ir,
            target: "rust".into(),
            options: Default::default(),
        })
        .unwrap()
}
fn execute(source: &str, consumer: &str) {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("main.rs");
    let output = dir.path().join("consumer");
    std::fs::write(&input, format!("{source}\nfn main() {{ {consumer} }}")).unwrap();
    let result = std::process::Command::new("rustc")
        .args(["--edition=2024"])
        .arg(input)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{source}\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        std::process::Command::new(output)
            .status()
            .unwrap()
            .success()
    );
}
#[test]
fn typed_conditional_function_executes_and_preserves_parameter_order() {
    let int = ty("morphir/SDK:basics#int");
    let boolean = ty("morphir/SDK:basics#bool");
    let body = Value::IfThenElse(
        Default::default(),
        Box::new(variable("flag")),
        Box::new(variable("z")),
        Box::new(variable("a")),
    );
    let result = generate(function(
        body,
        int.clone(),
        &[("flag", boolean), ("z", int.clone()), ("a", int)],
    ));
    assert!(result.success, "{:?}", result.diagnostics);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    execute(
        &result.artifacts[0].content,
        "assert_eq!(models::select(true, 7, 9),7); assert_eq!(models::select(false,7,9),9);",
    );
}
#[test]
fn unsupported_expression_fails_without_artifacts() {
    let result = generate(function(
        Value::List(Default::default(), vec![]),
        Type::Unit(Default::default()),
        &[],
    ));
    assert!(!result.success);
    assert!(result.artifacts.is_empty());
}
#[test]
fn repeated_owned_parameter_fails_without_artifacts() {
    let string = ty("morphir/SDK:string#string");
    let result = generate(function(
        Value::Tuple(Default::default(), vec![variable("x"), variable("x")]),
        Type::Tuple(Default::default(), vec![string.clone(), string.clone()]),
        &[("x", string)],
    ));
    assert!(!result.success);
    assert!(result.artifacts.is_empty());
}
#[test]
fn generic_selection_and_scalar_tuple_reuse_execute() {
    let boolean = ty("morphir/SDK:basics#bool");
    let generic = ty("a");
    let body = Value::IfThenElse(
        Default::default(),
        Box::new(variable("flag")),
        Box::new(variable("x")),
        Box::new(variable("y")),
    );
    let result = generate(function(
        body,
        generic.clone(),
        &[("flag", boolean), ("x", generic.clone()), ("y", generic)],
    ));
    assert!(result.success, "{:?}", result.diagnostics);
    execute(
        &result.artifacts[0].content,
        "assert_eq!(models::select(false,String::from(\"a\"),String::from(\"b\")),\"b\");",
    );
}

fn compare(left: Value, right: Value) -> Value {
    let reference = Value::Reference(
        Default::default(),
        morphir_core::naming::FQName::from_canonical_string("morphir/SDK:basics#equal").unwrap(),
    );
    Value::Apply(
        Default::default(),
        Box::new(Value::Apply(
            Default::default(),
            Box::new(reference),
            Box::new(left),
        )),
        Box::new(right),
    )
}
#[test]
fn comparison_cannot_move_a_borrowed_operand() {
    let string = ty("morphir/SDK:string#string");
    let right = Value::LetDefinition(
        Default::default(),
        Name::from_canonical_string("local").unwrap(),
        Box::new(function(variable("x"), string.clone(), &[])),
        Box::new(variable("local")),
    );
    let result = generate(function(
        compare(variable("x"), right),
        ty("morphir/SDK:basics#bool"),
        &[("x", string)],
    ));
    assert!(!result.success);
    assert!(result.artifacts.is_empty());
}
#[test]
fn anonymous_record_function_types_are_diagnosed() {
    let record = Type::Record(Default::default(), vec![]);
    let result = generate(function(variable("x"), record.clone(), &[("x", record)]));
    assert!(!result.success);
    assert!(result.artifacts.is_empty());
}
#[test]
fn scalar_tuple_reuse_and_string_comparisons_execute() {
    let int = ty("morphir/SDK:basics#int");
    let pair = Type::Tuple(Default::default(), vec![int; 2]);
    let result = generate(function(
        Value::Tuple(Default::default(), vec![variable("x"), variable("x")]),
        Type::Tuple(Default::default(), vec![pair.clone(); 2]),
        &[("x", pair)],
    ));
    assert!(result.success, "{:?}", result.diagnostics);
    execute(
        &result.artifacts[0].content,
        "assert_eq!(models::select((1,2)),((1,2),(1,2)));",
    );
    let string = ty("morphir/SDK:string#string");
    let body = Value::IfThenElse(
        Default::default(),
        Box::new(compare(variable("x"), variable("y"))),
        Box::new(variable("x")),
        Box::new(variable("y")),
    );
    let result = generate(function(
        body,
        string.clone(),
        &[("x", string.clone()), ("y", string)],
    ));
    assert!(result.success, "{:?}", result.diagnostics);
    execute(
        &result.artifacts[0].content,
        "assert_eq!(models::select(String::from(\"a\"),String::from(\"b\")),\"b\");",
    );
}
#[test]
fn classic_values_migrate_to_identical_executable_rust_with_docs_and_access() {
    let int = json!([
        "Reference",
        {},
        [[["morphir"], ["s", "d", "k"]], [["basics"]], ["int"]],
        []
    ]);
    let bool_type = json!([
        "Reference",
        {},
        [[["morphir"], ["s", "d", "k"]], [["basics"]], ["bool"]],
        []
    ]);
    let body = json!([
        "IfThenElse",
        int,
        ["Variable", bool_type, ["flag"]],
        ["Variable", int, ["z"]],
        ["Variable", int, ["a"]]
    ]);
    let v3 = json!({"formatVersion":3,"distribution":["Library",[["acme"],["example"]],[],{"modules":[[[["models"]],{"access":"Public","value":{"types":[],"values":[[["select"],{"access":"Private","value":{"doc":"Chooses a value.","value":{"inputTypes":[[["flag"],bool_type,bool_type],[["z"],int,int],[["a"],int,int]],"outputType":int,"body":body}}}]]}}]]}]});
    let classic: morphir_core::ir::classic::Distribution =
        serde_json::from_value(v3.clone()).unwrap();
    let v4 = serde_json::to_value(
        morphir_core::migration::migrate_distribution(&classic, Default::default())
            .unwrap()
            .value,
    )
    .unwrap();
    let outputs = [v3, v4].map(|ir| {
        RustExtension
            .generate(GenerateRequest {
                ir,
                target: "rust".into(),
                options: Default::default(),
            })
            .unwrap()
    });
    for result in &outputs {
        assert!(result.success, "{:?}", result.diagnostics);
        assert!(result.diagnostics.is_empty());
    }
    assert_eq!(
        outputs[0].artifacts[0].content,
        outputs[1].artifacts[0].content
    );
    let source = &outputs[0].artifacts[0].content;
    assert!(source.contains("Chooses a value."));
    assert!(source.contains("pub(crate) fn select"));
    execute(
        source,
        "assert_eq!(models::select(true,7,9),7); assert_eq!(models::select(false,7,9),9);",
    );
}

#[test]
fn lexical_shadowing_keeps_owned_bindings_distinct() {
    let string = ty("morphir/SDK:string#string");
    let local = Value::LetDefinition(
        Default::default(),
        Name::from_canonical_string("x").unwrap(),
        Box::new(function(variable("y"), string.clone(), &[])),
        Box::new(variable("x")),
    );
    let result = generate(function(
        Value::Tuple(Default::default(), vec![local, variable("x")]),
        Type::Tuple(Default::default(), vec![string.clone(); 2]),
        &[("x", string.clone()), ("y", string)],
    ));
    assert!(result.success, "{:?}", result.diagnostics);
    execute(
        &result.artifacts[0].content,
        "assert_eq!(models::select(String::from(\"outer\"),String::from(\"inner\")),(String::from(\"inner\"),String::from(\"outer\")));",
    );
}

#[test]
fn conditional_owned_comparison_operands_are_moves() {
    let string = ty("morphir/SDK:string#string");
    let boolean = ty("morphir/SDK:basics#bool");
    let conditional = Value::IfThenElse(
        Default::default(),
        Box::new(variable("flag")),
        Box::new(variable("x")),
        Box::new(variable("y")),
    );
    let comparison = compare(
        conditional,
        Value::Literal(Default::default(), Literal::String("value".into())),
    );
    let result = generate(function(
        Value::Tuple(Default::default(), vec![comparison, variable("x")]),
        Type::Tuple(Default::default(), vec![boolean.clone(), string.clone()]),
        &[("flag", boolean), ("x", string.clone()), ("y", string)],
    ));
    assert!(!result.success);
    assert!(result.artifacts.is_empty());
}

#[test]
fn non_expression_declarations_keep_omission_warnings() {
    for body in [
        ValueBody::External {
            externals: vec![ExternalBinding {
                target_platform: "rust".into(),
                external_name: "external".into(),
            }],
            fallback: None,
        },
        ValueBody::Incomplete {
            incompleteness: Incompleteness::Draft,
            partial_body: None,
        },
    ] {
        let result = generate(ValueDefinition {
            input_types: Default::default(),
            output_type: Some(Type::Unit(Default::default())),
            body,
        });
        assert!(result.success, "{:?}", result.diagnostics);
        assert_eq!(
            result.diagnostics[0].code.as_deref(),
            Some("RS_VALUES_OMITTED")
        );
        assert!(!result.artifacts[0].content.contains("fn select"));
    }
}

#[test]
fn function_and_type_with_same_morphir_name_do_not_box_the_signature() {
    let foo = ty("acme/example:models#foo");
    let def = function(variable("x"), foo.clone(), &[("x", foo)]);
    let ir = json!({"formatVersion":4,"distribution":{"Library":{"packageName":"acme/example","dependencies":{},"def":{"modules":{"models":{"Public":{
        "types":{"foo":{"Public":{"TypeAliasDefinition":{"typeParams":[],"typeExp":"morphir/SDK:basics#int"}}}},
        "values":{"foo":{"Public":serde_json::to_value(def).unwrap()}}
    }}}}}}});
    let result = RustExtension
        .generate(GenerateRequest {
            ir,
            target: "rust".into(),
            options: Default::default(),
        })
        .unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    execute(
        &result.artifacts[0].content,
        "assert_eq!(models::foo(42),42);",
    );
}
