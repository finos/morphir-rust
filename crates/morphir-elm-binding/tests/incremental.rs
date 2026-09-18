//! Stateless incremental compilation: the host keeps the baseline, the
//! extension decides per module whether to reuse it.
//!
//! Every case here runs the same two-module package — `A` imports `B` and
//! aliases one of its types — and threads each run's `moduleResults` back into
//! the next run's baseline, exactly as a daemon would.

use morphir_elm_binding::ElmExtension;
use morphir_extension_sdk::prelude::*;

const A_URI: &str = "file:///work/A.elm";
const B_URI: &str = "file:///work/B.elm";

const A: &str = "module A exposing (T)\n\nimport B\n\ntype alias T = B.U\n";
const A_CHANGED: &str =
    "module A exposing (T, S)\n\nimport B\n\ntype alias T = B.U\n\n\ntype alias S = Int\n";
const B: &str = "module B exposing (U)\n\ntype alias U = Int\n";
const B_BROKEN: &str = "module B exposing (U)\n\ntype alias U =\n";
const B_DOC: &str = "module B exposing (U)\n\n{-| A whole number. -}\ntype alias U = Int\n";
const B_FLOAT: &str = "module B exposing (U)\n\ntype alias U = Float\n";
const B_WIDER: &str = "module B exposing (U, V)\n\ntype alias U = Int\n\n\ntype alias V = String\n";

fn document(uri: &str, text: &str) -> SourceDocument {
    SourceDocument {
        uri: uri.into(),
        language_id: "elm".into(),
        version: 1,
        text: text.into(),
    }
}

fn compile(documents: Vec<SourceDocument>, baseline: Option<CompileBaseline>) -> CompileResult {
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    extension
        .frontend()
        .unwrap()
        .compile(CompileRequest {
            language_id: "elm".into(),
            documents,
            package: CompilePackage {
                name: "local/example".into(),
                exposed_modules: None,
            },
            dependencies: vec![],
            options: CompileOptions {
                types_only: false,
                ir_version: "3".into(),
                extra: Default::default(),
            },
            baseline,
        })
        .unwrap()
}

fn both(a: &str, b: &str) -> Vec<SourceDocument> {
    vec![document(A_URI, a), document(B_URI, b)]
}

/// The baseline a host would hold after `result`: freshly compiled modules
/// replace their entry, modules that were reused or could not be compiled keep
/// the entry they had, and modules that are no longer in the request are
/// dropped.
fn baseline_from(previous: &CompileBaseline, result: &CompileResult) -> CompileBaseline {
    let mut modules: Vec<BaselineModule> = previous
        .modules
        .iter()
        .filter(|entry| {
            result
                .module_results
                .iter()
                .any(|module| module.name == entry.name)
        })
        .cloned()
        .collect();

    for module in &result.module_results {
        if module.status != ModuleStatus::Compiled {
            continue;
        }
        let entry = BaselineModule {
            name: module.name.clone(),
            uri: module.uri.clone(),
            source_digest: module
                .source_digest
                .clone()
                .expect("a compiled module reports its source digest"),
            interface_digest: module
                .interface_digest
                .clone()
                .expect("a compiled module reports its interface digest"),
            depends_on: module.depends_on.clone(),
            ir: module
                .ir
                .clone()
                .expect("a compiled module reports its module IR"),
        };
        match modules.iter_mut().find(|held| held.name == entry.name) {
            Some(held) => *held = entry,
            None => modules.push(entry),
        }
    }

    CompileBaseline { modules }
}

fn module<'a>(result: &'a CompileResult, name: &str) -> &'a ModuleResult {
    result
        .module_results
        .iter()
        .find(|module| module.name == name)
        .unwrap_or_else(|| panic!("no result for module {name} in {:?}", result.module_results))
}

fn has_code(module: &ModuleResult, code: &str) -> bool {
    module
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code.as_deref() == Some(code))
}

/// A good first run of both modules, and the baseline it leaves behind.
fn first_run() -> (CompileResult, CompileBaseline) {
    let result = compile(both(A, B), None);
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "A").status, ModuleStatus::Compiled);
    assert_eq!(module(&result, "B").status, ModuleStatus::Compiled);
    let baseline = baseline_from(&CompileBaseline::default(), &result);
    (result, baseline)
}

fn library_modules(result: &CompileResult) -> Vec<String> {
    result.ir.as_ref().expect("a distribution")["distribution"][3]["modules"]
        .as_array()
        .expect("a v3 package definition with a module list")
        .iter()
        .map(|entry| {
            entry[0]
                .as_array()
                .expect("a module path")
                .iter()
                .map(|segment| {
                    segment
                        .as_array()
                        .expect("a name")
                        .iter()
                        .filter_map(|word| word.as_str())
                        .map(|word| {
                            let mut characters = word.chars();
                            match characters.next() {
                                Some(first) => {
                                    first.to_uppercase().collect::<String>() + characters.as_str()
                                }
                                None => String::new(),
                            }
                        })
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join(".")
        })
        .collect()
}

#[test]
fn case_1_a_changed_dependent_recompiles_against_a_broken_dependency_baseline() {
    let (_, baseline) = first_run();

    let result = compile(both(A_CHANGED, B_BROKEN), Some(baseline));

    assert!(!result.success);
    assert_eq!(module(&result, "A").status, ModuleStatus::Compiled);
    assert_eq!(module(&result, "B").status, ModuleStatus::Failed);
    assert!(has_code(module(&result, "B"), "ELM_SYNTAX"));
    assert!(module(&result, "B").ir.is_none());
    assert_eq!(library_modules(&result), vec!["A".to_string()]);
}

#[test]
fn case_2_fixing_a_dependency_leaves_an_untouched_dependent_unchanged() {
    let (_, baseline) = first_run();
    let broken = compile(both(A_CHANGED, B_BROKEN), Some(baseline.clone()));
    let baseline = baseline_from(&baseline, &broken);

    let result = compile(both(A_CHANGED, B_DOC), Some(baseline));

    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "B").status, ModuleStatus::Compiled);
    assert_eq!(module(&result, "A").status, ModuleStatus::Unchanged);
    assert_eq!(library_modules(&result), vec!["B".to_string(), "A".into()]);
}

/// Retyping an exposed alias changes what the alias *specifies*, so it is an
/// interface change and every dependent is recompiled.
#[test]
fn case_3_changing_a_dependency_interface_recompiles_its_dependents() {
    let (first, baseline) = first_run();

    let result = compile(both(A, B_FLOAT), Some(baseline));

    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "B").status, ModuleStatus::Compiled);
    assert_ne!(
        module(&result, "B").interface_digest,
        module(&first, "B").interface_digest
    );
    assert_eq!(module(&result, "A").status, ModuleStatus::Compiled);
}

/// Exposing a new type is an interface change too, for the same reason.
#[test]
fn widening_a_dependency_interface_recompiles_its_dependents() {
    let (first, baseline) = first_run();

    let result = compile(both(A, B_WIDER), Some(baseline));

    assert!(result.success, "{:?}", result.diagnostics);
    assert_ne!(
        module(&result, "B").interface_digest,
        module(&first, "B").interface_digest
    );
    assert_eq!(module(&result, "A").status, ModuleStatus::Compiled);
}

#[test]
fn case_4_a_documentation_only_change_does_not_recompile_dependents() {
    let (first, baseline) = first_run();

    let result = compile(both(A, B_DOC), Some(baseline));

    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "B").status, ModuleStatus::Compiled);
    assert_ne!(
        module(&result, "B").source_digest,
        module(&first, "B").source_digest
    );
    assert_eq!(
        module(&result, "B").interface_digest,
        module(&first, "B").interface_digest
    );
    assert_eq!(module(&result, "A").status, ModuleStatus::Unchanged);
}

#[test]
fn case_5_a_broken_dependency_without_a_baseline_blocks_its_dependents() {
    let result = compile(both(A, B_BROKEN), None);

    assert!(!result.success);
    assert_eq!(module(&result, "B").status, ModuleStatus::Failed);
    assert_eq!(module(&result, "A").status, ModuleStatus::Blocked);
    assert!(has_code(module(&result, "A"), "ELM_BLOCKED"));
    assert!(
        module(&result, "A")
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.location.is_some())
    );
}

#[test]
fn case_6_a_broken_dependency_with_a_baseline_leaves_dependents_unchanged() {
    let (_, baseline) = first_run();

    let result = compile(both(A, B_BROKEN), Some(baseline));

    assert!(!result.success);
    assert_eq!(module(&result, "B").status, ModuleStatus::Failed);
    assert_eq!(module(&result, "A").status, ModuleStatus::Unchanged);
    assert_eq!(library_modules(&result), vec!["A".to_string()]);
}

/// A baseline entry this version cannot read is no more usable than a deleted
/// one, so it must invalidate its dependents just as a deletion does. Reusing
/// `A` here would hand the host IR that references a module nothing describes.
#[test]
fn an_undecodable_baseline_entry_invalidates_its_dependents() {
    let (_, baseline) = first_run();
    let baseline = CompileBaseline {
        modules: baseline
            .modules
            .into_iter()
            .map(|mut entry| {
                if entry.name == "B" {
                    entry.ir = serde_json::json!({"not": "a module definition"});
                }
                entry
            })
            .collect(),
    };

    let result = compile(vec![document(A_URI, A)], Some(baseline));

    assert!(!result.success);
    assert_eq!(module(&result, "A").status, ModuleStatus::Failed);
    assert!(has_code(module(&result, "A"), "ELM_RESOLVE_NOT_FOUND"));
    assert!(
        result
            .diagnostics
            .iter()
            .any(
                |diagnostic| diagnostic.severity == DiagnosticSeverity::Warning
                    && diagnostic.code.as_deref() == Some("ELM_REQUEST")
                    && diagnostic.message.contains("baseline for module B ignored")
            ),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn case_7_deleting_a_dependency_fails_its_dependents() {
    let (_, baseline) = first_run();

    let result = compile(vec![document(A_URI, A)], Some(baseline));

    assert!(!result.success);
    assert_eq!(result.module_results.len(), 1);
    assert_eq!(module(&result, "A").status, ModuleStatus::Failed);
    assert!(has_code(module(&result, "A"), "ELM_RESOLVE_NOT_FOUND"));
}
