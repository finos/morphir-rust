use morphir_common::home::MorphirHome;
use morphir_distribution::{
    Channel, DistributionError, ExtensionId, ExtensionRepositories, Platform, RepositoryEndpoint,
    RepositoryName, RepositoryState, Selection, Sha256Digest,
};
use std::fs;
use std::sync::{Arc, Barrier};
use std::thread;
use tempfile::TempDir;

struct RepositoryMother {
    _root: TempDir,
    home: MorphirHome,
    endpoint: std::path::PathBuf,
}

impl RepositoryMother {
    fn empty() -> Self {
        let root = tempfile::tempdir().unwrap();
        let endpoint = root.path().join("repository");
        fs::create_dir_all(endpoint.join("extensions")).unwrap();
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        Self {
            _root: root,
            home,
            endpoint,
        }
    }

    fn with_release() -> Self {
        let mother = Self::empty();
        let artifact = mother.endpoint.join("artifacts/morphir-avro.wasm");
        fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        fs::write(&artifact, b"wasm extension").unwrap();
        let digest = Sha256Digest::of_bytes(b"wasm extension");
        let record = serde_json::json!({
            "schemaVersion": 2,
            "id": "morphir-avro",
            "name": "Morphir Avro",
            "version": "0.1.0",
            "channels": ["stable"],
            "mepVersions": ["0.1"],
            "capabilities": ["backend"],
            "backend": {
                "targets": ["avro"],
                "irVersions": ["4"],
                "generate": false
            },
            "artifacts": [{
                "runtime": "wasm",
                "source": { "kind": "local-file", "path": "artifacts/morphir-avro.wasm" },
                "sha256": digest,
                "filename": "morphir-avro.wasm"
            }]
        });
        fs::write(
            mother.endpoint.join("extensions/morphir-avro.jsonl"),
            format!("{record}\n"),
        )
        .unwrap();
        mother
    }

    fn repositories(&self) -> ExtensionRepositories<'_> {
        ExtensionRepositories::new(&self.home)
    }
}

#[test]
fn named_local_repository_round_trips_through_morphir_home() {
    let mother = RepositoryMother::empty();
    let name = RepositoryName::parse("local-dev").unwrap();
    let endpoint = RepositoryEndpoint::local_directory(&mother.endpoint).unwrap();

    let added = mother.repositories().add(name.clone(), endpoint).unwrap();

    assert_eq!(added.name(), &name);
    assert_eq!(added.state(), RepositoryState::Enabled);
    assert_eq!(
        added.endpoint().local_directory_path(),
        Some(fs::canonicalize(&mother.endpoint).unwrap().as_path())
    );
    assert!(mother.home.extension_repositories_file().is_file());

    let listed = mother.repositories().list().unwrap();
    assert_eq!(listed, vec![added]);
}

#[test]
fn repository_names_are_portable_tokens() {
    for invalid in ["", "Local", "-local", "local-", "../local", "local/repo"] {
        assert!(
            matches!(
                RepositoryName::parse(invalid),
                Err(DistributionError::InvalidValue {
                    kind: "repository name",
                    ..
                })
            ),
            "{invalid:?} should be rejected"
        );
    }
    assert_eq!(
        RepositoryName::parse("team-local").unwrap().as_str(),
        "team-local"
    );
}

#[test]
fn local_endpoint_requires_a_repository_metadata_directory() {
    let root = tempfile::tempdir().unwrap();

    assert!(matches!(
        RepositoryEndpoint::local_directory(root.path()),
        Err(DistributionError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::InvalidInput
    ));
}

#[test]
fn unsupported_repository_state_schema_is_reported_without_rewriting_it() {
    let mother = RepositoryMother::empty();
    let path = mother.home.extension_repositories_file();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let original = b"{\"schemaVersion\":999,\"repositories\":[]}\n";
    fs::write(&path, original).unwrap();

    assert!(matches!(
        mother.repositories().list(),
        Err(DistributionError::UnsupportedStateSchema {
            kind: "extension repositories",
            version: 999
        })
    ));
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn add_rejects_duplicate_names_without_replacing_configuration() {
    let mother = RepositoryMother::empty();
    let name = RepositoryName::parse("local-dev").unwrap();
    let endpoint = RepositoryEndpoint::local_directory(&mother.endpoint).unwrap();
    mother
        .repositories()
        .add(name.clone(), endpoint.clone())
        .unwrap();

    assert!(matches!(
        mother.repositories().add(name.clone(), endpoint),
        Err(DistributionError::RepositoryAlreadyExists { name: actual }) if actual == name
    ));
    assert_eq!(mother.repositories().list().unwrap().len(), 1);
}

#[test]
fn lifecycle_changes_are_persistent_and_missing_names_are_safe() {
    let mother = RepositoryMother::empty();
    let name = RepositoryName::parse("local-dev").unwrap();
    let missing = RepositoryName::parse("missing").unwrap();
    let endpoint = RepositoryEndpoint::local_directory(&mother.endpoint).unwrap();
    mother.repositories().add(name.clone(), endpoint).unwrap();

    let disabled = mother.repositories().disable(&name).unwrap();
    assert_eq!(disabled.state(), RepositoryState::Disabled);
    assert_eq!(
        mother.repositories().get(&name).unwrap().state(),
        RepositoryState::Disabled
    );
    let enabled = mother.repositories().enable(&name).unwrap();
    assert_eq!(enabled.state(), RepositoryState::Enabled);
    assert_eq!(mother.repositories().remove(&name).unwrap().name(), &name);
    assert!(mother.repositories().list().unwrap().is_empty());
    assert!(matches!(
        mother.repositories().remove(&missing),
        Err(DistributionError::RepositoryNotFound { name: actual }) if actual == missing
    ));
}

#[test]
fn verification_and_resolution_use_the_configured_repository() {
    let mother = RepositoryMother::with_release();
    let name = RepositoryName::parse("local-dev").unwrap();
    let endpoint = RepositoryEndpoint::local_directory(&mother.endpoint).unwrap();
    mother.repositories().add(name.clone(), endpoint).unwrap();

    let report = mother.repositories().verify(&name).unwrap();
    assert_eq!(report.history_count(), 1);
    assert_eq!(report.release_count(), 1);

    let selected = mother
        .repositories()
        .resolve(
            &name,
            &ExtensionId::parse("morphir-avro").unwrap(),
            Selection::Channel(Channel::Stable),
            &Platform::new("linux", "x86_64").unwrap(),
        )
        .unwrap();
    assert_eq!(selected.release().version().to_string(), "0.1.0");
}

#[test]
fn disabled_repository_cannot_resolve_extensions() {
    let mother = RepositoryMother::with_release();
    let name = RepositoryName::parse("local-dev").unwrap();
    let endpoint = RepositoryEndpoint::local_directory(&mother.endpoint).unwrap();
    mother.repositories().add(name.clone(), endpoint).unwrap();
    mother.repositories().disable(&name).unwrap();

    assert!(matches!(
        mother.repositories().resolve(
            &name,
            &ExtensionId::parse("morphir-avro").unwrap(),
            Selection::Channel(Channel::Stable),
            &Platform::new("linux", "x86_64").unwrap(),
        ),
        Err(DistributionError::RepositoryDisabled { name: actual }) if actual == name
    ));
}

#[test]
fn verification_rejects_history_whose_filename_disagrees_with_its_identity() {
    let mother = RepositoryMother::with_release();
    fs::rename(
        mother.endpoint.join("extensions/morphir-avro.jsonl"),
        mother.endpoint.join("extensions/not-avro.jsonl"),
    )
    .unwrap();
    let name = RepositoryName::parse("local-dev").unwrap();
    let endpoint = RepositoryEndpoint::local_directory(&mother.endpoint).unwrap();
    mother.repositories().add(name.clone(), endpoint).unwrap();

    assert!(matches!(
        mother.repositories().verify(&name),
        Err(DistributionError::RepositoryHistoryIdentity { .. })
    ));
}

#[test]
fn concurrent_adds_preserve_both_repository_configurations() {
    let mother = RepositoryMother::empty();
    let home = Arc::new(mother.home.clone());
    let endpoint = RepositoryEndpoint::local_directory(&mother.endpoint).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let handles = ["first", "second"].map(|name| {
        let home = Arc::clone(&home);
        let endpoint = endpoint.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            ExtensionRepositories::new(&home)
                .add(RepositoryName::parse(name).unwrap(), endpoint)
                .unwrap();
        })
    });

    barrier.wait();
    for handle in handles {
        handle.join().unwrap();
    }

    let names = ExtensionRepositories::new(&home)
        .list()
        .unwrap()
        .into_iter()
        .map(|repository| repository.name().to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, ["first", "second"]);
}
