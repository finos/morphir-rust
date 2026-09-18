use morphir_core::ir::classic;
use morphir_extension_sdk::prelude::*;
use morphir_python_binding::PythonExtension;
use serde_json::{Value, json};

fn a_request(version: &str) -> CompileRequest {
    CompileRequest {
        language_id: "python".into(),
        documents: vec![
            SourceDocument {
                uri: "domain/models.py".into(), language_id: "python".into(), version: 1,
                text: concat!(
                    "from __future__ import annotations\nfrom dataclasses import dataclass\n",
                    "@dataclass(frozen=True)\nclass Pending:\n    pass\n",
                    "@dataclass(frozen=True)\nclass Approved:\n    amount: float\n",
                    "type Decision = Pending | Approved\n",
                    "type Point = tuple[float, float]\n",
                    "@dataclass(frozen=True)\nclass APIResponse:\n    decision: Decision\n    point: Point\n",
                ).into(),
            },
            SourceDocument {
                uri: "domain/rules.py".into(), language_id: "python".into(), version: 1,
                text: concat!(
                    "from .models import Decision, Point\n",
                    "def choose(flag: bool, first: Decision, second: Decision) -> Decision:\n",
                    "    return first if flag else second\n",
                    "def locate(value: int, point: Point) -> tuple[Point, str]:\n",
                    "    if value < 0:\n        return (point, 'negative')\n",
                    "    elif value == 0:\n        return ((0.0, 0.0), 'zero')\n",
                    "    else:\n        return (point, 'positive')\n",
                ).into(),
            },
        ],
        package: CompilePackage { name: "acme/example".into(), exposed_modules: Some(vec!["domain.rules".into()]) },
        dependencies: vec![],
        options: CompileOptions { ir_version: version.into(), types_only: false, ..Default::default() },
    }
}

fn compile(request: CompileRequest) -> Value {
    let expected = if request.options.ir_version.starts_with('3') {
        "3"
    } else {
        "4"
    };
    let result = PythonExtension.compile(request).unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.ir_version.as_deref(), Some(expected));
    result.ir.unwrap()
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

#[test]
fn both_versions_preserve_imports_privacy_tuples_and_conditionals() {
    for version in ["3", "3.0.0", "4", "4.0.0"] {
        let mut request = a_request(version);
        let ir = compile(request.clone());
        if version.starts_with('3') {
            let decoded: classic::Distribution = serde_json::from_value(ir.clone()).unwrap();
            assert_eq!(decoded.format_version, 3);
            let classic::DistributionBody::Library(package, _, definition) = decoded.distribution;
            assert_eq!(
                serde_json::to_value(package).unwrap(),
                json!([["acme"], ["example"]])
            );
            assert_eq!(
                definition.modules[0].definition.access,
                classic::Access::Private
            );
            assert_eq!(
                definition.modules[1].definition.access,
                classic::Access::Public
            );
            assert_eq!(ir["distribution"][0], "Library");
        }
        let generated = generate(ir.clone());
        assert!(generated.success, "{:?}", generated.diagnostics);
        assert_eq!(generated.artifacts.len(), 2);
        request.documents = generated
            .artifacts
            .into_iter()
            .map(|artifact| SourceDocument {
                uri: artifact.path,
                text: artifact.content,
                language_id: "python".into(),
                version: 1,
            })
            .collect();
        assert_eq!(compile(request), ir);
    }
}

#[test]
fn independently_authored_v3_record_generates_python() {
    let scalar = json!([
        "Reference",
        {},
        [[["morphir"], ["s", "d", "k"]], [["basics"]], ["int"]],
        []
    ]);
    let ir = json!({"formatVersion": 3, "distribution": ["Library", [["acme"], ["example"]], [], {"modules": [
        [[["models"]], {"access":"Public", "value":{"types":[
            [["record"], {"access":"Public", "value":{"doc":"", "value":["TypeAliasDefinition", [], ["Record", {}, [
                {"name":["count"], "tpe":scalar}
            ]]]}}]
        ], "values":[]}}]
    ]}]});
    let result = generate(ir);
    assert!(result.success, "{:?}", result.diagnostics);
    assert!(result.artifacts[0].content.contains("count: int"));
}

#[test]
fn independently_authored_typed_v3_conditional_generates_python() {
    let scalar = |name| {
        json!([
            "Reference",
            {},
            [[["morphir"], ["s", "d", "k"]], [["basics"]], [name]],
            []
        ])
    };
    let integer = scalar("int");
    let boolean = scalar("bool");
    let ir = json!({"formatVersion":3,"distribution":["Library",[["acme"],["example"]],[],{"modules":[
        [[["models"]],{"access":"Public","value":{"types":[],"values":[
            [["choose"],{"access":"Public","value":{"doc":"","value":{
                "inputTypes":[[["flag"],boolean,boolean]], "outputType":integer,
                "body":["IfThenElse",integer,["Variable",boolean,["flag"]],
                    ["Literal",integer,["WholeNumberLiteral",1]],["Literal",integer,["WholeNumberLiteral",2]]]
            }}}]
        ]}}]
    ]}]});
    let result = generate(ir);
    assert!(result.success, "{:?}", result.diagnostics);
    assert!(
        result.artifacts[0]
            .content
            .contains("def choose(flag: bool) -> int:")
    );
    assert!(result.artifacts[0].content.contains("if flag:"));
}

#[test]
fn v3_parameter_annotations_accept_an_expanded_tuple_alias() {
    let mut ir = compile(a_request("3"));
    let float = json!([
        "Reference",
        {},
        [[["morphir"], ["s", "d", "k"]], [["basics"]], ["float"]],
        []
    ]);
    ir["distribution"][3]["modules"][1][1]["value"]["values"][1][1]["value"]["value"]["inputTypes"]
        [1][1] = json!(["Tuple", {}, [float.clone(), float]]);
    let result = generate(ir);
    assert!(result.success, "{:?}", result.diagnostics);
}

#[test]
fn v3_integer_range_is_checked_without_affecting_v4() {
    for number in ["-9223372036854775808", "9223372036854775807"] {
        let mut request = a_request("3");
        request.documents[1].text = format!("def number() -> int:\n    return {number}\n");
        let result = generate(compile(request));
        assert!(result.success, "{:?}", result.diagnostics);
    }
    for number in ["9223372036854775808", "-9223372036854775809"] {
        let mut request = a_request("3");
        request.documents[1].text = format!("def number() -> int:\n    return {number}\n");
        let result = PythonExtension.compile(request.clone()).unwrap();
        assert!(!result.success);
        assert!(result.ir.is_none());
        assert!(result.diagnostics[0].message.contains("64-bit"));
        request.options.ir_version = "4".into();
        compile(request);
    }
}

#[test]
fn rejects_unsupported_versions_and_inconsistent_v3_value_annotations() {
    for version in ["1", "2", "3.1.0", "3.0.1", "4.1.0", "3.0", "03", "latest"] {
        let result = PythonExtension.compile(a_request(version)).unwrap();
        assert!(!result.success, "accepted {version}");
        assert!(result.ir.is_none());
    }
    let mut ir = compile(a_request("3"));
    ir["distribution"][3]["modules"][1][1]["value"]["values"][0][1]["value"]["value"]["body"][1] =
        json!(["Unit", {}]);
    let result = generate(ir);
    assert!(!result.success);
    assert!(result.artifacts.is_empty());
}

#[test]
fn backend_reads_compatible_patches_but_rejects_other_minor_versions() {
    for baseline in ["3", "4"] {
        let mut ir = compile(a_request(baseline));
        ir["formatVersion"] = json!(format!("{baseline}.0.7"));
        let result = generate(ir.clone());
        assert!(result.success, "{:?}", result.diagnostics);
        ir["formatVersion"] = json!(format!("{baseline}.1.0"));
        assert!(!generate(ir).success);
    }
}

#[test]
fn v3_rejects_duplicate_names_documentation_and_untyped_function_bodies() {
    let source = compile(a_request("3"));
    let mut duplicate = source.clone();
    let module = duplicate["distribution"][3]["modules"][0].clone();
    duplicate["distribution"][3]["modules"]
        .as_array_mut()
        .unwrap()
        .push(module);
    assert!(!generate(duplicate).success);
    let mut docs = source.clone();
    docs["distribution"][3]["modules"][0][1]["value"]["types"][0][1]["value"]["doc"] =
        json!("Cannot preserve this");
    assert!(!generate(docs).success);
    let mut untyped = source;
    untyped["distribution"][3]["modules"][1][1]["value"]["values"][0][1]["value"]["value"]["body"]
        [1] = json!({});
    assert!(!generate(untyped).success);
}

#[test]
fn classic_names_cannot_silently_change_the_python_model() {
    let mut request = a_request("4");
    request.documents.truncate(1);
    request.package.exposed_modules = None;
    request.documents[0].text =
        "from dataclasses import dataclass\n@dataclass(frozen=True)\nclass Record:\n    a_b: int\n"
            .into();
    let expected = generate(compile(request.clone()));
    assert!(expected.success);
    request.options.ir_version = "3".into();
    let compiled = PythonExtension.compile(request).unwrap();
    if compiled.success {
        let actual = generate(compiled.ir.unwrap());
        assert!(actual.success, "{:?}", actual.diagnostics);
        assert_eq!(actual.artifacts[0].content, expected.artifacts[0].content);
    } else {
        assert_eq!(compiled.diagnostics[0].code.as_deref(), Some("PY003"));
    }
}
