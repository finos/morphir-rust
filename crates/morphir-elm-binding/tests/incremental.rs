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
    compile_with(documents, baseline, Vec::new(), None)
}

/// One run of the package, with the dependencies and the prelude the case needs.
fn compile_with(
    documents: Vec<SourceDocument>,
    baseline: Option<CompileBaseline>,
    dependencies: Vec<CompileDependency>,
    prelude: Option<&str>,
) -> CompileResult {
    let mut extra = std::collections::HashMap::new();
    if let Some(prelude) = prelude {
        extra.insert("elmPrelude".to_string(), serde_json::json!(prelude));
    }
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
            dependencies,
            options: CompileOptions {
                types_only: false,
                ir_version: "3".into(),
                extra,
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
/// dropped. The host also stores the context the run reported, which is what
/// scopes the whole baseline to the compilation it came from.
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

    CompileBaseline {
        modules,
        context_digest: result.context_digest.clone(),
    }
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
    let context_digest = baseline.context_digest.clone();
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
        context_digest,
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

// ----------------------------------------------------------------------------
// What a module depends on, and what a baseline was built with
// ----------------------------------------------------------------------------

const AMBIGUOUS_A: &str = "module A exposing (T)\n\nimport B exposing (..)\n\nimport C exposing (..)\n\ntype alias T = X\n";
const AMBIGUOUS_B: &str = "module B exposing (U)\n\ntype alias U = Int\n";
const AMBIGUOUS_B_WITH_X: &str =
    "module B exposing (U, X)\n\ntype alias U = Int\n\n\ntype alias X = Int\n";
const AMBIGUOUS_C: &str = "module C exposing (X)\n\ntype alias X = String\n";

fn three(a: &str, b: &str, c: &str) -> Vec<SourceDocument> {
    vec![
        document(A_URI, a),
        document(B_URI, b),
        document("file:///work/C.elm", c),
    ]
}

/// `dependsOn` is every in-package module a module *imports*, not only the ones
/// whose names it resolved against. A imports B and C, both `exposing (..)`,
/// and uses only C's `X`. Widening B so that it exposes an `X` too makes A's
/// bare `X` ambiguous — a change in a module A never named. Recording only the
/// resolved references would leave A reused, and an incremental run would
/// disagree with a clean one about a package that does not compile.
#[test]
fn an_unused_import_is_still_a_dependency() {
    let clean = compile(three(AMBIGUOUS_A, AMBIGUOUS_B, AMBIGUOUS_C), None);
    assert!(clean.success, "{:?}", clean.diagnostics);
    assert_eq!(
        module(&clean, "A").depends_on,
        vec!["B".to_string(), "C".to_string()],
        "A imports both, and uses only C"
    );
    let baseline = baseline_from(&CompileBaseline::default(), &clean);

    let result = compile(
        three(AMBIGUOUS_A, AMBIGUOUS_B_WITH_X, AMBIGUOUS_C),
        Some(baseline),
    );

    // A's source did not change, so the only thing that can have recompiled it
    // is the dependency it never named. Recompiling is what surfaces the
    // ambiguity, and a module whose references do not resolve is `failed`.
    assert_ne!(module(&result, "A").status, ModuleStatus::Unchanged);
    assert_eq!(module(&result, "A").status, ModuleStatus::Failed);
    assert!(
        has_code(module(&result, "A"), "ELM_RESOLVE_AMBIGUOUS"),
        "{:?}",
        module(&result, "A").diagnostics
    );
    assert!(!result.success);
}

/// A document whose module header is gone no longer says which module it is,
/// but the baseline still recognises its uri. That is enough to report it as
/// the module that failed rather than as a module that was deleted, so its
/// dependents take the ordinary failed-dependency path and resolve against its
/// last good interface.
#[test]
fn a_document_that_lost_its_module_header_fails_as_the_module_the_baseline_names() {
    let (_, baseline) = first_run();

    let headerless = "\ntype alias U = Int\n";
    let result = compile(both(A, headerless), Some(baseline));

    assert!(!result.success);
    assert_eq!(module(&result, "B").status, ModuleStatus::Failed);
    assert!(has_code(module(&result, "B"), "ELM_SYNTAX"));
    assert!(module(&result, "B").source_digest.is_some());
    assert!(module(&result, "B").ir.is_none());
    assert_eq!(module(&result, "B").uri, B_URI);
    assert_eq!(module(&result, "A").status, ModuleStatus::Unchanged);
    assert_eq!(library_modules(&result), vec!["A".to_string()]);
}

/// Without a baseline there is no name to give the document, so it is dropped
/// with its diagnostics as before, and its dependent is left with nothing to
/// resolve against.
#[test]
fn a_headerless_document_the_baseline_does_not_know_is_still_dropped() {
    let result = compile(both(A, "\ntype alias U = Int\n"), None);

    assert!(!result.success);
    assert_eq!(result.module_results.len(), 1);
    assert_eq!(module(&result, "A").status, ModuleStatus::Failed);
    assert!(has_code(module(&result, "A"), "ELM_RESOLVE_NOT_FOUND"));
}

/// Whether a run ignored the baseline, and why it said it did.
fn ignored_baseline_warning(result: &CompileResult) -> Option<&str> {
    result
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            diagnostic.severity == DiagnosticSeverity::Warning
                && diagnostic.code.as_deref() == Some("ELM_REQUEST")
        })
        .map(|diagnostic| diagnostic.message.as_str())
        .find(|message| message.starts_with("baseline ignored:"))
}

/// The context digest a run reports is the one the host stores, so echoing it
/// back reuses the baseline.
#[test]
fn a_baseline_from_the_same_context_is_reused() {
    let (first, baseline) = first_run();
    assert!(
        first.context_digest.is_some(),
        "a validated request reports the context it compiled under"
    );
    assert_eq!(baseline.context_digest, first.context_digest);

    let result = compile(both(A, B), Some(baseline));

    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "A").status, ModuleStatus::Unchanged);
    assert_eq!(module(&result, "B").status, ModuleStatus::Unchanged);
    assert_eq!(ignored_baseline_warning(&result), None);
    assert_eq!(result.context_digest, first.context_digest);
}

/// A baseline that will not say which compilation it came from cannot be shown
/// to describe this one, so it is ignored rather than taken at face value.
#[test]
fn a_baseline_without_a_context_digest_is_ignored() {
    let (_, baseline) = first_run();
    let anonymous = CompileBaseline {
        context_digest: None,
        ..baseline
    };

    let result = compile(both(A, B), Some(anonymous));

    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "A").status, ModuleStatus::Compiled);
    assert_eq!(module(&result, "B").status, ModuleStatus::Compiled);
    assert_eq!(
        ignored_baseline_warning(&result),
        Some("baseline ignored: it carries no contextDigest")
    );
}

/// What a name resolved to last time depends on the prelude, so a run under a
/// different one throws the baseline away whole — and then resolves the very
/// names the old prelude had supplied against nothing.
#[test]
fn a_baseline_from_a_different_prelude_is_ignored() {
    let (first, baseline) = first_run();

    let result = compile_with(both(A, B), Some(baseline), Vec::new(), Some("none"));

    assert_ne!(result.context_digest, first.context_digest);
    assert_eq!(
        ignored_baseline_warning(&result),
        Some("baseline ignored: it was built under a different compile context")
    );
    assert!(!result.success);
    assert_eq!(module(&result, "B").status, ModuleStatus::Failed);
    assert!(
        has_code(module(&result, "B"), "ELM_RESOLVE_NOT_FOUND"),
        "`Int` comes from the prelude, and there is no prelude now: {:?}",
        module(&result, "B").diagnostics
    );
}

/// The words a v3 IR `Path` (a name array) spells, capitalized and joined —
/// the same rendering [`library_modules`] uses for a module path.
fn ir_words(path: &serde_json::Value) -> Vec<String> {
    path.as_array()
        .expect("a package path")
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
        .collect()
}

/// The package path a v3 distribution names, as its words spell it.
fn ir_package_path(result: &CompileResult) -> Vec<String> {
    ir_words(&result.ir.as_ref().expect("a distribution")["distribution"][1])
}

/// The package path is part of the compile context: compiling the very same
/// sources under a renamed package must not reuse baseline IR that still
/// names the old package, since a reused module's FQNames and module keys
/// were resolved under the old one.
#[test]
fn a_baseline_from_a_different_package_is_ignored() {
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    let compile_pkg = |package: &str, baseline: Option<CompileBaseline>| {
        extension
            .frontend()
            .unwrap()
            .compile(CompileRequest {
                language_id: "elm".into(),
                documents: both(A, B),
                package: CompilePackage {
                    name: package.into(),
                    exposed_modules: None,
                },
                dependencies: Vec::new(),
                options: CompileOptions {
                    types_only: false,
                    ir_version: "3".into(),
                    extra: Default::default(),
                },
                baseline,
            })
            .unwrap()
    };

    let first = compile_pkg("my/pkg", None);
    assert!(first.success, "{:?}", first.diagnostics);
    assert_eq!(
        ir_package_path(&first),
        vec!["My".to_string(), "Pkg".to_string()]
    );
    let baseline = baseline_from(&CompileBaseline::default(), &first);

    let result = compile_pkg("other/pkg", Some(baseline));

    assert_ne!(result.context_digest, first.context_digest);
    assert_eq!(
        ignored_baseline_warning(&result),
        Some("baseline ignored: it was built under a different compile context")
    );
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(module(&result, "A").status, ModuleStatus::Compiled);
    assert_eq!(module(&result, "B").status, ModuleStatus::Compiled);
    assert_eq!(
        ir_package_path(&result),
        vec!["Other".to_string(), "Pkg".to_string()]
    );
}

// ----------------------------------------------------------------------------
// A dependency distribution is part of the compile context
// ----------------------------------------------------------------------------

const DEP_URI: &str = "file:///dep/Types.elm";
const DEP_WITH_V: &str =
    "module Types exposing (U, V)\n\ntype alias U = Int\n\n\ntype alias V = String\n";
const DEP_WITHOUT_V: &str = "module Types exposing (U)\n\ntype alias U = Int\n";
const A_ON_DEP: &str =
    "module A exposing (T)\n\nimport Acme.Lib.Types\n\ntype alias T = Acme.Lib.Types.V\n";

/// The dependency package `Acme.Lib`, compiled by this very frontend, so that
/// the two runs differ in nothing but the distribution supplied to them.
fn acme_lib(source: &str) -> CompileDependency {
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    let result = extension
        .frontend()
        .unwrap()
        .compile(CompileRequest {
            language_id: "elm".into(),
            documents: vec![document(DEP_URI, source)],
            package: CompilePackage {
                name: "Acme.Lib".into(),
                exposed_modules: None,
            },
            dependencies: vec![],
            options: CompileOptions {
                types_only: false,
                ir_version: "3".into(),
                extra: Default::default(),
            },
            baseline: None,
        })
        .unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    CompileDependency {
        package_name: "Acme.Lib".into(),
        ir_version: "3".into(),
        distribution: result.ir.expect("a distribution"),
    }
}

/// A module's compiled form depends on the dependency distributions it was
/// resolved against just as much as on its own source. Reusing a module here
/// would hand the host IR naming a type that no longer exists, and the run
/// would call itself a success.
#[test]
fn a_dependency_that_lost_a_type_invalidates_the_whole_baseline() {
    let documents = || vec![document(A_URI, A_ON_DEP), document(B_URI, B)];
    let first = compile_with(documents(), None, vec![acme_lib(DEP_WITH_V)], None);
    assert!(first.success, "{:?}", first.diagnostics);
    let baseline = baseline_from(&CompileBaseline::default(), &first);

    let result = compile_with(
        documents(),
        Some(baseline),
        vec![acme_lib(DEP_WITHOUT_V)],
        None,
    );

    assert_ne!(result.context_digest, first.context_digest);
    assert_eq!(
        ignored_baseline_warning(&result),
        Some("baseline ignored: it was built under a different compile context")
    );
    assert!(!result.success);
    // Nothing in the sources changed, so only the dependency can have done this.
    assert_eq!(module(&result, "B").status, ModuleStatus::Compiled);
    assert_eq!(module(&result, "A").status, ModuleStatus::Failed);
    assert!(
        has_code(module(&result, "A"), "ELM_RESOLVE_NOT_FOUND"),
        "{:?}",
        module(&result, "A").diagnostics
    );
}

/// The same dependency, supplied again, is the same context: reuse still works
/// when a run is handed dependencies at all.
#[test]
fn an_unchanged_dependency_keeps_the_baseline_usable() {
    let documents = || vec![document(A_URI, A_ON_DEP), document(B_URI, B)];
    let first = compile_with(documents(), None, vec![acme_lib(DEP_WITH_V)], None);
    let baseline = baseline_from(&CompileBaseline::default(), &first);

    let result = compile_with(
        documents(),
        Some(baseline),
        vec![acme_lib(DEP_WITH_V)],
        None,
    );

    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(ignored_baseline_warning(&result), None);
    assert_eq!(module(&result, "A").status, ModuleStatus::Unchanged);
    assert_eq!(module(&result, "B").status, ModuleStatus::Unchanged);
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
