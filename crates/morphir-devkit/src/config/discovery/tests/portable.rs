use super::*;

#[test]
fn native_request_and_snapshot_match_portable_fixture_discovery() {
    let options = ConfigLoadOptions {
        user_override: SourceSelection::Discover,
        ..ConfigLoadOptions::project_only()
    };
    let expected_request = fixture_request();

    let actual_request = build_workspace_discovery_request(&workspace_fixture_root(), &options)
        .expect("native request");
    let actual_snapshot =
        discover_workspace(&workspace_fixture_root(), &options).expect("native discovery");
    let expected_snapshot = morphir_workspace::discover(expected_request.clone())
        .into_result()
        .expect("portable discovery");

    assert_eq!(actual_request, expected_request);
    assert_eq!(actual_snapshot, expected_snapshot);
    assert_eq!(actual_snapshot.projects[0].relative_path.as_str(), ".");
    assert_eq!(actual_snapshot.projects[0].name, "acme/root");
    assert!(
        actual_snapshot
            .projects
            .iter()
            .all(|project| project.relative_path.as_str() != "packages/ignored")
    );
    assert_eq!(
        actual_snapshot
            .projects
            .iter()
            .find(|project| project.relative_path.as_str() == "packages/broken")
            .unwrap()
            .state,
        ProjectState::Error
    );
    assert_eq!(
        actual_snapshot
            .projects
            .iter()
            .filter(|project| project.name == "acme/risk")
            .map(|project| project.relative_path.as_str())
            .collect::<Vec<_>>(),
        ["packages/duplicate", "packages/risk"]
    );
}
