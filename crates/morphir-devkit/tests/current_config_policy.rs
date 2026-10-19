//! Characterize the existing writer policy from the source-authored packet.
//! Only the legacy-compatible cases listed here are exercised. Scope issuance,
//! currentness and candidate resource policy in the packet remain unimplemented.
use morphir_devkit::config::ProjectSelection;
use morphir_devkit::{
    ConfigLoadOptions, EnvSelection, SourceSelection, discover_config_at, load_config_context_with,
    module_path, resolve_out_root,
};
use serde_json::Value;
use std::path::PathBuf;

struct ExistingWriter {
    _directory: tempfile::TempDir,
    base: PathBuf,
}

impl ExistingWriter {
    fn from_case(case: &Value) -> (Self, ConfigLoadOptions) {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().canonicalize().unwrap();
        let writer = Self {
            _directory: directory,
            base,
        };
        let setup = &case["setup"];
        for (path, text) in setup["files"].as_object().unwrap() {
            writer.write(path, text.as_str().unwrap());
        }
        let source = |role: &str| {
            setup[role].as_str().map_or(SourceSelection::Skip, |text| {
                let path = format!("/mount/{role}.toml");
                writer.write(&path, text);
                SourceSelection::Explicit(writer.path(&path))
            })
        };
        let user_override = match &setup["userOverride"] {
            Value::String(mode) if mode == "discover" => SourceSelection::Discover,
            Value::Object(explicit) => {
                writer.write(
                    "/mount/override.toml",
                    explicit["explicit"].as_str().unwrap(),
                );
                SourceSelection::Explicit(writer.path("/mount/override.toml"))
            }
            _ => SourceSelection::Skip,
        };
        let options = ConfigLoadOptions {
            project: ProjectSelection::Current,
            system: source("system"),
            global: source("global"),
            user_override,
            env: setup["environment"]
                .as_object()
                .map_or(EnvSelection::Skip, |vars| {
                    EnvSelection::Explicit(
                        vars.iter()
                            .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_owned()))
                            .collect(),
                    )
                }),
            ..ConfigLoadOptions::project_only()
        };
        (writer, options)
    }

    fn path(&self, fixture_path: &str) -> PathBuf {
        self.base.join(fixture_path.strip_prefix('/').unwrap())
    }

    fn write(&self, path: &str, contents: &str) {
        let path = self.path(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
}

#[test]
fn source_authored_current_cases_keep_the_actual_writer_root_and_module() {
    let packet: Value =
        serde_json::from_str(include_str!("fixtures/current-output-source-cases.json")).unwrap();
    let selected = [
        "standalone-default",
        "system-output",
        "global-output",
        "primary-output",
        "root-user-output",
        "captured-env-output",
        "member-scoped-relocation",
        "explicit-user-does-not-load-member-user",
        "relative-flag",
        "relative-runtime-env",
        "absolute-flag",
        "empty-runtime-env",
        "ancestor-default-is-not-output-grant",
        "ancestor-output-inside-provider",
        "explicit-output-still-needs-ancestor-module",
        "nearest-matching-nested-workspace",
    ];
    let mut exercised = 0;
    for case in packet["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        if !selected.contains(&id) {
            continue;
        }
        let (writer, options) = ExistingWriter::from_case(case);
        let setup = &case["setup"];
        let provider = writer.path(setup["providerRoot"].as_str().unwrap());
        let project = provider.join(setup["selected"].as_str().unwrap());
        let primary = discover_config_at(&project).unwrap().unwrap();
        let context = load_config_context_with(&primary, &options)
            .unwrap_or_else(|error| panic!("{id}: {error:#}"));
        let flag = setup["flag"].as_str().map(|path| {
            if path.starts_with('/') {
                writer.path(path)
            } else {
                PathBuf::from(path)
            }
        });
        let environment = setup["runtimeEnv"].as_str().map(|path| {
            if path.starts_with('/') {
                writer.path(path).into_os_string()
            } else {
                path.into()
            }
        });
        let cwd = writer.path(setup["startupCwd"].as_str().unwrap_or("/shell"));
        let out = resolve_out_root(
            flag.as_deref(),
            environment.as_deref(),
            Some(&context),
            &cwd,
        );
        let expected = &case["expected"];
        let expected_out = expected["out"]
            .as_str()
            .or_else(|| expected["computedOut"].as_str())
            .unwrap();
        assert_eq!(out, writer.path(expected_out), "{id}");
        assert_eq!(
            module_path(&context),
            PathBuf::from(expected["module"].as_str().unwrap()),
            "{id}"
        );
        if let Some(root) = expected["writerRoot"].as_str() {
            assert_eq!(
                context
                    .workspace_root
                    .as_ref()
                    .or(context.project_root.as_ref()),
                Some(&writer.path(root)),
                "{id}"
            );
        }
        exercised += 1;
    }
    assert_eq!(exercised, selected.len());
}
