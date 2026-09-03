use morphir_common::home::MorphirHome;
use morphir_distribution::{
    CURRENT_RELEASE_SCHEMA_VERSION, Channel, DistributionError, ExtensionId, ExtensionRepositories,
    ExtensionSearchQuery, LocalExtensionRepository, Platform, PublicationStatus,
    RepositoryEndpoint, RepositoryName, RepositoryState, Selection, Sha256Digest,
};
use std::fs;
use std::path::{Path, PathBuf};
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
            "schemaVersion": "1.0",
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

fn release_bundle(
    root: &Path,
    extension_id: &str,
    version: &str,
    artifact_bytes: &[u8],
) -> PathBuf {
    let bundle = root.join(format!("{extension_id}-{version}"));
    fs::create_dir_all(&bundle).unwrap();
    let artifact_name = format!("{extension_id}-extension-{version}.wasm");
    let digest = Sha256Digest::of_bytes(artifact_bytes);
    fs::write(bundle.join(&artifact_name), artifact_bytes).unwrap();
    fs::write(
        bundle.join(format!("{artifact_name}.sha256")),
        format!("{digest}  {artifact_name}\n"),
    )
    .unwrap();
    let descriptor = serde_json::json!({
        "schemaVersion": 1,
        "shortId": extension_id.strip_prefix("morphir-").unwrap_or(extension_id),
        "extensionId": extension_id,
        "package": format!("{extension_id}-extension"),
        "version": version,
        "mepVersions": ["0.1"],
        "runtime": "wasm",
        "targets": [extension_id.strip_prefix("morphir-").unwrap_or(extension_id)],
        "irVersions": ["3", "4"],
        "artifact": artifact_name,
        "sha256": digest,
        "gitCommit": "0123456789abcdef"
    });
    fs::write(
        bundle.join("release.json"),
        format!("{}\n", serde_json::to_string_pretty(&descriptor).unwrap()),
    )
    .unwrap();
    bundle
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
fn add_revalidates_an_endpoint_decoded_without_its_constructor() {
    let mother = RepositoryMother::empty();
    let missing = mother._root.path().join("missing-repository");
    let endpoint: RepositoryEndpoint = serde_json::from_value(serde_json::json!({
        "kind": "local-directory",
        "path": missing
    }))
    .unwrap();

    assert!(matches!(
        mother
            .repositories()
            .add(RepositoryName::parse("unchecked").unwrap(), endpoint),
        Err(DistributionError::Io { .. })
    ));
    assert!(mother.repositories().list().unwrap().is_empty());
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

#[test]
fn local_repository_init_is_idempotent_and_creates_authoring_directories() {
    let root = tempfile::tempdir().unwrap();
    let repository_path = root.path().join("repository");

    let first = LocalExtensionRepository::init(&repository_path).unwrap();
    let second = LocalExtensionRepository::init(&repository_path).unwrap();

    assert_eq!(first.root(), second.root());
    assert!(first.root().join("extensions").is_dir());
    assert!(first.root().join("artifacts").is_dir());
    RepositoryEndpoint::local_directory(first.root()).unwrap();
}

#[test]
fn publishing_a_verified_bundle_is_repeatable_and_writes_valid_metadata() {
    let root = tempfile::tempdir().unwrap();
    let repository = LocalExtensionRepository::init(root.path().join("repository")).unwrap();
    let bundle = release_bundle(root.path(), "morphir-avro", "0.1.0", b"wasm extension");

    let first = repository.publish(&bundle).unwrap();
    let history_before = fs::read(repository.root().join("extensions/morphir-avro.jsonl")).unwrap();
    let second = repository.publish(&bundle).unwrap();

    assert_eq!(first.status(), PublicationStatus::Published);
    assert_eq!(second.status(), PublicationStatus::AlreadyPresent);
    assert_eq!(first.release().extension_id().as_str(), "morphir-avro");
    assert_eq!(first.release().name(), "Morphir Avro");
    assert_eq!(first.release().version().to_string(), "0.1.0");
    let emitted: serde_json::Value = serde_json::from_slice(&history_before).unwrap();
    assert_eq!(
        emitted["schemaVersion"],
        serde_json::to_value(CURRENT_RELEASE_SCHEMA_VERSION).unwrap()
    );
    assert_eq!(fs::read(first.artifact_path()).unwrap(), b"wasm extension");
    assert_eq!(
        fs::read(repository.root().join("extensions/morphir-avro.jsonl")).unwrap(),
        history_before
    );
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let report = ExtensionRepositories::new(&home);
    let endpoint = RepositoryEndpoint::local_directory(repository.root()).unwrap();
    let name = RepositoryName::parse("local-dev").unwrap();
    report.add(name.clone(), endpoint).unwrap();
    assert_eq!(report.verify(&name).unwrap().release_count(), 1);
}

#[test]
fn publication_records_the_declared_display_name() {
    let root = tempfile::tempdir().unwrap();
    let repository = LocalExtensionRepository::init(root.path().join("repository")).unwrap();
    let bundle = release_bundle(root.path(), "morphir-openapi", "0.1.0", b"wasm extension");
    let descriptor_path = bundle.join("release.json");
    let mut descriptor: serde_json::Value =
        serde_json::from_slice(&fs::read(&descriptor_path).unwrap()).unwrap();
    descriptor["name"] = serde_json::json!("Morphir OpenAPI");
    fs::write(
        &descriptor_path,
        serde_json::to_vec_pretty(&descriptor).unwrap(),
    )
    .unwrap();

    let publication = repository.publish(&bundle).unwrap();
    assert_eq!(publication.release().name(), "Morphir OpenAPI");

    descriptor["name"] = serde_json::json!(" ");
    fs::write(
        &descriptor_path,
        serde_json::to_vec_pretty(&descriptor).unwrap(),
    )
    .unwrap();
    let other = LocalExtensionRepository::init(root.path().join("other")).unwrap();
    let error = other.publish(&bundle).unwrap_err();
    assert!(
        error.to_string().contains("name must be non-empty"),
        "{error}"
    );
}

#[test]
fn prerelease_publication_uses_the_resolvable_preview_channel() {
    let root = tempfile::tempdir().unwrap();
    let repository = LocalExtensionRepository::init(root.path().join("repository")).unwrap();
    let bundle = release_bundle(
        root.path(),
        "morphir-avro",
        "1.0.0-rc.1",
        b"preview wasm extension",
    );

    let publication = repository.publish(bundle).unwrap();

    assert_eq!(publication.release().channels(), &[Channel::Preview(None)]);
}

#[test]
fn publication_rejects_tampered_and_unsafe_bundles_without_metadata_writes() {
    let root = tempfile::tempdir().unwrap();
    let repository = LocalExtensionRepository::init(root.path().join("repository")).unwrap();
    let tampered = release_bundle(root.path(), "morphir-avro", "0.1.0", b"original");
    fs::write(
        tampered.join("morphir-avro-extension-0.1.0.wasm"),
        b"tampered",
    )
    .unwrap();

    assert!(matches!(
        repository.publish(&tampered),
        Err(DistributionError::DigestMismatch { .. })
    ));
    assert!(
        fs::read_dir(repository.root().join("extensions"))
            .unwrap()
            .next()
            .is_none()
    );

    let unsafe_bundle = release_bundle(root.path(), "morphir-sql", "0.1.0", b"sql");
    let descriptor_path = unsafe_bundle.join("release.json");
    let mut descriptor: serde_json::Value =
        serde_json::from_slice(&fs::read(&descriptor_path).unwrap()).unwrap();
    descriptor["artifact"] = serde_json::json!("../escape.wasm");
    fs::write(
        &descriptor_path,
        format!("{}\n", serde_json::to_string_pretty(&descriptor).unwrap()),
    )
    .unwrap();

    assert!(matches!(
        repository.publish(&unsafe_bundle),
        Err(DistributionError::InvalidReleaseBundle { .. })
    ));
    assert!(!root.path().join("escape.wasm").exists());
    assert!(
        !repository
            .root()
            .join("extensions/morphir-sql.jsonl")
            .exists()
    );
}

#[test]
fn publication_rejects_a_malformed_release_descriptor() {
    let root = tempfile::tempdir().unwrap();
    let repository = LocalExtensionRepository::init(root.path().join("repository")).unwrap();
    let bundle = release_bundle(root.path(), "morphir-avro", "0.1.0", b"wasm");
    fs::write(bundle.join("release.json"), b"{ not json").unwrap();

    assert!(matches!(
        repository.publish(bundle),
        Err(DistributionError::InvalidReleaseBundle { .. })
    ));
    assert!(
        fs::read_dir(repository.root().join("extensions"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[test]
fn conflicting_publication_preserves_the_existing_artifact_and_history() {
    let root = tempfile::tempdir().unwrap();
    let repository = LocalExtensionRepository::init(root.path().join("repository")).unwrap();
    let first = release_bundle(root.path(), "morphir-avro", "0.1.0", b"first");
    repository.publish(&first).unwrap();
    let history_path = repository.root().join("extensions/morphir-avro.jsonl");
    let artifact_path = repository
        .root()
        .join("artifacts/morphir-avro-extension-0.1.0.wasm");
    let history_before = fs::read(&history_path).unwrap();
    let conflicting = root.path().join("conflicting");
    fs::rename(
        release_bundle(root.path(), "morphir-avro", "0.1.0", b"second"),
        &conflicting,
    )
    .unwrap();

    assert!(matches!(
        repository.publish(&conflicting),
        Err(DistributionError::RepositoryReleaseConflict { .. })
    ));
    assert_eq!(fs::read(history_path).unwrap(), history_before);
    assert_eq!(fs::read(artifact_path).unwrap(), b"first");
}

#[test]
fn search_returns_repository_qualified_matches_from_enabled_local_endpoints() {
    let root = tempfile::tempdir().unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let repositories = ExtensionRepositories::new(&home);
    for (repository_name, extension_id) in [("alpha", "morphir-avro"), ("beta", "morphir-sql")] {
        let repository = LocalExtensionRepository::init(root.path().join(repository_name)).unwrap();
        let bundle = release_bundle(root.path(), extension_id, "0.1.0", extension_id.as_bytes());
        repository.publish(bundle).unwrap();
        repositories
            .add(
                RepositoryName::parse(repository_name).unwrap(),
                RepositoryEndpoint::local_directory(repository.root()).unwrap(),
            )
            .unwrap();
    }

    let all = repositories
        .search(&ExtensionSearchQuery::parse("morphir").unwrap())
        .unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].repository().name().as_str(), "alpha");
    assert_eq!(all[0].release().extension_id().as_str(), "morphir-avro");
    assert_eq!(all[1].repository().name().as_str(), "beta");
    assert_eq!(all[1].release().extension_id().as_str(), "morphir-sql");

    let text = repositories
        .search(&ExtensionSearchQuery::parse("AvRo").unwrap())
        .unwrap();
    assert_eq!(text.len(), 1);
    assert_eq!(text[0].repository().name().as_str(), "alpha");

    repositories
        .disable(&RepositoryName::parse("alpha").unwrap())
        .unwrap();
    assert!(
        repositories
            .search(&ExtensionSearchQuery::parse("morphir-avro").unwrap())
            .unwrap()
            .is_empty()
    );
}
