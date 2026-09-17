mod resolution_mothers;

use morphir_package::resolution::{
    ResolutionDiagnostic, ResolutionResult, ViolationRule, resolve_library,
};

fn invalid_violations(input: &str) -> Vec<(String, ViolationRule)> {
    match resolve_library(input).unwrap() {
        ResolutionResult::Rejected(ResolutionDiagnostic::InvalidInput { violations }) => violations
            .into_iter()
            .map(|violation| (violation.pointer().to_owned(), violation.rule()))
            .collect(),
        other => panic!("expected invalid input, got {other:?}"),
    }
}

#[test]
fn missing_catalog_is_not_an_empty_catalog() {
    let input = resolution_mothers::root_with_missing_dependency_catalog();
    let result = resolve_library(&input).unwrap();
    assert!(matches!(
        result,
        morphir_package::resolution::ResolutionResult::Rejected(
            morphir_package::resolution::ResolutionDiagnostic::IncompleteInput { .. }
        )
    ));
}

#[test]
fn malformed_json_precedes_duplicate_keys() {
    let result = resolve_library(r#"{"a":1,"a":"\ud800"}"#).unwrap();
    assert!(matches!(
        result,
        ResolutionResult::Rejected(ResolutionDiagnostic::InvalidInput { ref violations })
            if violations.len() == 1
                && violations[0].pointer() == ""
                && violations[0].rule() == ViolationRule::MalformedJson
    ));
}

#[test]
fn duplicate_decoded_keys_accumulate_in_pointer_order() {
    let result =
        resolve_library(r#"{"z":{"b":1,"b":2},"a/b~c":1,"a\u002fb~c":2,"a":1,"a":2}"#).unwrap();
    let ResolutionResult::Rejected(ResolutionDiagnostic::InvalidInput { violations }) = result
    else {
        panic!("expected invalid input")
    };
    assert_eq!(
        violations
            .iter()
            .map(|violation| (violation.pointer(), violation.rule()))
            .collect::<Vec<_>>(),
        vec![
            ("/a", ViolationRule::DuplicateKey),
            ("/a~1b~0c", ViolationRule::DuplicateKey),
            ("/z/b", ViolationRule::DuplicateKey),
        ]
    );
}

fn violations(input: &str) -> Vec<(String, ViolationRule)> {
    let ResolutionResult::Rejected(ResolutionDiagnostic::InvalidInput { violations }) =
        resolve_library(input).unwrap()
    else {
        panic!("expected invalid input")
    };
    violations
        .iter()
        .map(|violation| (violation.pointer().to_owned(), violation.rule()))
        .collect()
}

#[test]
fn outer_shape_faults_accumulate_before_later_phases() {
    assert_eq!(
        violations(&resolution_mothers::independent_outer_shape_faults()),
        vec![
            ("/catalogs".into(), ViolationRule::InvalidType),
            ("/extra".into(), ViolationRule::UnknownField),
            ("/formatVersion".into(), ViolationRule::InvalidValue),
        ]
    );
}

#[test]
fn release_identity_shape_accumulates_independent_sibling_errors() {
    assert_eq!(
        invalid_violations(&resolution_mothers::invalid_release_identity_siblings()),
        vec![
            (
                "/root/release/packagePath".into(),
                ViolationRule::InvalidType
            ),
            ("/root/release/version".into(), ViolationRule::InvalidType),
        ]
    );
}

#[test]
fn requirement_shape_accumulates_all_independent_sibling_errors() {
    assert_eq!(
        invalid_violations(&resolution_mothers::invalid_requirement_siblings()),
        vec![
            (
                "/root/dependencies/0/irPackageName".into(),
                ViolationRule::InvalidType,
            ),
            (
                "/root/dependencies/0/packagePath".into(),
                ViolationRule::InvalidType,
            ),
            (
                "/root/dependencies/0/versionRange/maximumExclusive".into(),
                ViolationRule::InvalidType,
            ),
            (
                "/root/dependencies/0/versionRange/minimumInclusive".into(),
                ViolationRule::InvalidType,
            ),
        ]
    );
}

#[test]
fn replay_node_shape_accumulates_release_and_metadata_siblings() {
    let result =
        resolve_library(&resolution_mothers::replay_node_with_independent_shape_faults()).unwrap();
    let ResolutionResult::Rejected(ResolutionDiagnostic::InvalidLock { violations }) = result
    else {
        panic!("expected invalid lock")
    };
    assert_eq!(
        violations
            .into_iter()
            .map(|violation| (violation.pointer().to_owned(), violation.rule()))
            .collect::<Vec<_>>(),
        vec![
            (
                "/lock/nodes/0/contentDigest".into(),
                ViolationRule::MissingField,
            ),
            (
                "/lock/nodes/0/irPackageName".into(),
                ViolationRule::MissingField,
            ),
            (
                "/lock/nodes/0/manifestDigest".into(),
                ViolationRule::MissingField,
            ),
            (
                "/lock/nodes/0/release/packagePath".into(),
                ViolationRule::InvalidType,
            ),
            (
                "/lock/nodes/0/release/version".into(),
                ViolationRule::InvalidType,
            ),
        ]
    );
}

#[test]
fn invalid_target_discriminators_still_report_unrelated_unknown_fields() {
    for kind in [
        None,
        Some(serde_json::json!(0)),
        Some(serde_json::json!("nope")),
    ] {
        let violations =
            invalid_violations(&resolution_mothers::update_with_invalid_target_kind(kind));
        assert_eq!(violations.len(), 3);
        assert!(violations.contains(&("/targets/0/extra".into(), ViolationRule::UnknownField)));
        assert!(
            violations.contains(&("/targets/0/packagePath".into(), ViolationRule::InvalidType,))
        );
        assert!(
            violations
                .iter()
                .any(|(pointer, _)| pointer == "/targets/0/kind")
        );
    }
}

#[test]
fn initial_mode_treats_lock_as_unknown() {
    assert_eq!(
        violations(&resolution_mothers::initial_with_lock()),
        vec![("/lock".into(), ViolationRule::UnknownField)]
    );
}

#[test]
fn duplicate_catalog_suppresses_its_semantic_subtree() {
    assert_eq!(
        violations(&resolution_mothers::duplicate_catalog_with_invalid_subtree()),
        vec![(
            "/catalogs/1/packagePath".into(),
            ViolationRule::DuplicateIdentity
        )]
    );
}

#[test]
fn empty_interval_is_rejected_before_completeness() {
    assert_eq!(
        violations(&resolution_mothers::empty_interval()),
        vec![(
            "/root/dependencies/0/versionRange".into(),
            ViolationRule::InvalidInterval
        )]
    );
}

#[test]
fn replay_preserves_the_locked_release_when_newer_metadata_exists() {
    let result = resolve_library(&resolution_mothers::replay_with_newer_release()).unwrap();
    let ResolutionResult::Resolved(graph) = result else {
        panic!("expected a replayed graph")
    };
    assert_eq!(graph.nodes().len(), 2);
    assert_eq!(graph.nodes()[1].release().version().as_str(), "1.2.0");
}

#[test]
fn replay_missing_lock_is_an_invalid_lock() {
    let result = resolve_library(&resolution_mothers::replay_missing_lock()).unwrap();
    assert!(matches!(
        result,
        ResolutionResult::Rejected(ResolutionDiagnostic::InvalidLock { ref violations })
            if violations.len() == 1
                && violations[0].pointer() == "/lock"
                && violations[0].rule() == ViolationRule::MissingField
    ));
}

#[test]
fn replay_checks_selected_metadata_digests() {
    let result = resolve_library(&resolution_mothers::replay_with_digest_mismatch()).unwrap();
    assert!(matches!(
        result,
        ResolutionResult::Rejected(ResolutionDiagnostic::InvalidLock { ref violations })
            if violations.len() == 1
                && violations[0].pointer() == "/lock/nodes/1/contentDigest"
                && violations[0].rule() == ViolationRule::DigestMismatch
    ));
}

fn selected_version<'a>(
    graph: &'a morphir_package::resolution::LockedGraph,
    package_path: &str,
) -> &'a str {
    graph
        .nodes()
        .iter()
        .find(|node| node.release().package_path().as_str() == package_path)
        .unwrap()
        .release()
        .version()
        .as_str()
}

#[test]
fn overlapping_ranges_select_one_release_satisfying_every_consumer() {
    let ResolutionResult::Resolved(graph) =
        resolve_library(&resolution_mothers::overlapping_ranges()).unwrap()
    else {
        panic!("expected a graph")
    };
    assert_eq!(selected_version(&graph, "example.com/pkg/shared"), "2.0.0");
}

#[test]
fn search_backtracks_from_an_infeasible_newer_parent() {
    let ResolutionResult::Resolved(graph) =
        resolve_library(&resolution_mothers::backtracking()).unwrap()
    else {
        panic!("expected a graph")
    };
    assert_eq!(selected_version(&graph, "example.com/pkg/a"), "1.0.0");
}

#[test]
fn stable_versions_compare_unbounded_decimal_components() {
    let ResolutionResult::Resolved(graph) =
        resolve_library(&resolution_mothers::unbounded_versions()).unwrap()
    else {
        panic!("expected a graph")
    };
    assert_eq!(
        selected_version(&graph, "example.com/pkg/a"),
        "9007199254740994.0.0"
    );
}

#[test]
fn update_maximizes_target_freshness_before_preservation() {
    let ResolutionResult::Resolved(graph) =
        resolve_library(&resolution_mothers::target_freshness()).unwrap()
    else {
        panic!("expected an updated graph")
    };
    assert_eq!(selected_version(&graph, "example.com/pkg/a"), "2.0.0");
    assert_eq!(selected_version(&graph, "example.com/pkg/shared"), "2.0.0");
}

#[test]
fn update_minimizes_changed_old_non_targets_after_freshness() {
    let ResolutionResult::Resolved(graph) =
        resolve_library(&resolution_mothers::preservation()).unwrap()
    else {
        panic!("expected an updated graph")
    };
    assert_eq!(selected_version(&graph, "example.com/pkg/shared"), "1.0.0");
}

#[test]
fn consumer_scoped_versions_report_the_missing_capability() {
    let result = resolve_library(&resolution_mothers::capability_failure()).unwrap();
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(value["diagnostic"]["code"], "unsupported-capability");
    assert_eq!(
        value["diagnostic"]["requiredCapabilities"],
        serde_json::json!(["graph-aware-coexistence"])
    );
    assert_eq!(
        value["diagnostic"]["witness"]["nodes"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn relaxed_out_of_scope_pin_reports_scope_conflict() {
    let result = resolve_library(&resolution_mothers::scope_conflict()).unwrap();
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(value["diagnostic"]["code"], "update-scope-conflict");
    assert_eq!(
        value["diagnostic"]["changedPins"],
        serde_json::json!([{
            "kind": "changed",
            "previous": { "packagePath": "example.com/pkg/reporting", "version": "1.0.0" },
            "selected": { "packagePath": "example.com/pkg/reporting", "version": "2.0.0" }
        }])
    );
}

#[test]
fn lock_identity_phase_precedes_topology() {
    let result = resolve_library(&resolution_mothers::replay_with_duplicate_node()).unwrap();
    assert!(matches!(
        result,
        ResolutionResult::Rejected(ResolutionDiagnostic::InvalidLock { ref violations })
            if violations.len() == 1
                && violations[0].pointer() == "/lock/nodes/2/release"
                && violations[0].rule() == ViolationRule::DuplicateIdentity
    ));
}

#[test]
fn duplicate_lock_path_suppresses_later_node_identity_semantics() {
    let result =
        resolve_library(&resolution_mothers::replay_duplicate_path_suppresses_node_semantics())
            .unwrap();
    assert!(matches!(
        result,
        ResolutionResult::Rejected(ResolutionDiagnostic::InvalidLock { ref violations })
            if violations.len() == 1
                && violations[0].pointer() == "/lock/nodes/1/release/packagePath"
                && violations[0].rule() == ViolationRule::DuplicateIdentity
    ));
}

#[test]
fn lock_topology_phase_precedes_selected_metadata() {
    let result = resolve_library(&resolution_mothers::replay_with_root_mismatch()).unwrap();
    assert!(matches!(
        result,
        ResolutionResult::Rejected(ResolutionDiagnostic::InvalidLock { ref violations })
            if violations.len() == 1
                && violations[0].pointer() == "/lock/root"
                && violations[0].rule() == ViolationRule::IdentityMismatch
    ));
}

#[test]
fn topology_reports_every_original_edge_inside_disconnected_cycles() {
    let result = resolve_library(&resolution_mothers::replay_with_disconnected_cycles()).unwrap();
    let ResolutionResult::Rejected(ResolutionDiagnostic::InvalidLock { violations }) = result
    else {
        panic!("expected invalid lock")
    };
    let cycle_pointers = violations
        .iter()
        .filter(|violation| violation.rule() == ViolationRule::Cycle)
        .map(|violation| violation.pointer())
        .collect::<Vec<_>>();
    assert_eq!(
        cycle_pointers,
        vec![
            "/lock/nodes/1/bindings/0/target",
            "/lock/nodes/2/bindings/0/target",
            "/lock/nodes/2/bindings/1/target",
            "/lock/nodes/3/bindings/0/target",
            "/lock/nodes/4/bindings/0/target",
        ]
    );
}

#[test]
fn dense_acyclic_replay_topology_remains_valid() {
    let result = resolve_library(&resolution_mothers::dense_replay_topology()).unwrap();
    let ResolutionResult::Resolved(graph) = result else {
        panic!("expected replayed graph")
    };
    assert_eq!(graph.nodes().len(), 81);
}

#[test]
fn selected_metadata_phase_precedes_lock_metadata() {
    let result = resolve_library(&resolution_mothers::replay_missing_selected_metadata()).unwrap();
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(value["diagnostic"]["code"], "incomplete-input");
    assert_eq!(value["diagnostic"]["missing"][0]["kind"], "release");
}

#[test]
fn target_membership_runs_after_old_lock_validation() {
    let result = resolve_library(&resolution_mothers::update_targeting_root()).unwrap();
    assert!(matches!(
        result,
        ResolutionResult::Rejected(ResolutionDiagnostic::InvalidInput { ref violations })
            if violations.len() == 1
                && violations[0].pointer() == "/targets/0/packagePath"
                && violations[0].rule() == ViolationRule::IdentityMismatch
    ));
}

#[test]
fn search_resource_exhaustion_is_an_execution_error_not_unsatisfiable() {
    let result = resolve_library(&resolution_mothers::search_resource_exhaustion());
    assert!(result.is_err());
}

#[test]
fn dense_pending_edges_do_not_consume_recursive_stack() {
    let result = resolve_library(&resolution_mothers::dense_acyclic_graph()).unwrap();
    let ResolutionResult::Resolved(graph) = result else {
        panic!("expected a supported dense graph")
    };
    assert_eq!(graph.nodes().len(), 21);
}

#[test]
fn dense_state_clone_work_exhaustion_is_an_execution_error() {
    assert!(resolve_library(&resolution_mothers::dense_search_work_exhaustion()).is_err());
}
