use morphir_extension_sdk::{
    native::NativeExtension,
    prelude::*,
    protocol::{ExtensionRequest, methods},
};
use morphir_python_binding::PythonExtension;
use serde_json::json;

fn a_request(source: &str) -> CompileRequest {
    CompileRequest {
        language_id: "python".into(),
        sources: SourceSet {
            root: None,
            documents: vec![SourceDocument {
                uri: "models.py".into(),
                language_id: "python".into(),
                version: 1,
                text: source.into(),
            }],
        },
        package: CompilePackage {
            name: "acme/example".into(),
            exposed_modules: Some(vec!["Models".into()]),
        },
        dependencies: vec![],
        options: CompileOptions {
            ir_version: "4".into(),
            types_only: true,
            ..Default::default()
        },
        baseline: None,
    }
}

fn compile(source: &str) -> CompileResult {
    PythonExtension.compile(a_request(source)).unwrap()
}

#[test]
fn product_and_sum_have_the_expected_ir_and_roundtrip_through_mep() {
    let extension = NativeExtension::frontend_backend(PythonExtension).unwrap();
    let response = extension.protocol().handle(
        ExtensionRequest::new(
            methods::COMPILE,
            a_request(include_str!("fixtures/models.py")),
            1,
        )
        .unwrap(),
    );
    let compiled: CompileResult = serde_json::from_value(response.result.unwrap()).unwrap();
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    let ir = compiled.ir.unwrap();
    let types = &ir["distribution"]["Library"]["def"]["modules"]["models"]["Public"]["types"];
    assert_eq!(
        types["address"]["Public"]["TypeAliasDefinition"]["typeExp"]["Record"]["fields"]["zip-code"],
        json!("morphir/SDK:basics#int")
    );
    assert_eq!(
        types["decision"]["Public"]["CustomTypeDefinition"]["constructors"]["pending"],
        json!([])
    );
    assert!(
        types.get("pending").is_none(),
        "variants are constructors, not record aliases"
    );
    let response = extension.protocol().handle(
        ExtensionRequest::new(
            methods::GENERATE,
            GenerateRequest {
                ir: ir.clone(),
                target: "python".into(),
                options: Default::default(),
            },
            2,
        )
        .unwrap(),
    );
    let generated: GenerateResult = serde_json::from_value(response.result.unwrap()).unwrap();
    assert!(generated.success, "{:?}", generated.diagnostics);
    assert_eq!(generated.artifacts[0].path, "models.py");
    let recompiled = compile(&generated.artifacts[0].content);
    assert!(recompiled.success, "{:?}", recompiled.diagnostics);
    assert_eq!(recompiled.ir.unwrap(), ir);
}

#[test]
fn rejects_unsupported_or_ambiguous_source_without_partial_ir() {
    for source in [
        "def calculate():\n    return 1\n",
        "from dataclasses import dataclass\n@dataclass\nclass Mutable:\n    x: int\n",
        "from dataclasses import dataclass\n@dataclass(frozen=True)\nclass Default:\n    x: int = 1\n",
        "from dataclasses import dataclass\n@dataclass(frozen=True)\nclass Unknown:\n    x: Missing\n",
        "import os\n",
        "type Generic[T] = T\n",
        "from dataclasses import dataclass\nfrom __future__ import annotations\n",
        "class Broken(:\n",
    ] {
        let result = compile(source);
        assert!(!result.success, "accepted: {source}");
        assert!(result.ir.is_none());
        assert!(!result.diagnostics.is_empty());
    }
}

#[test]
fn advertises_and_enforces_the_initial_contract() {
    let capabilities = PythonExtension::capabilities();
    assert_eq!(capabilities.frontend.unwrap().ir_versions, ["3", "4"]);
    assert_eq!(capabilities.backend.unwrap().targets, ["python"]);
    let mut request = a_request("");
    request.options.ir_version = "2".into();
    assert!(!PythonExtension.compile(request).unwrap().success);
    assert!(
        !PythonExtension
            .generate(GenerateRequest {
                ir: json!({}),
                target: "elm".into(),
                options: Default::default()
            })
            .unwrap()
            .success
    );
}

#[test]
fn accepts_cli_context_but_rejects_required_parse_stage_output() {
    let mut request = a_request(include_str!("fixtures/models.py"));
    request.options.extra = [
        ("outputDir".into(), json!("ignored/output")),
        ("sourceRootUri".into(), json!("file:///project/src")),
        ("emitParseStage".into(), json!(true)),
        ("emitParseStageFatal".into(), json!(false)),
    ]
    .into_iter()
    .collect();
    let result = PythonExtension.compile(request.clone()).unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.diagnostics[0].severity, DiagnosticSeverity::Warning);
    request
        .options
        .extra
        .insert("emitParseStageFatal".into(), json!(true));
    assert!(!PythonExtension.compile(request).unwrap().success);
}

#[test]
fn rejects_name_collisions_shared_variants_and_constructor_references() {
    for source in [
        "@dataclass(frozen=True)\nclass Thing:\n    zip_code: int\n    zipCode: int\n",
        "@dataclass(frozen=True)\nclass A:\n    pass\ntype Choice = A | A\n",
        "@dataclass(frozen=True)\nclass A:\n    pass\ntype Choice = A\ntype Other = A\n",
        "@dataclass(frozen=True)\nclass A:\n    pass\ntype Choice = A\n@dataclass(frozen=True)\nclass B:\n    field: A\n",
        "@dataclass(frozen=True)\nclass FooBar:\n    pass\n@dataclass(frozen=True)\nclass Foo_Bar:\n    pass\n",
        "@dataclass(frozen=True)\nclass A(Base):\n    pass\n",
        "@dataclass(frozen=True)\nclass A:\n    field: __import__('os').system('echo unsafe')\n",
    ] {
        let result = compile(&format!("from dataclasses import dataclass\n{source}"));
        assert!(!result.success, "accepted: {source}");
        assert!(result.ir.is_none());
    }
}

#[test]
fn rejects_names_whose_generated_spelling_is_reserved() {
    for body in [
        "@dataclass(frozen=True)\nclass Product:\n    Class: str\n",
        "@dataclass(frozen=True)\nclass Variant:\n    Int: int\ntype Choice = Variant\n",
        "@dataclass(frozen=True)\nclass none:\n    pass\n",
        "@dataclass(frozen=True)\nclass Variant:\n    pass\ntype true = Variant\n",
    ] {
        let result = compile(&format!("from dataclasses import dataclass\n{body}"));
        assert!(!result.success, "accepted: {body}");
        assert!(result.ir.is_none());
        assert_eq!(result.diagnostics[0].code.as_deref(), Some("PY003"));
    }
}

fn a_library(definition: serde_json::Value) -> serde_json::Value {
    json!({"formatVersion": 4, "distribution": {"Library": {
        "packageName": "acme/example", "dependencies": {}, "def": {"modules": {
            "models": {"Public": {"types": {"choice": {"Public": definition}}, "values": {}}}
        }}
    }}})
}

#[test]
fn backend_accepts_an_independently_authored_sum() {
    let ir = a_library(
        json!({"CustomTypeDefinition": {"typeParams": [], "access": "Public", "constructors": {
            "absent": [], "present": [["value", "morphir/SDK:string#string"]]
        }}}),
    );
    let result = PythonExtension
        .generate(GenerateRequest {
            ir: ir.clone(),
            target: "python".into(),
            options: Default::default(),
        })
        .unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(compile(&result.artifacts[0].content).ir.unwrap(), ir);
}

#[test]
fn backend_refuses_unsupported_ir_without_partial_artifacts() {
    for definition in [
        json!({"TypeAliasDefinition": {"typeParams": ["a"], "typeExp": "a"}}),
        json!({"TypeAliasDefinition": {"typeParams": [], "typeExp": {"Function": {"parameterType": "morphir/SDK:basics#int", "returnType": "morphir/SDK:basics#int"}}}}),
        json!({"TypeAliasDefinition": {"typeParams": [], "typeExp": {"Record": {"fields": {"value": "elsewhere/pkg:other#type"}}}}}),
        json!({"CustomTypeDefinition": {"typeParams": [], "access": "Private", "constructors": {"hidden": []}}}),
        json!({"CustomTypeDefinition": {"typeParams": [], "access": "Public", "constructors": {"choice": []}}}),
        json!({"CustomTypeDefinition": {"typeParams": [], "access": "Public", "constructors": {}}}),
    ] {
        let result = PythonExtension
            .generate(GenerateRequest {
                ir: a_library(definition),
                target: "python".into(),
                options: Default::default(),
            })
            .unwrap();
        assert!(!result.success);
        assert!(result.artifacts.is_empty());
        assert!(!result.diagnostics.is_empty());
    }
}

#[test]
fn recursive_types_and_initialisms_roundtrip() {
    let result = compile(
        "from __future__ import annotations\nfrom dataclasses import dataclass\n@dataclass(frozen=True)\nclass Empty:\n    pass\n@dataclass(frozen=True)\nclass Link:\n    value_in_USD: float\n    rest: Chain\ntype Chain = Empty | Link\n",
    );
    assert!(result.success, "{:?}", result.diagnostics);
    let ir = result.ir.unwrap();
    let generated = PythonExtension
        .generate(GenerateRequest {
            ir: ir.clone(),
            target: "python".into(),
            options: Default::default(),
        })
        .unwrap();
    assert!(generated.success, "{:?}", generated.diagnostics);
    assert_eq!(compile(&generated.artifacts[0].content).ir.unwrap(), ir);
}

#[test]
fn syntax_errors_identify_the_source_document() {
    let result = compile("class Broken(:\n");
    assert_eq!(result.diagnostics[0].code.as_deref(), Some("PY002"));
    assert_eq!(
        result.diagnostics[0].location.as_ref().unwrap().uri,
        "models.py"
    );
}

#[test]
fn refuses_nonportable_output_module_names() {
    for module in [
        "../models",
        "nested/con",
        "dataclasses",
        "con",
        "aux",
        "COM1",
        "lpt9",
    ] {
        let mut ir = a_library(
            json!({"TypeAliasDefinition": {"typeParams": [], "typeExp": {"Record": {"fields": {}}}}}),
        );
        let modules = ir["distribution"]["Library"]["def"]["modules"]
            .as_object_mut()
            .unwrap();
        let definition = modules.remove("models").unwrap();
        modules.insert(module.into(), definition);
        let result = PythonExtension
            .generate(GenerateRequest {
                ir,
                target: "python".into(),
                options: Default::default(),
            })
            .unwrap();
        assert!(!result.success, "accepted module {module}");
        assert!(result.artifacts.is_empty());
    }
}
