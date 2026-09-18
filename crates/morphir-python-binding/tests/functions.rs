use morphir_extension_sdk::prelude::*;
use morphir_python_binding::PythonExtension;

fn request(version: &str, source: &str) -> CompileRequest {
    CompileRequest {
        language_id: "python".into(),
        documents: vec![SourceDocument {
            uri: "functions.py".into(),
            language_id: "python".into(),
            text: source.into(),
            ..Default::default()
        }],
        package: CompilePackage {
            name: "acme/functions".into(),
            ..Default::default()
        },
        options: CompileOptions {
            ir_version: version.into(),
            types_only: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

const FUNCTIONS: &str = include_str!("fixtures/functions.py");

#[test]
fn functions_calls_and_captured_lambdas_roundtrip_in_both_ir_versions() {
    for version in ["3", "4"] {
        let mut input = request(version, FUNCTIONS);
        input.documents.push(SourceDocument {
            uri: "consumer.py".into(), language_id: "python".into(),
            text: "from typing import Callable as Fn\nfrom functions import identity as ident, apply\ndef use(value: int) -> int:\n    return apply(ident, value)\ndef make(value: int) -> Fn[[int], int]:\n    return lambda ignored: ident(value)\n".into(),
            ..Default::default()
        });
        input.package.exposed_modules = Some(vec!["consumer".into()]);
        let compiled = PythonExtension.compile(input.clone()).unwrap();
        assert!(compiled.success, "{version}: {:?}", compiled.diagnostics);
        let ir = compiled.ir.unwrap();
        let generated = PythonExtension
            .generate(GenerateRequest {
                target: "python".into(),
                ir: ir.clone(),
                options: Default::default(),
            })
            .unwrap();
        assert!(generated.success, "{version}: {:?}", generated.diagnostics);
        assert!(
            generated
                .artifacts
                .iter()
                .any(|a| a.content.contains("lambda"))
        );
        input.documents = generated
            .artifacts
            .into_iter()
            .map(|artifact| SourceDocument {
                uri: artifact.path,
                text: artifact.content,
                language_id: "python".into(),
                ..Default::default()
            })
            .collect();
        let recompiled = PythonExtension.compile(input).unwrap();
        assert!(
            recompiled.success,
            "{version}: {:?}",
            recompiled.diagnostics
        );
        assert_eq!(recompiled.ir, Some(ir));
    }
}

#[test]
fn invalid_calls_and_lambda_types_return_diagnostics_without_ir() {
    for version in ["3", "4"] {
        for source in [
            "def bad(value: int) -> int:\n    return missing(value)\n",
            "from typing import Callable\ndef many(first: int, second: int) -> int:\n    return first\ndef bad(value: int) -> Callable[[int], Callable[[int], int]]:\n    return many\n",
            "from typing import Callable\ndef constant() -> int:\n    return 1\ndef bad(value: int) -> Callable[[int], int]:\n    return constant\n",
            "from typing import Callable\ndef bad(value: Callable[[int], int]) -> int:\n    return value(True)\n",
            "from typing import Callable\ndef bad(value: int) -> Callable[[int], int]:\n    return lambda item: absent\n",
            "def identity(value: int) -> int:\n    return value\ndef bad(value: int) -> int:\n    return identity(True)\n",
            "def choose(a: int, b: int) -> int:\n    return a\ndef bad(value: int) -> int:\n    return choose(value)\n",
            "from collections.abc import Callable\ndef bad(value: int) -> Callable[[int], int]:\n    return lambda item: True\n",
            "from collections.abc import Callable\ndef bad(value: int) -> Callable[[int], int]:\n    return lambda first, second: first\n",
            "from collections.abc import Callable\ndef bad(value: int) -> Callable[[int], int]:\n    return lambda item=1: item\n",
            "from collections.abc import Callable\ndef bad(fooBar: int) -> Callable[[int], int]:\n    return lambda foo_bar: fooBar\n",
            "from collections.abc import Callable\ndef bad(value: Callable[[int, int], int]) -> int:\n    return value(1, 2)\n",
            "def identity(value: int) -> int:\n    return value\ndef bad(value: int) -> int:\n    return identity(value=value)\n",
            "def identity(value: int) -> int:\n    return value\ndef bad(value: int) -> int:\n    return identity(*value)\n",
        ] {
            let result = PythonExtension.compile(request(version, source)).unwrap();
            assert!(!result.success, "{version}: {source}");
            assert!(result.ir.is_none());
            assert!(!result.diagnostics.is_empty());
        }
    }
}

fn generate(ir: serde_json::Value) -> GenerateResult {
    PythonExtension
        .generate(GenerateRequest {
            target: "python".into(),
            ir,
            options: Default::default(),
        })
        .unwrap()
}

/// Authored directly against the classic IR contract, independently of the frontend.
fn classic_lambda() -> serde_json::Value {
    use serde_json::json;
    let int = json!([
        "Reference",
        {},
        [[["morphir"], ["s", "d", "k"]], [["basics"]], ["int"]],
        []
    ]);
    let function = json!(["Function", {}, int, int]);
    let identity = json!([
        "Lambda",
        function,
        ["AsPattern", int, ["WildcardPattern", int], ["item"]],
        ["Variable", int, ["item"]]
    ]);
    json!({"formatVersion":3,"distribution":["Library",[["acme"],["functions"]],[],{"modules":[
        [[["functions"]],{"access":"Public","value":{"doc":null,"types":[],"values":[
            [["run"],{"access":"Public","value":{"doc":"","value":{
                "inputTypes":[[["value"],int,int]],"outputType":int,
                "body":["Apply",int,identity,["Variable",int,["value"]]]
            }}}]
        ]}}]
    ]}]})
}

fn independent_closure() -> serde_json::Value {
    use morphir_core::ir::v4::*;
    use morphir_core::naming::FQName;
    let int = Type::Reference(
        Default::default(),
        FQName::from_canonical_string("morphir/SDK:basics#int").unwrap(),
        vec![],
    );
    let variable = |name: &str| {
        Value::Variable(
            Default::default(),
            Name::from_canonical_string(name).unwrap(),
        )
    };
    let identity = ValueDefinition {
        input_types: [("value".into(), int.clone())].into_iter().collect(),
        output_type: Some(int.clone()),
        body: ValueBody::Expression(variable("value")),
    };
    let closure = ValueDefinition {
        input_types: [
            ("identity".into(), int.clone()),
            ("morphir-module-1".into(), int.clone()),
        ]
        .into_iter()
        .collect(),
        output_type: Some(Type::Function(
            Default::default(),
            Box::new(int.clone()),
            Box::new(int),
        )),
        body: ValueBody::Expression(Value::Lambda(
            Default::default(),
            Pattern::AsPattern(
                Default::default(),
                Box::new(Pattern::WildcardPattern(Default::default())),
                Name::from_canonical_string("morphir-module-2").unwrap(),
            ),
            Box::new(Value::Apply(
                Default::default(),
                Box::new(Value::Reference(
                    Default::default(),
                    FQName::from_canonical_string("acme/functions:functions#identity").unwrap(),
                )),
                Box::new(variable("identity")),
            )),
        )),
    };
    let value = |definition| AccessControlled {
        access: Access::Public,
        value: Documented {
            doc: None,
            value: definition,
        },
    };
    serde_json::to_value(IRFile {
        format_version: FormatVersion::Integer(4),
        distribution: Distribution::Library(LibraryContent {
            package_name: PackageName::from_canonical_string("acme/functions").unwrap(),
            dependencies: Default::default(),
            def: PackageDefinition {
                modules: [(
                    "functions".into(),
                    AccessControlled {
                        access: Access::Public,
                        value: ModuleDefinition {
                            types: Default::default(),
                            values: [
                                ("identity".into(), value(identity)),
                                ("make".into(), value(closure)),
                            ]
                            .into_iter()
                            .collect(),
                            doc: None,
                        },
                    },
                )]
                .into_iter()
                .collect(),
            },
        }),
    })
    .unwrap()
}

#[test]
fn independent_ir_keeps_global_references_distinct_from_captured_parameters() {
    let ir = independent_closure();
    let generated = generate(ir.clone());
    assert!(generated.success, "{:?}", generated.diagnostics);
    assert!(
        generated.artifacts[0]
            .content
            .contains("import functions as morphir_module_3")
    );
    let compiled = PythonExtension
        .compile(request("4", &generated.artifacts[0].content))
        .unwrap();
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    assert_eq!(
        serde_json::from_value::<morphir_core::ir::v4::IRFile>(compiled.ir.unwrap()).unwrap(),
        serde_json::from_value::<morphir_core::ir::v4::IRFile>(ir).unwrap()
    );
}

#[test]
fn backend_rejects_unsupported_lambda_patterns_and_unknown_references() {
    use serde_json::json;
    for (path, replacement) in [
        ("/pattern", json!({"WildcardPattern": {}})),
        (
            "/body/Apply/function",
            json!({"Reference": "acme/functions:missing#identity"}),
        ),
        ("/body/Apply/argument", json!({"Variable": "missing"})),
    ] {
        let mut ir = independent_closure();
        let body = &mut ir["distribution"]["Library"]["def"]["modules"]["functions"]["Public"]["values"]
            ["make"]["Public"]["ExpressionBody"]["body"]["Lambda"];
        *body.pointer_mut(path).unwrap() = replacement;
        let result = generate(ir);
        assert!(!result.success, "accepted {path}");
        assert!(result.artifacts.is_empty());
    }
}

#[test]
fn function_exports_cannot_be_used_as_type_references() {
    let encoded = independent_closure().to_string().replace(
        "morphir/SDK:basics#int",
        "acme/functions:functions#identity",
    );
    let result = generate(serde_json::from_str(&encoded).unwrap());
    assert!(
        !result.success,
        "a value reference was accepted in a type position"
    );
    assert!(result.artifacts.is_empty());
}

#[test]
fn callable_qualified_annotations_and_aliases_resolve_statically() {
    for version in ["3", "4"] {
        for import in ["import typing as types", "import collections.abc as types"] {
            let source = format!(
                "{import}\ndef apply(transform: types.Callable[[int], int], value: int) -> int:\n    return transform(value)\n"
            );
            let result = PythonExtension.compile(request(version, &source)).unwrap();
            assert!(result.success, "{:?}", result.diagnostics);
        }
    }
}

#[test]
fn independently_authored_classic_lambda_and_application_generate_and_roundtrip() {
    let ir = classic_lambda();
    let result = generate(ir.clone());
    assert!(result.success, "{:?}", result.diagnostics);
    let compiled = PythonExtension
        .compile(request("3", &result.artifacts[0].content))
        .unwrap();
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    assert_eq!(compiled.ir, Some(ir));
}

#[test]
fn classic_lambda_and_pattern_annotations_are_checked() {
    use serde_json::json;
    let boolean = json!([
        "Reference",
        {},
        [[["morphir"], ["s", "d", "k"]], [["basics"]], ["bool"]],
        []
    ]);
    for path in ["/1", "/2/1", "/2/2/1", "/3/1"] {
        let mut ir = classic_lambda();
        let lambda = &mut ir["distribution"][3]["modules"][0][1]["value"]["values"][0][1]["value"]
            ["value"]["body"][2];
        *lambda.pointer_mut(path).unwrap() = boolean.clone();
        let result = generate(ir);
        assert!(!result.success, "accepted malformed annotation {path}");
        assert!(result.artifacts.is_empty());
    }
}

#[test]
#[ignore = "requires MORPHIR_TEST_PYTHON; exercised in the Python extension CI job"]
fn generated_python_executes_functions_and_closures() {
    use std::{fs, process::Command};
    let python = std::env::var_os("MORPHIR_TEST_PYTHON")
        .expect("set MORPHIR_TEST_PYTHON to a Python 3.12+ executable");
    for version in ["3", "4"] {
        let mut input = request(version, FUNCTIONS);
        input.documents.push(SourceDocument { uri: "consumer.py".into(), language_id: "python".into(), text: "from functions import apply, identity\ndef use(value: int) -> int:\n    return apply(identity, value)\n".into(), ..Default::default() });
        let compiled = PythonExtension.compile(input).unwrap();
        assert!(compiled.success, "{:?}", compiled.diagnostics);
        let generated = generate(compiled.ir.unwrap());
        assert!(generated.success, "{:?}", generated.diagnostics);
        let directory = tempfile::tempdir().unwrap();
        for artifact in generated.artifacts {
            fs::write(directory.path().join(artifact.path), artifact.content).unwrap();
        }
        let output = Command::new(&python).current_dir(directory.path()).arg("-c").arg(concat!(
            "import functions as f, consumer\n",
            "assert f.run(7) == 7\nassert consumer.use(9) == 9\n",
            "assert f.capture(7)(2) == 7\nassert f.shadow(7)(2) == 2\n",
            "assert f.curry(True)(1)(2) == 1\nassert f.curry(False)(1)(2) == 2\n",
            "assert f.recurse(True, 8) == 8\nassert f.read_constant(0) == 42\nassert f.immediate(5) == 5\n",
            "assert f.tupled(7)[0](2) == 7\nassert f.tupled(7)[1] == 7\n",
            "assert f.conditional(True)(4) == 4\nassert f.conditional(False)(4) == 4\nassert f.forward(6) == 6\n",
        )).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let generated = generate(independent_closure());
    assert!(generated.success, "{:?}", generated.diagnostics);
    let directory = tempfile::tempdir().unwrap();
    for artifact in generated.artifacts {
        fs::write(directory.path().join(artifact.path), artifact.content).unwrap();
    }
    let output = Command::new(&python)
        .current_dir(directory.path())
        .args([
            "-c",
            "import functions; assert functions.make(7, 99)(3) == 7",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
