use morphir_common::home::MorphirHome;
use morphir_distribution::{
    ArtifactStore, Channel, ExtensionId, ExtensionInstaller, InstalledCatalog, LocalIndex,
    Platform, Selection, Sha256Digest, activate_installed, read_extension_lock,
    write_extension_lock,
};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

struct DistributionMother {
    root: TempDir,
    index: std::path::PathBuf,
    home: MorphirHome,
    id: ExtensionId,
    digest: Sha256Digest,
}

impl DistributionMother {
    fn a_local_process_artifact() -> Self {
        let root = tempfile::tempdir().unwrap();
        let index = root.path().join("index");
        let source = index.join("artifacts/morphir-elm");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(index.join("extensions")).unwrap();
        fs::write(&source, b"#!/bin/sh\necho morphir\n").unwrap();
        let digest = Sha256Digest::of_bytes(&fs::read(&source).unwrap());
        let record = serde_json::json!({
            "schemaVersion": 1,
            "id": "morphir-elm",
            "name": "Morphir Elm",
            "version": "3.2.1",
            "channels": ["stable"],
            "mepVersions": ["0.1"],
            "capabilities": ["frontend"],
            "artifacts": [{
                "runtime": "process",
                "platform": { "os": "linux", "arch": "x86_64" },
                "source": { "kind": "local-file", "path": "artifacts/morphir-elm" },
                "sha256": digest,
                "filename": "morphir-elm",
                "args": ["serve"],
                "executable": true
            }]
        });
        fs::write(
            index.join("extensions/morphir-elm.jsonl"),
            format!("{record}\n"),
        )
        .unwrap();
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        Self {
            root,
            index,
            home,
            id: ExtensionId::parse("morphir-elm").unwrap(),
            digest,
        }
    }

    fn selected(&self) -> morphir_distribution::ResolvedArtifact {
        LocalIndex::open(&self.index)
            .unwrap()
            .resolve(
                &self.id,
                Selection::Channel(Channel::Stable),
                &Platform::new("linux", "x86_64").unwrap(),
            )
            .unwrap()
    }

    fn declare_executable(&self, executable: bool) {
        let history = self.index.join("extensions/morphir-elm.jsonl");
        let current = fs::read_to_string(&history).unwrap();
        let updated = if executable {
            current.replace("\"executable\":false", "\"executable\":true")
        } else {
            current.replace("\"executable\":true", "\"executable\":false")
        };
        assert_ne!(
            current, updated,
            "fixture executable declaration did not change"
        );
        fs::write(history, updated).unwrap();
    }
}

#[test]
fn morphir_home_has_durable_distribution_paths() {
    let mother = DistributionMother::a_local_process_artifact();
    assert_eq!(
        mother.home.extensions_store_dir(),
        mother.root.path().join("home/store/extensions/sha256")
    );
    assert_eq!(
        mother.home.extensions_catalog_file(),
        mother.root.path().join("home/catalog/extensions.json")
    );
    assert_eq!(
        mother.home.extensions_locks_dir(),
        mother.root.path().join("home/locks/extensions")
    );
}

#[test]
fn local_index_rejects_source_traversal_and_symlink_escape() {
    let mother = DistributionMother::a_local_process_artifact();
    let history = mother.index.join("extensions/morphir-elm.jsonl");
    let original = fs::read_to_string(&history).unwrap();
    fs::write(
        &history,
        original.replace("artifacts/morphir-elm", "../outside"),
    )
    .unwrap();
    assert!(
        LocalIndex::open(&mother.index)
            .unwrap()
            .resolve(
                &mother.id,
                Selection::Channel(Channel::Stable),
                &Platform::new("linux", "x86_64").unwrap(),
            )
            .is_err()
    );

    fs::write(&history, original).unwrap();
    let outside = mother.root.path().join("outside");
    fs::write(&outside, b"outside").unwrap();
    let source = mother.index.join("artifacts/morphir-elm");
    fs::remove_file(&source).unwrap();
    make_file_symlink(&outside, &source);
    let error = ArtifactStore::from_home(&mother.home)
        .materialize(mother.selected())
        .unwrap_err();
    assert!(error.to_string().contains("escapes local index root"));
}

#[test]
fn content_addressed_publication_rejects_a_symlinked_digest_directory() {
    let mother = DistributionMother::a_local_process_artifact();
    let outside = mother.root.path().join("outside-store");
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(mother.home.extensions_store_dir()).unwrap();
    make_dir_symlink(
        &outside,
        &mother
            .home
            .extensions_store_dir()
            .join(mother.digest.to_string()),
    );

    let error = ArtifactStore::from_home(&mother.home)
        .materialize(mother.selected())
        .unwrap_err();
    assert!(error.to_string().contains("escapes Morphir home"));
    assert!(!outside.join("morphir-elm").exists());
}

#[test]
fn materialization_verifies_then_atomically_publishes_into_the_cas() {
    let mother = DistributionMother::a_local_process_artifact();
    let verified = ArtifactStore::from_home(&mother.home)
        .materialize(mother.selected())
        .unwrap();
    let expected = mother
        .home
        .extensions_store_dir()
        .join(mother.digest.to_string())
        .join("morphir-elm");

    assert_eq!(verified.path(), fs::canonicalize(&expected).unwrap());
    assert_eq!(fs::read(expected).unwrap(), b"#!/bin/sh\necho morphir\n");
    assert_eq!(count_staging_files(&mother.home.extensions_store_dir()), 0);
}

#[cfg(unix)]
#[test]
fn executable_permission_is_applied_only_when_declared() {
    use std::os::unix::fs::PermissionsExt;

    let executable = DistributionMother::a_local_process_artifact();
    let executable_artifact = ArtifactStore::from_home(&executable.home)
        .materialize(executable.selected())
        .unwrap();
    assert_ne!(
        fs::metadata(executable_artifact.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o100,
        0
    );

    let non_executable = DistributionMother::a_local_process_artifact();
    let history = non_executable.index.join("extensions/morphir-elm.jsonl");
    let record = fs::read_to_string(&history)
        .unwrap()
        .replace("\"executable\":true", "\"executable\":false");
    fs::write(&history, record).unwrap();
    let non_executable_artifact = ArtifactStore::from_home(&non_executable.home)
        .materialize(non_executable.selected())
        .unwrap();
    assert_eq!(
        fs::metadata(non_executable_artifact.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o100,
        0
    );
}

#[test]
fn an_existing_cas_object_is_rehashed_before_reuse() {
    let mother = DistributionMother::a_local_process_artifact();
    let store = ArtifactStore::from_home(&mother.home);
    let verified = store.materialize(mother.selected()).unwrap();
    fs::write(verified.path(), b"tampered installed bytes").unwrap();

    let error = store.materialize(mother.selected()).unwrap_err();
    assert!(error.to_string().contains("digest mismatch"));
    assert_eq!(
        fs::read(verified.path()).unwrap(),
        b"tampered installed bytes"
    );
}

#[cfg(unix)]
#[test]
fn non_executable_cas_content_cannot_be_reused_as_executable() {
    let mother = DistributionMother::a_local_process_artifact();
    mother.declare_executable(false);
    let store = ArtifactStore::from_home(&mother.home);
    let installed = store.materialize(mother.selected()).unwrap();
    assert!(!owner_executable(installed.path()));

    mother.declare_executable(true);
    let error = store.materialize(mother.selected()).unwrap_err();
    assert!(error.to_string().contains("executable mode mismatch"));
    assert!(!owner_executable(installed.path()));
}

#[cfg(unix)]
#[test]
fn executable_cas_content_cannot_be_reused_as_non_executable() {
    let mother = DistributionMother::a_local_process_artifact();
    let store = ArtifactStore::from_home(&mother.home);
    let installed = store.materialize(mother.selected()).unwrap();
    assert!(owner_executable(installed.path()));

    mother.declare_executable(false);
    let error = store.materialize(mother.selected()).unwrap_err();
    assert!(error.to_string().contains("executable mode mismatch"));
    assert!(owner_executable(installed.path()));
}

#[test]
fn lock_is_exact_and_catalog_registration_accepts_only_verified_artifacts() {
    let mother = DistributionMother::a_local_process_artifact();
    let verified = ArtifactStore::from_home(&mother.home)
        .materialize(mother.selected())
        .unwrap();
    assert!(!mother.home.extensions_catalog_file().exists());

    write_extension_lock(&mother.home, &verified).unwrap();
    let lock = read_extension_lock(&mother.home, &mother.id).unwrap();
    assert_eq!(lock.schema_version(), 1);
    assert_eq!(lock.selection(), &Selection::Channel(Channel::Stable));
    assert_eq!(lock.extension_id(), &mother.id);
    assert_eq!(lock.version().to_string(), "3.2.1");
    assert_eq!(lock.digest(), &mother.digest);
    assert!(lock.executable());
    assert_eq!(lock.index().kind().as_str(), "local-directory");
    assert!(lock.index().identity().is_absolute());
    let lock_json: serde_json::Value = serde_json::from_slice(
        &fs::read(mother.home.extensions_locks_dir().join("morphir-elm.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(lock_json["schemaVersion"], 1);
    assert_eq!(lock_json["selection"]["kind"], "channel");
    assert_eq!(lock_json["selection"]["value"], "stable");
    assert_eq!(lock_json["version"], "3.2.1");
    assert_eq!(lock_json["digest"], mother.digest.to_string());
    assert_eq!(lock_json["executable"], true);
    assert_eq!(
        lock_json["index"]["revision"],
        lock.index().revision().to_string()
    );
    assert!(lock_json.get("storePath").is_none());
    assert!(!mother.home.extensions_catalog_file().exists());

    let mut catalog = InstalledCatalog::load(&mother.home).unwrap();
    let installed = catalog.register(verified).unwrap();
    assert_eq!(installed.extension_id(), &mother.id);
    assert!(installed.executable());
    assert!(mother.home.extensions_catalog_file().exists());
}

#[test]
fn installer_orders_materialization_lock_then_catalog() {
    let mother = DistributionMother::a_local_process_artifact();
    let installed = ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();

    assert_eq!(installed.version().to_string(), "3.2.1");
    assert!(
        mother
            .home
            .extensions_locks_dir()
            .join("morphir-elm.json")
            .exists()
    );
    assert!(mother.home.extensions_catalog_file().exists());
    assert!(installed.store_path().is_relative());
}

#[test]
fn source_tampering_never_creates_a_lock_or_active_catalog_entry() {
    let mother = DistributionMother::a_local_process_artifact();
    let selected = mother.selected();
    fs::write(
        mother.index.join("artifacts/morphir-elm"),
        b"tampered source",
    )
    .unwrap();

    let error = ExtensionInstaller::new(&mother.home)
        .install(selected)
        .unwrap_err();
    assert!(error.to_string().contains("digest mismatch"));
    assert!(!mother.home.extensions_catalog_file().exists());
    assert!(
        !mother
            .home
            .extensions_locks_dir()
            .join("morphir-elm.json")
            .exists()
    );
}

#[test]
fn activation_is_offline_and_reverifies_installed_content() {
    let mother = DistributionMother::a_local_process_artifact();
    let installed = ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();
    fs::remove_dir_all(&mother.index).unwrap();

    let launch = activate_installed(&mother.home, &mother.id).unwrap();
    assert_eq!(
        launch.program(),
        fs::canonicalize(mother.home.root().join(installed.store_path())).unwrap()
    );
    assert_eq!(launch.args(), ["serve"]);
    let info = launch.extension_info();
    assert_eq!(info.id, "morphir-elm");
    assert_eq!(info.name, "Morphir Elm");
    assert_eq!(info.version, "3.2.1");
    assert_eq!(info.types, [morphir_extension_sdk::ExtensionType::Frontend]);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(launch.program()).unwrap().permissions();
        permissions.set_mode(permissions.mode() & !0o100);
        fs::set_permissions(launch.program(), permissions).unwrap();
        let error = activate_installed(&mother.home, &mother.id).unwrap_err();
        assert!(error.to_string().contains("executable mode mismatch"));

        let mut permissions = fs::metadata(launch.program()).unwrap().permissions();
        permissions.set_mode(permissions.mode() | 0o100);
        fs::set_permissions(launch.program(), permissions).unwrap();
    }

    fs::write(launch.program(), b"tampered after install").unwrap();
    let error = activate_installed(&mother.home, &mother.id).unwrap_err();
    assert!(error.to_string().contains("digest mismatch"));
}

fn count_staging_files(root: &Path) -> usize {
    fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                count_staging_files(&entry.path())
            } else {
                usize::from(entry.file_name().to_string_lossy().starts_with(".stage-"))
            }
        })
        .sum()
}

#[cfg(unix)]
fn owner_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path).unwrap().permissions().mode() & 0o100 != 0
}

#[cfg(unix)]
fn make_file_symlink(source: &Path, destination: &Path) {
    std::os::unix::fs::symlink(source, destination).unwrap();
}

#[cfg(unix)]
fn make_dir_symlink(source: &Path, destination: &Path) {
    std::os::unix::fs::symlink(source, destination).unwrap();
}

#[cfg(windows)]
fn make_dir_symlink(source: &Path, destination: &Path) {
    std::os::windows::fs::symlink_dir(source, destination).unwrap();
}

#[cfg(windows)]
fn make_file_symlink(source: &Path, destination: &Path) {
    std::os::windows::fs::symlink_file(source, destination).unwrap();
}
