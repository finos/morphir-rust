use morphir_devkit::config::{ConfigLoadOptions, ProjectSelection, load_config_context_with};
use std::fs;

#[test]
fn selection_uses_member_name_or_declared_path_before_merging_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("morphir.toml"), "[workspace]\nmembers = ['packages/*']\ndefault_member = 'packages/first'\n[frontend]\nlanguage = 'python'\n").unwrap();
    for name in ["first", "second"] {
        let member = root.join("packages").join(name);
        fs::create_dir_all(&member).unwrap();
        fs::create_dir(member.join(".morphir")).unwrap();
        fs::write(
            member.join("morphir.toml"),
            format!(
                "[project]\nversion = '1.0.0'\nname = 'acme/{name}'\nsource_directory = 'models'\n"
            ),
        )
        .unwrap();
    }
    for selector in ["acme/second", "packages/second"] {
        let options = ConfigLoadOptions {
            project: ProjectSelection::Explicit(selector.into()),
            ..ConfigLoadOptions::project_only()
        };
        // The same explicit selection works from the workspace or a sibling.
        for config in [
            root.join("morphir.toml"),
            root.join("packages/first/morphir.toml"),
        ] {
            let context = load_config_context_with(&config, &options).unwrap();
            assert_eq!(context.current_project.unwrap().name, "acme/second");
            assert_eq!(context.project_root.unwrap(), root.join("packages/second"));
            assert_eq!(context.morphir_dir, root.join("packages/second/.morphir"));
            assert_eq!(
                context.config.frontend.unwrap().language.as_deref(),
                Some("python")
            );
        }
    }
    let options = ConfigLoadOptions {
        project: ProjectSelection::Explicit("missing".into()),
        ..ConfigLoadOptions::project_only()
    };
    assert!(
        load_config_context_with(&root.join("morphir.toml"), &options)
            .unwrap_err()
            .to_string()
            .contains("Unknown project")
    );
}

#[test]
fn root_selection_and_ambiguous_names_do_not_silently_choose_a_default() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(root.join("morphir.toml"), "[workspace]\nmembers = ['a', 'b']\ndefault_member = 'a'\n[project]\nversion = '1.0.0'\nname = 'root'\n").unwrap();
    for member in ["a", "b"] {
        fs::create_dir(root.join(member)).unwrap();
        fs::write(
            root.join(member).join("morphir.toml"),
            "[project]\nversion = '1.0.0'\nname = 'duplicate'\n",
        )
        .unwrap();
    }
    let options = ConfigLoadOptions {
        project: ProjectSelection::Explicit(".".into()),
        ..ConfigLoadOptions::project_only()
    };
    let context = load_config_context_with(&root.join("morphir.toml"), &options).unwrap();
    assert_eq!(context.current_project.unwrap().name, "root");
    assert_eq!(context.project_root.unwrap(), root);
    let options = ConfigLoadOptions {
        project: ProjectSelection::Explicit("duplicate".into()),
        ..ConfigLoadOptions::project_only()
    };
    assert!(
        load_config_context_with(&root.join("morphir.toml"), &options)
            .unwrap_err()
            .to_string()
            .contains("Ambiguous project")
    );
}

#[test]
fn declared_path_selection_does_not_decode_unrelated_members() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::write(
        root.join("morphir.toml"),
        "[workspace]\nmembers = ['a', 'b']\n",
    )
    .unwrap();
    for member in ["a", "b"] {
        fs::create_dir(root.join(member)).unwrap();
    }
    fs::write(root.join("a/morphir.toml"), "not valid TOML!").unwrap();
    fs::write(root.join("b/morphir.toml"), "[project]\nname = 'b'\n").unwrap();
    let options = ConfigLoadOptions {
        project: ProjectSelection::Explicit("b".into()),
        ..ConfigLoadOptions::project_only()
    };
    let context = load_config_context_with(&root.join("morphir.toml"), &options).unwrap();
    assert_eq!(context.current_project.unwrap().version, "0.1.0");
}

#[test]
fn a_project_can_configure_output_without_declaring_workspace_members() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("morphir.toml");
    fs::write(
        &config,
        "[project]\nname = 'acme/app'\n[workspace]\nout_dir = 'build/out'\n",
    )
    .unwrap();
    let context = load_config_context_with(&config, &ConfigLoadOptions::project_only()).unwrap();
    assert_eq!(context.project_root.as_deref(), Some(temp.path()));
}
