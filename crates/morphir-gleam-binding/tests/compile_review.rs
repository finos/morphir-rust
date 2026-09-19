use morphir_extension_sdk::prelude::*;
use morphir_gleam_binding::GleamExtension;
use std::collections::HashMap;

fn request(version: &str, modules: &[(&str, &str)]) -> CompileRequest {
    CompileRequest {
        language_id: "gleam".into(),
        documents: modules
            .iter()
            .map(|(name, source)| SourceDocument {
                uri: format!("file:///workspace/src/{name}.gleam"),
                language_id: "gleam".into(),
                version: 1,
                text: (*source).into(),
            })
            .collect(),
        package: CompilePackage {
            name: "example/package".into(),
            exposed_modules: None,
        },
        dependencies: vec![],
        baseline: None,
        options: CompileOptions {
            types_only: false,
            ir_version: version.into(),
            extra: HashMap::from([("emitParseStage".into(), serde_json::json!(false))]),
        },
    }
}

fn baseline(result: &CompileResult) -> CompileBaseline {
    CompileBaseline {
        context_digest: result.context_digest.clone(),
        modules: result
            .module_results
            .iter()
            .map(|module| BaselineModule {
                name: module.name.clone(),
                uri: module.uri.clone(),
                source_digest: module.source_digest.clone().unwrap(),
                interface_digest: module.interface_digest.clone().unwrap(),
                depends_on: module.depends_on.clone(),
                ir: module.ir.clone().unwrap(),
            })
            .collect(),
    }
}

#[test]
fn edited_baseline_imports_cannot_reuse_a_now_invalid_reference() {
    let first = GleamExtension
        .compile(request(
            "4.0.0",
            &[
                ("a", "pub type A = Int"),
                ("b", "import a\npub type B = a.A"),
            ],
        ))
        .unwrap();
    assert!(first.success, "{:?}", first.diagnostics);
    let mut cached = baseline(&first);
    cached
        .modules
        .iter_mut()
        .find(|module| module.name == "b")
        .unwrap()
        .depends_on
        .clear();
    let mut next = request(
        "4.0.0",
        &[
            ("a", "pub type Other = Int"),
            ("b", "import a\npub type B = a.A"),
        ],
    );
    next.baseline = Some(cached);
    let second = GleamExtension.compile(next).unwrap();
    assert!(!second.success, "a removed type must be resolved again");
    assert_eq!(
        second
            .module_results
            .iter()
            .find(|module| module.name == "b")
            .unwrap()
            .status,
        ModuleStatus::Failed
    );
}

#[test]
fn equivalent_ir_version_spellings_reuse_the_same_context() {
    let first = GleamExtension
        .compile(request("4", &[("a", "pub type A = Int")]))
        .unwrap();
    assert!(first.success, "{:?}", first.diagnostics);
    let mut next = request("4.0.0", &[("a", "pub type A = Int")]);
    next.baseline = Some(baseline(&first));
    let second = GleamExtension.compile(next).unwrap();
    assert!(second.success, "{:?}", second.diagnostics);
    assert_eq!(second.context_digest, first.context_digest);
    assert_eq!(second.module_results[0].status, ModuleStatus::Unchanged);
}

#[test]
fn private_modules_remain_importable_inside_the_package() {
    let mut compile = request(
        "4.0.0",
        &[
            ("a", "pub type A = Int"),
            ("b", "import a\npub type B = a.A"),
        ],
    );
    compile.package.exposed_modules = Some(vec!["b".into()]);
    let result = GleamExtension.compile(compile).unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
}

#[test]
fn polymorphic_function_annotations_survive_type_resolution() {
    let result = GleamExtension
        .compile(request(
            "4.0.0",
            &[("identity", "pub fn identity(value: a) -> a { value }")],
        ))
        .unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.module_results[0].status, ModuleStatus::Compiled);
}
