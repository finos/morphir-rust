use morphir_extension_sdk::prelude::*;
use morphir_gleam_binding::GleamExtension;
use serde_json::json;

fn request(version: &str) -> CompileRequest {
    CompileRequest {
        language_id: "gleam".into(),
        documents: vec![SourceDocument {
            uri: "file:///src/model.gleam".into(),
            language_id: "gleam".into(),
            version: 1,
            text: "pub type Status { Active Closed }".into(),
        }],
        package: CompilePackage {
            name: "example/model".into(),
            exposed_modules: None,
        },
        dependencies: vec![],
        options: CompileOptions {
            ir_version: version.into(),
            types_only: false,
            extra: [("emitParseStage".into(), json!(false))].into(),
        },
        baseline: None,
    }
}

#[test]
fn compiles_and_generates_both_supported_ir_releases() {
    for version in ["3", "3.0.0", "4", "4.0.0"] {
        let compiled = GleamExtension.compile(request(version)).unwrap();
        assert!(compiled.success, "{version}: {:?}", compiled.diagnostics);
        let ir = compiled.ir.unwrap();
        assert_eq!(
            ir["formatVersion"],
            if version.starts_with('3') {
                json!(3)
            } else {
                json!(4)
            }
        );
        let generated = GleamExtension
            .generate(GenerateRequest {
                ir,
                target: "gleam".into(),
                options: Default::default(),
            })
            .unwrap();
        assert!(generated.success, "{version}: {:?}", generated.diagnostics);
        assert!(generated.artifacts[0].content.contains("pub type Status"));
        assert!(generated.artifacts[0].content.contains("Active"));
    }
}

#[test]
fn types_only_omits_values_with_an_explicit_diagnostic() {
    let mut input = request("4.0.0");
    input.options.types_only = true;
    input.documents[0]
        .text
        .push_str("\npub fn hello() { \"world\" }");
    let compiled = GleamExtension.compile(input).unwrap();
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    assert!(compiled.ir.unwrap()["distribution"]["Library"]["def"]["modules"]["model"]["Public"]["values"].as_object().unwrap().is_empty());
    assert!(
        compiled
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_deref() == Some("GLEAM_VALUE_SKIPPED"))
    );
}

#[test]
fn unsupported_releases_fail_without_ir() {
    for version in ["2", "3.1.0", "4.1.0", "5"] {
        let compiled = GleamExtension.compile(request(version)).unwrap();
        assert!(!compiled.success);
        assert!(compiled.ir.is_none());
    }
}

#[test]
fn dependencies_from_both_ir_versions_resolve_when_compiling_either_version() {
    for dependency_version in ["3", "4"] {
        let mut input = request(dependency_version);
        input.package.name = "example/dependency".into();
        let dependency = GleamExtension.compile(input).unwrap().ir.unwrap();
        for output_version in ["3", "4"] {
            let mut input = request(output_version);
            input.documents[0].uri = "file:///src/consumer.gleam".into();
            input.documents[0].text =
                "import model\npub type Wrapper { Wrapper(model.Status) }".into();
            input.dependencies.push(CompileDependency {
                package_name: "example/dependency".into(),
                ir_version: dependency_version.into(),
                distribution: dependency.clone(),
            });
            let compiled = GleamExtension.compile(input).unwrap();
            assert!(
                compiled.success,
                "dependency {dependency_version}, output {output_version}: {:?}",
                compiled.diagnostics
            );
        }
    }
}
