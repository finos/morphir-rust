use morphir_core::{
    ir::v4::*,
    naming::{FQName, Name},
};
use morphir_extension_sdk::{Backend, GenerateRequest, GenerateResult};
use morphir_rust_binding::RustExtension;
use serde_json::json;

fn name(text: &str) -> Name {
    Name::from_canonical_string(text).unwrap()
}
fn ty(text: &str) -> Type {
    serde_json::from_value(json!(text)).unwrap()
}
fn int() -> Type {
    ty("morphir/SDK:basics#int")
}
fn boolean() -> Type {
    ty("morphir/SDK:basics#bool")
}
fn variable(text: &str) -> Value {
    Value::Variable(Default::default(), name(text))
}
fn integer(value: i64) -> Value {
    Value::Literal(Default::default(), Literal::Integer(value.into()))
}
fn reference(text: &str) -> Value {
    Value::Reference(
        Default::default(),
        FQName::from_canonical_string(text).unwrap(),
    )
}
fn apply(function: Value, argument: Value) -> Value {
    Value::Apply(Default::default(), Box::new(function), Box::new(argument))
}
fn callable(input: Type, output: Type) -> Type {
    Type::Function(Default::default(), Box::new(input), Box::new(output))
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
fn lambda(parameter: &str, input: Type, body: Value) -> Value {
    let attrs = ValueAttributes {
        inferred_type: Some(Box::new(input)),
        ..Default::default()
    };
    Value::Lambda(
        Default::default(),
        Pattern::AsPattern(
            attrs.clone(),
            Box::new(Pattern::WildcardPattern(attrs)),
            name(parameter),
        ),
        Box::new(body),
    )
}
fn generate(functions: &[(&str, ValueDefinition)]) -> GenerateResult {
    let values: serde_json::Map<String, serde_json::Value> = functions
        .iter()
        .map(|(name, definition)| (name.to_string(), json!({"Public":definition})))
        .collect();
    let ir = json!({"formatVersion":4,"distribution":{"Library":{"packageName":"acme/example","dependencies":{},"def":{"modules":{"models":{"Public":{"types":{},"values":values}}}}}}});
    RustExtension
        .generate(GenerateRequest {
            ir,
            target: "rust".into(),
            options: Default::default(),
        })
        .unwrap()
}
fn execute(result: GenerateResult, consumer: &str) {
    assert!(result.success, "{:?}", result.diagnostics);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let source = &result.artifacts[0].content;
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("main.rs");
    let output = dir
        .path()
        .join(format!("consumer{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&input, format!("{source}\nfn main() {{ {consumer} }}")).unwrap();
    let compiled = std::process::Command::new("rustc")
        .args(["--edition=2024"])
        .arg(input)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{source}\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let ran = std::process::Command::new(output).output().unwrap();
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
}
#[test]
fn direct_ir_named_calls_preserve_argument_order() {
    let pick = function(
        Value::IfThenElse(
            Default::default(),
            Box::new(variable("flag")),
            Box::new(variable("z")),
            Box::new(variable("a")),
        ),
        int(),
        &[("flag", boolean()), ("z", int()), ("a", int())],
    );
    let call = apply(
        apply(
            apply(reference("acme/example:models#pick"), variable("flag")),
            integer(7),
        ),
        integer(9),
    );
    execute(
        generate(&[
            ("pick", pick),
            ("select", function(call, int(), &[("flag", boolean())])),
        ]),
        "assert_eq!(models::select(true),7); assert_eq!(models::select(false),9);",
    );
}
#[test]
fn zero_input_definition_references_evaluate_the_named_value() {
    execute(
        generate(&[
            ("constant", function(integer(42), int(), &[])),
            (
                "select",
                function(reference("acme/example:models#constant"), int(), &[]),
            ),
        ]),
        "assert_eq!(models::select(),42);",
    );
}
#[test]
fn captured_lambda_values_execute_and_can_be_called_repeatedly() {
    let comparison = apply(
        apply(reference("morphir/SDK:basics#greater-than"), variable("n")),
        variable("limit"),
    );
    let predicate = lambda("n", int(), comparison);
    let body = Value::LetDefinition(
        Default::default(),
        name("predicate"),
        Box::new(function(predicate, callable(int(), boolean()), &[])),
        Box::new(Value::Tuple(
            Default::default(),
            vec![
                apply(variable("predicate"), variable("value")),
                apply(variable("predicate"), variable("limit")),
            ],
        )),
    );
    execute(
        generate(&[(
            "select",
            function(
                body,
                Type::Tuple(Default::default(), vec![boolean(), boolean()]),
                &[("limit", int()), ("value", int())],
            ),
        )]),
        "assert_eq!(models::select(10,11),(true,false)); assert_eq!(models::select(10,9),(false,false));",
    );
}
#[test]
fn higher_order_parameters_and_returned_copy_captures_execute() {
    let apply_body = apply(variable("callback"), variable("value"));
    let select = function(
        apply_body,
        int(),
        &[("callback", callable(int(), int())), ("value", int())],
    );
    let capture = function(
        lambda("ignored", int(), variable("limit")),
        callable(int(), int()),
        &[("limit", int())],
    );
    execute(
        generate(&[("select", select), ("capture", capture)]),
        "assert_eq!(models::select(::std::rc::Rc::new(|x|x),42),42); let f=models::capture(7); assert_eq!(f(1),7); assert_eq!(f(2),7);",
    );
}
#[test]
fn invalid_calls_and_lambdas_return_no_artifacts() {
    let string = ty("morphir/SDK:string#string");
    let bad_lambdas = [
        function(
            lambda("n", int(), variable("owned")),
            callable(int(), string.clone()),
            &[("owned", string)],
        ),
        function(
            lambda("n", int(), variable("missing")),
            callable(int(), int()),
            &[],
        ),
        function(apply(integer(42), integer(1)), int(), &[]),
        function(reference("acme/example:models#missing"), int(), &[]),
        function(
            apply(
                lambda("n", int(), variable("n")),
                Value::Literal(Default::default(), Literal::Bool(true)),
            ),
            int(),
            &[],
        ),
        function(
            Value::Lambda(
                Default::default(),
                Pattern::LiteralPattern(
                    ValueAttributes {
                        inferred_type: Some(Box::new(boolean())),
                        ..Default::default()
                    },
                    Literal::Bool(true),
                ),
                Box::new(integer(1)),
            ),
            callable(boolean(), int()),
            &[],
        ),
    ];
    for definition in bad_lambdas {
        let result = generate(&[("select", definition)]);
        assert!(!result.success);
        assert!(result.artifacts.is_empty());
    }
}

#[test]
fn recursive_function_references_are_diagnosed() {
    let definition = function(
        apply(reference("acme/example:models#select"), variable("value")),
        int(),
        &[("value", int())],
    );
    let result = generate(&[("select", definition)]);
    assert!(!result.success, "recursive calls must be diagnosed");
    assert!(result.artifacts.is_empty());
}

#[test]
fn tuples_of_function_handles_and_copy_values_can_be_reused() {
    let pair = Type::Tuple(Default::default(), vec![callable(int(), int()), boolean()]);
    let definition = function(
        Value::Tuple(Default::default(), vec![variable("pair"), variable("pair")]),
        Type::Tuple(Default::default(), vec![pair.clone(), pair.clone()]),
        &[("pair", pair)],
    );
    execute(
        generate(&[("select", definition)]),
        "let (left,right)=models::select((::std::rc::Rc::new(|x|x),true)); assert_eq!((left.0)(7),7); assert_eq!((right.0)(9),9); assert!(left.1 && right.1);",
    );
}
