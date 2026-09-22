use super::*;
use serde_json::Value;
use std::collections::BTreeSet;
#[path = "../../tests/resolution_mothers.rs"]
#[allow(dead_code)]
mod mothers;

fn active_except(input: &Value, path: &str, version: &str) -> BTreeSet<ReleaseId> {
    input["catalogs"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|catalog| catalog["releases"].as_array().unwrap())
        .map(|record| &record["release"])
        .filter(|id| id["packagePath"] != path || id["version"] != version)
        .map(|id| {
            ReleaseId::new(
                PackagePath::parse(id["packagePath"].as_str().unwrap()).unwrap(),
                StableVersion::parse(id["version"].as_str().unwrap()).unwrap(),
            )
        })
        .collect()
}
#[test]
fn status_filter_applies_to_relaxed_and_abstract_diagnostics() {
    let input: Value = serde_json::from_str(&mothers::scope_conflict()).unwrap();
    assert!(matches!(
        resolve_library(&input.to_string()).unwrap(),
        ResolutionResult::Rejected(ResolutionDiagnostic::UpdateScopeConflict { .. })
    ));
    let active = active_except(&input, "example.com/pkg/a", "2.0.0");
    assert!(matches!(
        update_library(&input.to_string(), &active).unwrap(),
        ResolutionResult::Rejected(ResolutionDiagnostic::UnsatisfiableRequirements { .. })
    ));
}
#[test]
fn entire_baseline_is_validated_before_ineligible_old_target_is_filtered() {
    let mut input: Value = serde_json::from_str(&mothers::scope_conflict()).unwrap();
    let active = active_except(&input, "example.com/pkg/a", "1.0.0");
    input["lock"]["nodes"][1]["manifestDigest"] =
        serde_json::json!(format!("sha256:{}", "1".repeat(64)));
    assert!(matches!(
        update_library(&input.to_string(), &active).unwrap(),
        ResolutionResult::Rejected(ResolutionDiagnostic::InvalidLock { .. })
    ));
}
