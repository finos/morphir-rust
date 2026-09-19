//! Incremental compilation through the public frontend contract.
use morphir_extension_sdk::{
    BaselineModule, CompileBaseline, CompileOptions, CompilePackage, CompileRequest, CompileResult,
    Frontend, ModuleResult, ModuleStatus, SourceDocument,
};
use morphir_gleam_binding::GleamExtension;

const A: &str = "import b\npub type Alias = b.Number\n";
const A_CHANGED: &str = "import b\npub type Alias = List(b.Number)\n";
const B: &str = "pub type Number = Int\n";
const B_BROKEN: &str = "pub type Number =\n";

fn request(a: &str, b: Option<&str>) -> CompileRequest {
    let mut documents = vec![document("a", a)];
    if let Some(b) = b {
        documents.push(document("b", b));
    }
    CompileRequest {
        language_id: "gleam".into(),
        documents,
        package: CompilePackage {
            name: "sample".into(),
            exposed_modules: None,
        },
        options: CompileOptions {
            types_only: false,
            ir_version: "4.0.0".into(),
            extra: [("emitParseStage".into(), false.into())].into(),
        },
        ..Default::default()
    }
}

fn document(name: &str, source: &str) -> SourceDocument {
    SourceDocument {
        uri: format!("file:///work/src/{name}.gleam"),
        language_id: "gleam".into(),
        version: 1,
        text: source.into(),
    }
}

fn compile(request: CompileRequest) -> CompileResult {
    GleamExtension.compile(request).expect("frontend response")
}

fn module<'a>(result: &'a CompileResult, name: &str) -> &'a ModuleResult {
    result
        .module_results
        .iter()
        .find(|module| module.name == name)
        .unwrap_or_else(|| panic!("no result for {name}: {result:?}"))
}

fn baseline(previous: &CompileBaseline, result: &CompileResult) -> CompileBaseline {
    CompileBaseline {
        context_digest: result.context_digest.clone(),
        modules: result
            .module_results
            .iter()
            .filter_map(|module| match module.status {
                ModuleStatus::Compiled => Some(BaselineModule {
                    name: module.name.clone(),
                    uri: module.uri.clone(),
                    source_digest: module.source_digest.clone().expect("source digest"),
                    interface_digest: module.interface_digest.clone().expect("interface digest"),
                    depends_on: module.depends_on.clone(),
                    ir: module.ir.clone().expect("compiled IR"),
                }),
                _ => previous
                    .modules
                    .iter()
                    .find(|entry| entry.name == module.name)
                    .cloned(),
            })
            .collect(),
    }
}

fn first() -> (CompileResult, CompileBaseline) {
    let result = compile(request(A, Some(B)));
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "a").status, ModuleStatus::Compiled);
    assert_eq!(module(&result, "a").depends_on, vec!["b"]);
    let baseline = baseline(&CompileBaseline::default(), &result);
    (result, baseline)
}

fn rerun(a: &str, b: Option<&str>, baseline: CompileBaseline) -> CompileResult {
    compile(CompileRequest {
        baseline: Some(baseline),
        ..request(a, b)
    })
}

#[test]
fn unchanged_sources_reuse_ir_and_report_digests() {
    let (first, baseline) = first();
    let result = rerun(A, Some(B), baseline);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.ir, first.ir);
    assert_eq!(result.context_digest, first.context_digest);
    for name in ["a", "b"] {
        assert_eq!(module(&result, name).status, ModuleStatus::Unchanged);
        assert!(module(&result, name).ir.is_none());
        assert_eq!(
            module(&result, name).source_digest,
            module(&first, name).source_digest
        );
        assert!(
            module(&result, name)
                .source_digest
                .as_ref()
                .unwrap()
                .starts_with("sha256:")
        );
    }
}

#[test]
fn changed_public_type_recompiles_its_dependents() {
    let (first, baseline) = first();
    let result = rerun(A, Some("pub type Number = Float\n"), baseline);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "a").status, ModuleStatus::Compiled);
    assert_eq!(module(&result, "b").status, ModuleStatus::Compiled);
    assert_ne!(
        module(&result, "b").interface_digest,
        module(&first, "b").interface_digest
    );
}

#[test]
fn documentation_change_does_not_recompile_dependents() {
    let (first, baseline) = first();
    let result = rerun(A, Some("/// A number.\npub type Number = Int\n"), baseline);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "b").status, ModuleStatus::Compiled);
    assert_eq!(module(&result, "a").status, ModuleStatus::Unchanged);
    assert_eq!(
        module(&result, "b").interface_digest,
        module(&first, "b").interface_digest
    );
}

#[test]
fn private_type_change_does_not_recompile_dependents() {
    let (first, baseline) = first();
    let result = rerun(
        A,
        Some("pub type Number = Int\ntype Hidden = Float\n"),
        baseline,
    );
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "b").status, ModuleStatus::Compiled);
    assert_eq!(module(&result, "a").status, ModuleStatus::Unchanged);
    assert_eq!(
        module(&result, "b").interface_digest,
        module(&first, "b").interface_digest
    );
}

#[test]
fn broken_dependency_without_baseline_blocks_dependent() {
    let result = compile(request(A, Some(B_BROKEN)));
    assert!(!result.success);
    assert_eq!(module(&result, "b").status, ModuleStatus::Failed);
    assert_eq!(module(&result, "a").status, ModuleStatus::Blocked);
    assert!(
        module(&result, "a")
            .diagnostics
            .iter()
            .any(|d| d.location.is_some())
    );
}

#[test]
fn broken_dependency_reuses_last_good_interface_for_unchanged_dependent() {
    let (_, baseline) = first();
    let result = rerun(A, Some(B_BROKEN), baseline);
    assert!(!result.success);
    assert_eq!(module(&result, "b").status, ModuleStatus::Failed);
    assert_eq!(module(&result, "a").status, ModuleStatus::Unchanged);
    assert_eq!(result.modules, vec!["a"]);
}

#[test]
fn changed_dependent_compiles_against_broken_dependency_last_good_interface() {
    let (_, baseline) = first();
    let result = rerun(A_CHANGED, Some(B_BROKEN), baseline);
    assert!(!result.success);
    assert_eq!(module(&result, "b").status, ModuleStatus::Failed);
    assert_eq!(module(&result, "a").status, ModuleStatus::Compiled);
    assert_eq!(result.modules, vec!["a"]);
}

#[test]
fn repaired_dependency_preserves_dependent_after_a_failed_run() {
    let (_, initial) = first();
    let broken = rerun(A_CHANGED, Some(B_BROKEN), initial.clone());
    let next = baseline(&initial, &broken);
    let result = rerun(A_CHANGED, Some(B), next);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "a").status, ModuleStatus::Unchanged);
}

#[test]
fn deletion_invalidates_unchanged_dependent() {
    let (_, baseline) = first();
    let result = rerun(A, None, baseline);
    assert!(!result.success);
    assert_eq!(module(&result, "a").status, ModuleStatus::Failed);
}

#[test]
fn malformed_baseline_ir_recompiles_affected_modules() {
    let (_, mut baseline) = first();
    baseline
        .modules
        .iter_mut()
        .find(|entry| entry.name == "b")
        .unwrap()
        .ir = serde_json::json!({"invalid": true});
    let result = rerun(A, Some(B), baseline);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "b").status, ModuleStatus::Compiled);
    assert_eq!(module(&result, "a").status, ModuleStatus::Compiled);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("baseline"))
    );
}

#[test]
fn missing_context_digest_prevents_baseline_reuse() {
    let (_, mut baseline) = first();
    baseline.context_digest = None;
    let result = rerun(A, Some(B), baseline);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "a").status, ModuleStatus::Compiled);
    assert_eq!(module(&result, "b").status, ModuleStatus::Compiled);
}

#[test]
fn changed_package_prevents_baseline_reuse() {
    let (first, baseline) = first();
    let mut request = request(A, Some(B));
    request.baseline = Some(baseline);
    request.package.name = "other".into();
    let result = compile(request);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_ne!(result.context_digest, first.context_digest);
    assert_eq!(module(&result, "a").status, ModuleStatus::Compiled);
}

#[test]
fn changed_types_only_mode_prevents_baseline_reuse() {
    let (first, baseline) = first();
    let mut request = request(A, Some(B));
    request.baseline = Some(baseline);
    request.options.types_only = true;
    let result = compile(request);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_ne!(result.context_digest, first.context_digest);
    assert_eq!(module(&result, "a").status, ModuleStatus::Compiled);
}

#[test]
fn changed_module_access_prevents_baseline_reuse() {
    let (first, baseline) = first();
    let mut request = request(A, Some(B));
    request.baseline = Some(baseline);
    request.package.exposed_modules = Some(vec!["a".into()]);
    let result = compile(request);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_ne!(result.context_digest, first.context_digest);
    assert_eq!(module(&result, "b").status, ModuleStatus::Compiled);
}

#[test]
fn changed_ir_version_prevents_baseline_reuse() {
    let (first, baseline) = first();
    let mut request = request(A, Some(B));
    request.baseline = Some(baseline);
    request.options.ir_version = "3".into();
    let result = compile(request);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_ne!(result.context_digest, first.context_digest);
    assert_eq!(module(&result, "a").status, ModuleStatus::Compiled);
}

#[test]
fn nested_module_paths_are_recorded_as_dependencies() {
    let mut request = request("import domain/model\npub type Alias = model.Number\n", None);
    request.documents.push(document("domain/model", B));
    let first = compile(request.clone());
    assert!(first.success, "{:?}", first.diagnostics);
    assert_eq!(module(&first, "a").depends_on, vec!["domain/model"]);
    request.baseline = Some(baseline(&CompileBaseline::default(), &first));
    let result = compile(request);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(
        module(&result, "domain/model").status,
        ModuleStatus::Unchanged
    );
}

#[test]
fn changed_source_recompiles_only_the_changed_module() {
    let (_, baseline) = first();
    let result = rerun(A_CHANGED, Some(B), baseline);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "a").status, ModuleStatus::Compiled);
    assert_eq!(module(&result, "b").status, ModuleStatus::Unchanged);
}

#[test]
fn an_unrelated_module_still_compiles_when_another_fails() {
    let mut request = request(A, Some(B_BROKEN));
    request
        .documents
        .push(document("unrelated", "pub type Name = String\n"));
    let result = compile(request);
    assert!(!result.success);
    assert_eq!(module(&result, "a").status, ModuleStatus::Blocked);
    assert_eq!(module(&result, "unrelated").status, ModuleStatus::Compiled);
    assert_eq!(result.modules, vec!["unrelated"]);
}

#[test]
fn baseline_from_before_structural_value_fixes_recompiles_unchanged_source() {
    use morphir_core::ir::v4::{AccessControlled, Literal, ModuleDefinition, Value, ValueBody};

    // Frozen from 0.3.0 commit 2a04872632a6cbd9f0ccd02dab533b1042adc3e5,
    // incremental-v2-gleam-1.18.1, for this suite's sample/gleam/v4 request.
    // Source text does not participate in the context digest. Do not derive
    // this old context from the current compiler: the revision must invalidate it.
    const PRE_STRUCTURAL_FIX_CONTEXT: &str =
        "sha256:18db626c9393713204c6086f5f9476a138fd3b3fbb0cba9e038eef5cdd8aaa6b";
    const SOURCE: &str = "pub fn prepend(rest: List(Int)) -> List(Int) { [1, ..rest] }";
    let fresh = compile(request(SOURCE, None));
    assert!(fresh.success, "{:?}", fresh.diagnostics);
    let mut old = baseline(&CompileBaseline::default(), &fresh);
    old.context_digest = Some(PRE_STRUCTURAL_FIX_CONTEXT.into());
    let entry = &mut old.modules[0];
    let mut definition: AccessControlled<ModuleDefinition> =
        serde_json::from_value(entry.ir.clone()).unwrap();
    // That release checked the tail for unsupported syntax but discarded it
    // during lowering. The stored public signature and source digest still match.
    definition
        .value
        .values
        .get_mut("prepend")
        .unwrap()
        .value
        .value
        .body = ValueBody::Expression(Value::List(
        Default::default(),
        vec![Value::Literal(Default::default(), Literal::integer(1))],
    ));
    entry.ir = serde_json::to_value(definition).unwrap();

    let result = rerun(SOURCE, None, old);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "a").status, ModuleStatus::Compiled);
    assert_eq!(result.ir, fresh.ir, "the lost list tail must be rebuilt");
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.code.as_deref() == Some("GLEAM_BASELINE")
            && diagnostic.message.contains("different compile context")
    }));
    let rebuilt: AccessControlled<ModuleDefinition> =
        serde_json::from_value(module(&result, "a").ir.clone().unwrap()).unwrap();
    assert!(matches!(
        rebuilt.value.values["prepend"].value.value.body,
        ValueBody::Expression(Value::Apply(..))
    ));
}
