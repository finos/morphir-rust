use super::*;
use morphir_config::env_config_value;
use serde_json::json;
#[test]
fn immutable_system_layer_controls_writer_output_without_a_filesystem() {
    let packet: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/current-output-source-cases.json"
    )))
    .unwrap();
    let case = packet["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["id"] == "system-output")
        .unwrap();
    let system =
        morphir_config::parse_config("system.toml", case["setup"]["system"].as_str().unwrap())
            .unwrap();
    let primary = morphir_config::parse_config(
        "morphir.toml",
        case["setup"]["files"]["/work/app/morphir.toml"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    let (config, _) = LayerStack::new()
        .system(Some(FileLayer::new(
            system,
            PathBuf::from("/not-opened/system.toml"),
        )))
        .unwrap()
        .global(None)
        .unwrap()
        .project(Some(FileLayer::new(
            primary,
            PathBuf::from("/not-opened/morphir.toml"),
        )))
        .unwrap();
    let expected = case["expected"]["out"]
        .as_str()
        .unwrap()
        .strip_prefix("/work/app/")
        .unwrap();
    assert_eq!(config.workspace.unwrap().out_dir, expected);
}
#[test]
fn selected_member_is_applied_without_relocating_the_shared_output() {
    let root = morphir_config::parse_config(
        "morphir.toml",
        "[workspace]\nmembers=['orders']\nout_dir='shared'\n",
    )
    .unwrap();
    let member = morphir_config::parse_config(
        "morphir.toml",
        "[project]\nname='acme/orders'\n[workspace]\nout_dir='member/ignored'\n",
    )
    .unwrap();
    let (_, selected) = LayerStack::new()
        .system(None)
        .unwrap()
        .global(None)
        .unwrap()
        .project(Some(FileLayer::new(
            root,
            PathBuf::from("/not-opened/morphir.toml"),
        )))
        .unwrap();
    let merged = selected
        .member(Some(FileLayer::new(
            member,
            PathBuf::from("/not-opened/orders/morphir.toml"),
        )))
        .unwrap()
        .environment(EnvironmentLayer::Skipped)
        .unwrap();
    assert_eq!(merged.value["project"]["name"], "acme/orders");
    assert_eq!(merged.value["workspace"]["out_dir"], "shared");
    assert_eq!(
        merged.notes.out_dir,
        [PathBuf::from("/not-opened/orders/morphir.toml")]
    );
    assert_eq!(
        merged
            .provenance
            .origin(&["workspace".into(), "out_dir".into()])
            .unwrap()
            .path,
        Some(PathBuf::from("/not-opened/morphir.toml"))
    );
}

#[test]
fn explicit_override_plan_cannot_load_an_adjacent_member_override() {
    let explicit = PathBuf::from("/not-opened/override.toml");
    let plan = user_override_plan(
        &SourceSelection::Explicit(explicit.clone()),
        Some(Path::new("/not-opened/morphir.toml")),
        Some(Path::new("/not-opened/member/morphir.toml")),
    );
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0].selection, SourceSelection::Explicit(explicit));
    assert_eq!(plan[0].scope, LayerScope::Workspace);
    assert!(plan[0].candidates.is_empty());
}

#[test]
fn captured_environment_keeps_alias_precedence_and_truthful_origin() {
    let primary = morphir_config::parse_config(
        "morphir.toml",
        "[project]\nname='acme/root'\n[ir]\nlayout='single-file'\n",
    )
    .unwrap();
    let (_, selected) = LayerStack::new()
        .system(None)
        .unwrap()
        .global(None)
        .unwrap()
        .project(Some(FileLayer::new(
            primary,
            PathBuf::from("/not-opened/morphir.toml"),
        )))
        .unwrap();
    let environment = env_config_value("MORPHIR", [("MORPHIR_IR__MODE", "vfs")]);
    let merged = selected
        .member(None)
        .unwrap()
        .environment(EnvironmentLayer::Selected(environment))
        .unwrap();
    assert_eq!(merged.value["ir"]["layout"], "document-tree");
    let origin = merged
        .provenance
        .origin(&["ir".into(), "layout".into()])
        .unwrap();
    assert_eq!(origin.kind, ConfigSourceKind::Environment);
    assert_eq!(origin.path, None);
}

#[test]
fn current_member_predicate_does_not_require_a_live_directory() {
    let selected = Path::new("/not-opened/ws/nested/orders");
    assert!(crate::config::members::member_path_selected(
        Path::new("/not-opened/ws/nested"),
        &["orders".into()],
        &[],
        selected,
        &mut Vec::new()
    ));
    assert!(!crate::config::members::member_path_selected(
        Path::new("/not-opened/ws/nested"),
        &["orders".into()],
        &["orders".into()],
        selected,
        &mut Vec::new()
    ));
}

#[test]
fn root_decode_errors_cannot_be_deferred_until_a_later_layer_repairs_them() {
    let primary = json!({"project":{"name":"acme/root"}, "ir":{"format_version":"invalid"}});
    let root = LayerStack::new()
        .system(None)
        .unwrap()
        .global(None)
        .unwrap()
        .project(Some(FileLayer::new(
            primary,
            PathBuf::from("/not-opened/morphir.toml"),
        )));
    let error = root
        .err()
        .expect("root validation must happen before a member or environment can be supplied");
    assert_eq!(error.to_string(), "Failed to decode project Morphir config");
}

#[test]
fn empty_selected_environment_and_skipped_environment_remain_distinct() {
    fn user_secret() -> LayerStack<AwaitUsers> {
        let (_, selected) = LayerStack::new()
            .system(None)
            .unwrap()
            .global(None)
            .unwrap()
            .project(None)
            .unwrap();
        let mut users = selected.member(None).unwrap();
        users
            .user(
                Some(FileLayer::new(
                    json!({"env":"CONFIG"}),
                    PathBuf::from("/not-opened/morphir.user.toml"),
                )),
                LayerScope::Workspace,
            )
            .unwrap();
        users
    }
    assert_eq!(
        user_secret()
            .environment(EnvironmentLayer::Skipped)
            .unwrap()
            .value,
        json!({"env":"CONFIG"})
    );
    assert_eq!(
        user_secret()
            .environment(EnvironmentLayer::Selected(json!({})))
            .unwrap()
            .value,
        json!({})
    );
}

#[test]
fn adjacent_override_plan_retains_scope_order_and_skipped_reports() {
    let root = Path::new("/not-opened/.morphir/morphir.toml");
    let member = Path::new("/not-opened/orders/.config/morphir/config.yaml");
    let plan = user_override_plan(&SourceSelection::Discover, Some(root), Some(member));
    assert_eq!(plan.len(), 2);
    assert_eq!(plan[0].scope, LayerScope::Workspace);
    assert_eq!(
        plan[0].candidates[0],
        PathBuf::from("/not-opened/.morphir/morphir.user.toml")
    );
    assert_eq!(plan[1].scope, LayerScope::Member);
    assert_eq!(
        plan[1].candidates[0],
        PathBuf::from("/not-opened/orders/.config/morphir/config.user.toml")
    );
    assert_eq!(
        user_override_plan(&SourceSelection::Discover, Some(root), Some(root)).len(),
        1
    );
    for (root, member) in [
        (None, None),
        (Some(Path::new("/not-opened/morphir.json")), None),
    ] {
        let skipped = user_override_plan(&SourceSelection::Discover, root, member);
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0].selection, SourceSelection::Skip);
    }
}
