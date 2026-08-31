use super::*;

#[test]
fn explicit_system_and_global_mounts_accept_supported_extension_variants() {
    let directory = tempfile::tempdir().unwrap();
    let cases = [
        ("system.yml", "system", "morphir.yaml"),
        ("global.YAML", "global user", "morphir.yaml"),
        ("system.TOML", "system", "morphir.toml"),
        ("global.JSON", "global user", "morphir.json"),
    ];

    for (file_name, description, virtual_name) in cases {
        let path = directory.path().join(file_name);
        let contents = format!("contents for {file_name}");
        fs::write(&path, &contents).unwrap();

        let mut payload = PayloadBudget::new(TraversalBudgets::DEFAULT.config_bytes);
        let tree = selected_mount(
            &SourceSelection::Explicit(path),
            Vec::new,
            description,
            &mut payload,
        )
        .unwrap()
        .expect("explicit config should be mounted");

        assert_eq!(
            tree.file_text(&RelativePath::parse(virtual_name).unwrap()),
            Some(contents.as_str()),
            "explicit {description} config {file_name} should be normalized to {virtual_name}"
        );
    }
}

#[test]
fn explicit_mounts_preserve_unsupported_extension_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("morphir.txt");
    fs::write(&path, "not a supported serialization").unwrap();

    let mut payload = PayloadBudget::new(TraversalBudgets::DEFAULT.config_bytes);
    let error = selected_mount(
        &SourceSelection::Explicit(path.clone()),
        Vec::new,
        "system",
        &mut payload,
    )
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        format!(
            "Unsupported system config serialization at {}; expected TOML, YAML, or JSON",
            path.display()
        )
    );
}

#[test]
fn explicit_user_overrides_accept_supported_extension_variants() {
    let directory = tempfile::tempdir().unwrap();
    let cases = [
        (
            "selected.yml",
            "project:\n  version: 2.0.0\n",
            "morphir.user.yaml",
        ),
        (
            "selected.YAML",
            "project:\n  version: 3.0.0\n",
            "morphir.user.yaml",
        ),
        (
            "selected.TOML",
            "[project]\nversion = \"4.0.0\"\n",
            "morphir.user.toml",
        ),
    ];

    for (file_name, contents, virtual_name) in cases {
        let source = directory.path().join(file_name);
        fs::write(&source, contents).unwrap();
        let mut tree = FileTree {
            entries: BTreeMap::from([
                (RelativePath::root(), FileEntry::Directory),
                (
                    RelativePath::parse("morphir.toml").unwrap(),
                    FileEntry::File {
                        text: "[project]\nname = \"acme/root\"\n".to_owned(),
                    },
                ),
            ]),
        };

        let mut payload = payload_for_entries(&tree.entries);
        apply_user_override_selection(&mut tree, &SourceSelection::Explicit(source), &mut payload)
            .unwrap();

        assert_eq!(
            tree.file_text(&RelativePath::parse(virtual_name).unwrap()),
            Some(contents),
            "explicit user override {file_name} should be normalized to {virtual_name}"
        );
    }
}
