use morphir_extension_sdk::{
    native::NativeExtension,
    prelude::*,
    protocol::{ExtensionRequest, methods},
};
use morphir_python_binding::PythonExtension;
use serde_json::{Value, json};

fn request(source: &str) -> CompileRequest {
    CompileRequest {
        language_id: "python".into(),
        documents: vec![SourceDocument {
            uri: "models.py".into(),
            language_id: "python".into(),
            version: 1,
            text: source.into(),
        }],
        package: CompilePackage {
            name: "acme/example".into(),
            exposed_modules: vec![],
        },
        dependencies: vec![],
        options: CompileOptions {
            ir_version: "4".into(),
            ..Default::default()
        },
    }
}

fn compile(source: &str) -> CompileResult {
    let extension = NativeExtension::frontend_backend(PythonExtension).unwrap();
    let response = extension
        .protocol()
        .handle(ExtensionRequest::new(methods::COMPILE, request(source), 1).unwrap());
    serde_json::from_value(response.result.unwrap()).unwrap()
}

fn generate(ir: Value) -> GenerateResult {
    PythonExtension
        .generate(GenerateRequest {
            ir,
            target: "python".into(),
            options: Default::default(),
        })
        .unwrap()
}

fn roundtrip(source: &str) -> Value {
    let compiled = compile(source);
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    let ir = compiled.ir.unwrap();
    let generated = generate(ir.clone());
    assert!(generated.success, "{:?}", generated.diagnostics);
    let again = compile(&generated.artifacts[0].content);
    assert!(again.success, "{:?}", again.diagnostics);
    assert_eq!(again.ir.as_ref(), Some(&ir));
    ir
}

#[test]
fn conditionals_lower_to_canonical_if_then_else_and_roundtrip() {
    let ir = roundtrip(include_str!("fixtures/conditionals.py"));
    let choose = &ir["distribution"]["Library"]["def"]["modules"]["models"]["Public"]["values"]["choose"]
        ["Public"]["ExpressionBody"];
    assert_eq!(
        choose["inputTypes"],
        json!({"flag": "morphir/SDK:basics#bool", "first": "morphir/SDK:basics#int", "second": "morphir/SDK:basics#int"})
    );
    assert_eq!(choose["outputType"], json!("morphir/SDK:basics#int"));
    assert_eq!(
        choose["body"],
        json!({"IfThenElse": {"condition": {"Variable": "flag"}, "then": {"Variable": "first"}, "else": {"Variable": "second"}}})
    );
}

#[test]
fn branches_can_select_adt_and_tuple_parameters() {
    roundtrip(&format!(
        "{}\ndef select(flag: bool, first: Decision, second: Decision) -> Decision:\n    return first if flag else second\n\ndef pair(flag: bool, first: tuple[int, str], second: tuple[int, str]) -> tuple[int, str]:\n    return first if flag else second\n",
        include_str!("fixtures/models.py")
    ));
}

#[test]
fn tuple_types_and_values_have_distinct_ir_encodings() {
    let ir = roundtrip(
        "def pair(amount: int, label: str) -> tuple[int, str]:\n    return amount, label\n",
    );
    let definition = &ir["distribution"]["Library"]["def"]["modules"]["models"]["Public"]["values"]
        ["pair"]["Public"]["ExpressionBody"];
    assert_eq!(
        definition["outputType"],
        json!({"Tuple": ["morphir/SDK:basics#int", "morphir/SDK:string#string"]})
    );
    assert_eq!(
        definition["body"],
        json!({"Tuple": [{"Variable": "amount"}, {"Variable": "label"}]})
    );
}

#[test]
fn named_and_nested_tuple_aliases_roundtrip() {
    roundtrip(include_str!("fixtures/tuples.py"));
    roundtrip(
        "type Pair = tuple[int, str]\ndef select(flag: bool, pair: Pair) -> tuple[int, str]:\n    return pair if flag else (0, 'none')\n",
    );
    roundtrip(
        "type Pair = tuple[int, str]\ntype Nested = tuple[Pair, bool]\ndef make_nested(value: int) -> Nested:\n    return ((value, 'value'), True)\n",
    );
}

#[test]
fn invalid_tuple_types_and_values_are_rejected() {
    for source in [
        "def f() -> tuple[int, str]:\n    return (1, 2)\n",
        "def f() -> tuple[int, str]:\n    return (1, 'a', True)\n",
        "def f(flag: bool) -> tuple[int, str]:\n    return (1, 'a') if flag else (2, False)\n",
        "def f() -> tuple[int]:\n    return (1,)\n",
        "def f() -> tuple[()]:\n    return ()\n",
        "type Pair = tuple[int, ...]\n",
        "type Pair = tuple[int, Pair]\n",
        "type A = tuple[int, B]\ntype B = tuple[str, A]\n",
        "type Pair = tuple[int, str]\ndef f() -> Pair:\n    return [1, 'a']\n",
        "def f(pair: tuple[int, str]) -> tuple[int, str]:\n    return (*pair,)\n",
    ] {
        let result = compile(source);
        assert!(!result.success, "accepted {source}");
        assert!(result.ir.is_none());
    }
}

#[test]
fn backend_accepts_independent_tuple_value_and_rejects_wrong_elements() {
    let mut ir = independent_ir(
        json!({"Tuple": [{"Literal": {"IntegerLiteral": 1}}, {"Literal": {"StringLiteral": "one"}}]}),
    );
    ir["distribution"]["Library"]["def"]["modules"]["models"]["Public"]["values"]["select"]["Public"]
        ["ExpressionBody"]["outputType"] =
        json!({"Tuple": ["morphir/SDK:basics#int", "morphir/SDK:string#string"]});
    let generated = generate(ir.clone());
    assert!(generated.success, "{:?}", generated.diagnostics);
    assert_eq!(
        compile(&generated.artifacts[0].content).ir,
        Some(ir.clone())
    );
    ir["distribution"]["Library"]["def"]["modules"]["models"]["Public"]["values"]["select"]["Public"]
        ["ExpressionBody"]["body"]["Tuple"][1] = json!({"Literal": {"BoolLiteral": true}});
    let invalid = generate(ir);
    assert!(!invalid.success);
    assert!(invalid.artifacts.is_empty());
}

#[test]
fn comparisons_and_literal_boundaries_roundtrip() {
    for operator in ["==", "!=", "<", "<=", ">", ">="] {
        roundtrip(&format!(
            "def compare(left: int, right: int) -> str:\n    return 'yes' if left {operator} right else 'no'\n"
        ));
    }
    roundtrip(
        "def limits(flag: bool) -> int:\n    return -9223372036854775808 if flag else 9223372036854775807\n",
    );
    roundtrip(
        "def text(flag: bool) -> str:\n    return 'line\\n\\x00\\U0001f600' if flag else '\"quoted\"'\n",
    );
}

#[test]
fn invalid_functions_fail_without_partial_ir() {
    for source in [
        "def f(flag: bool) -> int:\n    if flag:\n        return 1\n",
        "def f(flag: bool) -> int:\n    return 1 if flag else 'no'\n",
        "def f(flag: int) -> int:\n    return 1 if flag else 2\n",
        "def f(flag: bool) -> int:\n    return missing if flag else 2\n",
        "def f(flag: bool) -> int:\n    if flag:\n        print('side effect')\n        return 1\n    return 2\n",
        "def f(flag: bool) -> int:\n    result = 1\n    return result\n",
        "def f(flag: bool = True) -> int:\n    return 1\n",
        "def f(flag) -> int:\n    return 1\n",
        "def f(flag: bool):\n    return 1\n",
        "async def f(flag: bool) -> int:\n    return 1\n",
        "def f(*, flag: bool) -> int:\n    return 1\n",
        "def f(flag: bool, /) -> int:\n    return 1\n",
        "def f(a_b: bool, aB: bool) -> int:\n    return 1\n",
        "def f(Class: bool) -> int:\n    return 1\n",
        "def f(flag: bool) -> int:\n    return\n",
        "def f(flag: bool) -> int:\n    return 1\n    return 2\n",
        "def f(flag: bool) -> int:\n    return 9223372036854775808\n",
        "def f(flag: bool) -> float:\n    return 1e400\n",
        "def f(flag: bool) -> int:\n    return 1 if flag < True else 2\n",
        "def f(a: int, b: str) -> int:\n    return 1 if a == b else 2\n",
        "def f(flag: bool) -> int:\n    return 1 if 0 < 1 < 2 else 2\n",
    ] {
        let result = compile(source);
        assert!(!result.success, "accepted {source}");
        assert!(result.ir.is_none());
        assert!(!result.diagnostics.is_empty());
    }
}

#[test]
fn types_only_does_not_silently_emit_function_bodies() {
    let mut request = request(include_str!("fixtures/conditionals.py"));
    request.options.types_only = true;
    let result = PythonExtension.compile(request).unwrap();
    assert!(!result.success);
    assert!(result.ir.is_none());
}

fn independent_ir(body: Value) -> Value {
    let definition = json!({"ExpressionBody": {
        "inputTypes": {"flag": "morphir/SDK:basics#bool"},
        "outputType": "morphir/SDK:basics#int", "body": body
    }});
    json!({"formatVersion": 4, "distribution": {"Library": {
        "packageName": "acme/example", "dependencies": {}, "def": {"modules": {
            "models": {"Public": {"types": {}, "values": {"select": {"Public": definition}}}}
        }}
    }}})
}

#[test]
fn backend_accepts_independent_if_then_else_ir() {
    let ir = independent_ir(
        json!({"IfThenElse": {"condition": {"Variable": "flag"}, "then": {"Literal": {"IntegerLiteral": 1}}, "else": {"Literal": {"IntegerLiteral": 2}}}}),
    );
    let result = generate(ir.clone());
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(compile(&result.artifacts[0].content).ir, Some(ir));
}

#[test]
fn backend_rejects_invalid_bodies_without_artifacts() {
    for body in [
        json!({"IfThenElse": {"condition": {"Literal": {"IntegerLiteral": 1}}, "then": {"Literal": {"IntegerLiteral": 1}}, "else": {"Literal": {"IntegerLiteral": 2}}}}),
        json!({"IfThenElse": {"condition": {"Variable": "flag"}, "then": {"Literal": {"IntegerLiteral": 1}}, "else": {"Literal": {"StringLiteral": "no"}}}}),
        json!({"Variable": "unknown"}),
        json!({"Reference": "acme/example:models#select"}),
    ] {
        let result = generate(independent_ir(body));
        assert!(!result.success);
        assert!(result.artifacts.is_empty());
    }
}
