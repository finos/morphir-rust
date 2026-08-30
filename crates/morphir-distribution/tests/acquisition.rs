use morphir_common::home::MorphirHome;
use morphir_distribution::{
    ArtifactFilename, ArtifactRuntime, ArtifactStore, Channel, DistributionError, ExtensionId,
    ExtensionInstaller, InstalledCatalog, LocalIndex, Platform, RelativeArtifactPath, Selection,
    Sha256Digest, VerifiedExtensionArtifact, VerifiedProcessArtifact, activate_installed,
    list_installed, read_extension_lock, uninstall_extension, write_extension_lock,
};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Barrier, mpsc};
use std::thread;
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

    fn a_local_wasm_artifact() -> Self {
        let root = tempfile::tempdir().unwrap();
        let index = root.path().join("index");
        let source = index.join("artifacts/morphir_avro_extension.wasm");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(index.join("extensions")).unwrap();
        fs::write(&source, b"portable wasm artifact").unwrap();
        let digest = Sha256Digest::of_bytes(&fs::read(&source).unwrap());
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
                "irVersions": ["3", "4"],
                "generate": false
            },
            "artifacts": [{
                "runtime": "wasm",
                "source": {
                    "kind": "local-file",
                    "path": "artifacts/morphir_avro_extension.wasm"
                },
                "sha256": digest,
                "filename": "morphir_avro_extension.wasm"
            }]
        });
        fs::write(
            index.join("extensions/morphir-avro.jsonl"),
            format!("{record}\n"),
        )
        .unwrap();
        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        Self {
            root,
            index,
            home,
            id: ExtensionId::parse("morphir-avro").unwrap(),
            digest,
        }
    }

    fn selected(&self) -> morphir_distribution::ResolvedArtifact {
        self.selected_for(&self.id)
    }

    fn selected_for(&self, id: &ExtensionId) -> morphir_distribution::ResolvedArtifact {
        LocalIndex::open(&self.index)
            .unwrap()
            .resolve(
                id,
                Selection::Channel(Channel::Stable),
                &Platform::new("linux", "x86_64").unwrap(),
            )
            .unwrap()
    }

    fn add_local_process_artifact(
        &self,
        id: &str,
        name: &str,
        filename: &str,
        bytes: &[u8],
    ) -> ExtensionId {
        let source = self.index.join("artifacts").join(filename);
        fs::write(&source, bytes).unwrap();
        let digest = Sha256Digest::of_bytes(bytes);
        let record = serde_json::json!({
            "schemaVersion": 1,
            "id": id,
            "name": name,
            "version": "1.0.0",
            "channels": ["stable"],
            "mepVersions": ["0.1"],
            "capabilities": ["backend"],
            "artifacts": [{
                "runtime": "process",
                "platform": { "os": "linux", "arch": "x86_64" },
                "source": { "kind": "local-file", "path": format!("artifacts/{filename}") },
                "sha256": digest,
                "filename": filename,
                "args": [],
                "executable": true
            }]
        });
        fs::write(
            self.index.join("extensions").join(format!("{id}.jsonl")),
            format!("{record}\n"),
        )
        .unwrap();
        ExtensionId::parse(id).unwrap()
    }

    #[cfg(unix)]
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
    assert_eq!(
        mother.home.extensions_state_lock_file(),
        mother.root.path().join("home/locks/extensions.state.lock")
    );
}

#[test]
fn tools_and_extensions_share_verification_but_not_store_namespaces() {
    let root = tempfile::tempdir().unwrap();
    let source_root = root.path().join("repository");
    let source = source_root.join("artifacts/example");
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::write(&source, b"verified tool bytes").unwrap();
    let digest = Sha256Digest::of_bytes(b"verified tool bytes");
    let relative_source = RelativeArtifactPath::parse("artifacts/example").unwrap();
    let filename = ArtifactFilename::parse("example").unwrap();
    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();

    let tool = ArtifactStore::for_tools(&home)
        .materialize_file(&source_root, &relative_source, &digest, &filename, false)
        .unwrap();
    let extension = ArtifactStore::from_home(&home)
        .materialize_file(&source_root, &relative_source, &digest, &filename, false)
        .unwrap();

    assert_eq!(
        tool.store_path().to_string_lossy().replace('\\', "/"),
        format!("store/tools/sha256/{digest}/objects/example")
    );
    assert_eq!(
        extension.store_path().to_string_lossy().replace('\\', "/"),
        format!("store/extensions/sha256/{digest}/example")
    );
    assert_ne!(tool.path(), extension.path());
    assert_eq!(fs::read(tool.path()).unwrap(), b"verified tool bytes");
    assert_eq!(fs::read(extension.path()).unwrap(), b"verified tool bytes");
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
    assert_eq!(lock.schema_version(), 3);
    assert_eq!(lock.selection(), &Selection::Channel(Channel::Stable));
    assert_eq!(lock.extension_id(), &mother.id);
    assert_eq!(lock.version().to_string(), "3.2.1");
    assert_eq!(lock.digest(), &mother.digest);
    assert_eq!(lock.args(), ["serve"]);
    assert_eq!(
        lock.capabilities(),
        [morphir_distribution::Capability::Frontend]
    );
    assert_eq!(lock.mep_versions(), ["0.1"]);
    assert!(lock.executable());
    assert_eq!(lock.index().kind().as_str(), "local-directory");
    assert!(lock.index().identity().is_absolute());
    let lock_json: serde_json::Value = serde_json::from_slice(
        &fs::read(mother.home.extensions_locks_dir().join("morphir-elm.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(lock_json["schemaVersion"], 3);
    assert_eq!(lock_json["selection"]["kind"], "channel");
    assert_eq!(lock_json["selection"]["value"], "stable");
    assert_eq!(lock_json["version"], "3.2.1");
    assert_eq!(lock_json["digest"], mother.digest.to_string());
    assert_eq!(lock_json["executable"], true);
    assert_eq!(lock_json["args"], serde_json::json!(["serve"]));
    assert_eq!(lock_json["capabilities"], serde_json::json!(["frontend"]));
    assert_eq!(lock_json["mepVersions"], serde_json::json!(["0.1"]));
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
fn installed_wasm_persists_runtime_metadata_and_activates_offline() {
    let mother = DistributionMother::a_local_wasm_artifact();
    let installed = ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();
    let expected_canonical_path =
        fs::canonicalize(mother.home.root().join(installed.store_path())).unwrap();

    let lock = read_extension_lock(&mother.home, &mother.id).unwrap();
    assert_eq!(lock.schema_version(), 3);
    assert_eq!(lock.runtime(), ArtifactRuntime::Wasm);
    assert_eq!(lock.platform(), None);
    assert_eq!(lock.digest(), &mother.digest);
    assert_eq!(lock.backend().unwrap().targets(), ["avro"]);
    assert_eq!(lock.backend().unwrap().ir_versions(), ["3", "4"]);
    assert!(!lock.backend().unwrap().generate());
    assert!(!lock.executable());

    let catalog = InstalledCatalog::load(&mother.home).unwrap();
    let catalog_entry = catalog.get(&mother.id).unwrap();
    assert_eq!(catalog_entry.runtime(), ArtifactRuntime::Wasm);
    assert_eq!(catalog_entry.platform(), None);
    assert_eq!(catalog_entry.digest(), &mother.digest);
    assert_eq!(catalog_entry.backend().unwrap().targets(), ["avro"]);
    assert_eq!(catalog_entry.backend().unwrap().ir_versions(), ["3", "4"]);
    assert!(!catalog_entry.backend().unwrap().generate());
    assert!(!catalog_entry.executable());
    assert!(expected_canonical_path.starts_with(fs::canonicalize(mother.home.root()).unwrap()));

    let lock_json: serde_json::Value = serde_json::from_slice(
        &fs::read(mother.home.extensions_locks_dir().join("morphir-avro.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(lock_json["schemaVersion"], 3);
    assert_eq!(lock_json["runtime"], "wasm");
    assert_eq!(lock_json["platform"], serde_json::Value::Null);
    assert_eq!(lock_json["digest"], mother.digest.to_string());
    assert_eq!(lock_json["backend"]["targets"], serde_json::json!(["avro"]));
    assert_eq!(
        lock_json["backend"]["irVersions"],
        serde_json::json!(["3", "4"])
    );
    assert_eq!(lock_json["backend"]["generate"], false);
    assert_eq!(lock_json["executable"], false);

    let catalog_json: serde_json::Value =
        serde_json::from_slice(&fs::read(mother.home.extensions_catalog_file()).unwrap()).unwrap();
    assert_eq!(catalog_json["schemaVersion"], 2);
    assert_eq!(catalog_json["extensions"][0]["runtime"], "wasm");
    assert_eq!(
        catalog_json["extensions"][0]["platform"],
        serde_json::Value::Null
    );
    assert_eq!(
        catalog_json["extensions"][0]["backend"]["targets"],
        serde_json::json!(["avro"])
    );
    assert_eq!(catalog_json["extensions"][0]["backend"]["generate"], false);
    assert_eq!(catalog_json["extensions"][0]["executable"], false);

    fs::remove_dir_all(&mother.index).unwrap();
    let activated = activate_installed(&mother.home, &mother.id).unwrap();
    match activated {
        VerifiedExtensionArtifact::Wasm(wasm) => {
            assert_eq!(wasm.extension_info().id, "morphir-avro");
            assert_eq!(wasm.path(), expected_canonical_path);
            assert_eq!(wasm.backend().unwrap().targets(), ["avro"]);
            let backend = wasm.extension_capabilities().backend.unwrap();
            assert_eq!(backend.targets, ["avro"]);
            assert_eq!(backend.ir_versions, ["3", "4"]);
            assert!(!backend.generate);
        }
        VerifiedExtensionArtifact::Process(_) => panic!("expected wasm artifact"),
    }
}

#[test]
fn installed_wasm_rejects_tampered_bytes() {
    let mother = DistributionMother::a_local_wasm_artifact();
    let installed = ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();
    let path = mother.home.root().join(installed.store_path());
    fs::write(&path, b"tampered wasm bytes").unwrap();

    let error = activate_installed(&mother.home, &mother.id).unwrap_err();

    assert!(error.to_string().contains("digest mismatch"));
}

#[test]
fn installed_wasm_rejects_oversized_bytes_before_buffering() {
    let mother = DistributionMother::a_local_wasm_artifact();
    let installed = ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();
    let path = mother.home.root().join(installed.store_path());
    let oversized = 256 * 1024 * 1024 + 1;
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(oversized)
        .unwrap();

    let error = activate_installed(&mother.home, &mother.id).unwrap_err();

    match error {
        DistributionError::ArtifactTooLarge {
            path: actual_path,
            actual,
            limit,
        } => {
            assert_eq!(actual_path, fs::canonicalize(path).unwrap());
            assert_eq!(actual, oversized);
            assert_eq!(limit, 256 * 1024 * 1024);
        }
        other => panic!("expected ArtifactTooLarge, got {other}"),
    }
}

#[test]
fn installed_wasm_rejects_backend_lock_catalog_mismatch() {
    let mother = DistributionMother::a_local_wasm_artifact();
    ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();
    let lock_path = mother.home.extensions_locks_dir().join("morphir-avro.json");
    let mut lock: serde_json::Value =
        serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();
    lock["backend"]["targets"] = serde_json::json!(["protobuf"]);
    fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();

    let error = activate_installed(&mother.home, &mother.id).unwrap_err();

    match error {
        DistributionError::StateMismatch { id } => assert_eq!(id, mother.id),
        other => panic!("expected StateMismatch, got {other}"),
    }
}

#[cfg(unix)]
#[test]
fn installed_wasm_rejects_symlink_escape_and_executable_mode() {
    use std::os::unix::fs::PermissionsExt;

    let escaped = DistributionMother::a_local_wasm_artifact();
    let installed = ExtensionInstaller::new(&escaped.home)
        .install(escaped.selected())
        .unwrap();
    let path = escaped.home.root().join(installed.store_path());
    let outside = escaped.root.path().join("outside.wasm");
    fs::write(&outside, b"portable wasm artifact").unwrap();
    fs::remove_file(&path).unwrap();
    make_file_symlink(&outside, &path);
    let error = activate_installed(&escaped.home, &escaped.id).unwrap_err();
    assert!(error.to_string().contains("escapes Morphir home"));

    let executable = DistributionMother::a_local_wasm_artifact();
    let installed = ExtensionInstaller::new(&executable.home)
        .install(executable.selected())
        .unwrap();
    let path = executable.home.root().join(installed.store_path());
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(permissions.mode() | 0o100);
    fs::set_permissions(&path, permissions).unwrap();
    let error = activate_installed(&executable.home, &executable.id).unwrap_err();
    assert!(error.to_string().contains("executable mode mismatch"));
}

#[test]
fn installed_wasm_rejects_process_only_state() {
    for (field, value) in [
        ("args", serde_json::json!(["serve"])),
        ("executable", serde_json::json!(true)),
        (
            "platform",
            serde_json::json!({ "os": "linux", "arch": "x86_64" }),
        ),
        ("backend", serde_json::Value::Null),
    ] {
        let mother = DistributionMother::a_local_wasm_artifact();
        ExtensionInstaller::new(&mother.home)
            .install(mother.selected())
            .unwrap();
        let lock_path = mother.home.extensions_locks_dir().join("morphir-avro.json");
        let catalog_path = mother.home.extensions_catalog_file();
        let mut lock: serde_json::Value =
            serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();
        let mut catalog: serde_json::Value =
            serde_json::from_slice(&fs::read(&catalog_path).unwrap()).unwrap();
        lock[field] = value.clone();
        catalog["extensions"][0][field] = value;
        fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
        fs::write(&catalog_path, serde_json::to_vec_pretty(&catalog).unwrap()).unwrap();

        let error = activate_installed(&mother.home, &mother.id).unwrap_err();

        assert!(
            error.to_string().contains("invalid installed state"),
            "unexpected error after tampering {field}: {error}"
        );
    }
}

#[test]
fn legacy_process_installed_state_remains_activatable() {
    let mother = DistributionMother::a_local_process_artifact();
    ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();
    let lock_path = mother.home.extensions_locks_dir().join("morphir-elm.json");
    let catalog_path = mother.home.extensions_catalog_file();
    let mut lock: serde_json::Value =
        serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog_path).unwrap()).unwrap();
    lock["schemaVersion"] = serde_json::json!(2);
    lock.as_object_mut().unwrap().remove("backend");
    catalog["schemaVersion"] = serde_json::json!(1);
    catalog["extensions"][0]
        .as_object_mut()
        .unwrap()
        .remove("backend");
    fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
    fs::write(&catalog_path, serde_json::to_vec_pretty(&catalog).unwrap()).unwrap();

    let lock = read_extension_lock(&mother.home, &mother.id).unwrap();
    assert_eq!(lock.schema_version(), 2);
    assert_eq!(lock.platform().unwrap().os(), "linux");
    assert_eq!(lock.backend(), None);
    let process = expect_process(activate_installed(&mother.home, &mother.id).unwrap());
    assert_eq!(process.extension_info().id, "morphir-elm");
    assert_eq!(process.args(), ["serve"]);
}

#[test]
fn serialized_legacy_v1_lock_is_rejected_by_its_schema_version() {
    let mother = DistributionMother::a_local_process_artifact();
    ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();
    let lock_path = mother.home.extensions_locks_dir().join("morphir-elm.json");
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();
    legacy["schemaVersion"] = serde_json::json!(1);
    legacy.as_object_mut().unwrap().remove("args");
    legacy.as_object_mut().unwrap().remove("capabilities");
    legacy.as_object_mut().unwrap().remove("mepVersions");
    fs::write(lock_path, serde_json::to_vec_pretty(&legacy).unwrap()).unwrap();

    let error = read_extension_lock(&mother.home, &mother.id).unwrap_err();

    match error {
        DistributionError::UnsupportedStateSchema { kind, version } => {
            assert_eq!(kind, "extension lock");
            assert_eq!(version, 1);
        }
        other => panic!("expected UnsupportedStateSchema, got {other}"),
    }
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
fn concurrent_catalog_transactions_preserve_both_entries() {
    let mother = DistributionMother::a_local_process_artifact();
    let second_id = mother.add_local_process_artifact(
        "morphir-test-backend",
        "Morphir Test Backend",
        "morphir-test-backend",
        b"test backend",
    );
    let store = ArtifactStore::from_home(&mother.home);
    let first_artifact = store.materialize(mother.selected()).unwrap();
    let second_artifact = store.materialize(mother.selected_for(&second_id)).unwrap();

    let barrier = Arc::new(Barrier::new(3));
    let (first_done_tx, first_done_rx) = mpsc::channel();
    let first_home = mother.home.clone();
    let first_barrier = Arc::clone(&barrier);
    let first = thread::spawn(move || {
        let mut catalog = InstalledCatalog::load(&first_home).unwrap();
        first_barrier.wait();
        catalog.register(first_artifact).unwrap();
        first_done_tx.send(()).unwrap();
    });
    let second_home = mother.home.clone();
    let second_barrier = Arc::clone(&barrier);
    let second = thread::spawn(move || {
        let mut catalog = InstalledCatalog::load(&second_home).unwrap();
        second_barrier.wait();
        first_done_rx.recv().unwrap();
        catalog.register(second_artifact).unwrap();
    });

    barrier.wait();
    first.join().unwrap();
    second.join().unwrap();

    let catalog = InstalledCatalog::load(&mother.home).unwrap();
    assert!(catalog.get(&mother.id).is_some());
    assert!(catalog.get(&second_id).is_some());
    assert_eq!(catalog.entries().len(), 2);
}

#[test]
fn atomic_listing_returns_validated_entries_with_their_requested_selections() {
    let mother = DistributionMother::a_local_process_artifact();
    let installed = ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();

    let snapshots = list_installed(&mother.home).unwrap();

    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].installed(), &installed);
    assert_eq!(
        snapshots[0].selection(),
        &Selection::Channel(Channel::Stable)
    );
}

#[test]
fn atomic_listing_rejects_a_corrupted_catalog_and_lock_pair() {
    let mother = DistributionMother::a_local_process_artifact();
    ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();
    let lock_path = mother.home.extensions_locks_dir().join("morphir-elm.json");
    let mut lock: serde_json::Value =
        serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();
    lock["args"] = serde_json::json!(["--tampered"]);
    fs::write(lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();

    let error = list_installed(&mother.home).unwrap_err();

    match error {
        DistributionError::StateMismatch { id } => assert_eq!(id, mother.id),
        other => panic!("expected StateMismatch, got {other}"),
    }
}

#[test]
fn uninstall_removes_the_catalog_entry_and_exact_lock() {
    let mother = DistributionMother::a_local_process_artifact();
    let installed = ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();

    let removed = uninstall_extension(&mother.home, &mother.id).unwrap();

    assert_eq!(removed, installed);
    assert!(
        InstalledCatalog::load(&mother.home)
            .unwrap()
            .get(&mother.id)
            .is_none()
    );
    assert!(
        !mother
            .home
            .extensions_locks_dir()
            .join("morphir-elm.json")
            .exists()
    );
}

#[test]
fn uninstall_leaves_content_addressed_artifact_bytes_untouched() {
    let mother = DistributionMother::a_local_process_artifact();
    let installed = ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();
    let artifact = mother.home.root().join(installed.store_path());
    let expected = fs::read(&artifact).unwrap();

    uninstall_extension(&mother.home, &mother.id).unwrap();

    assert_eq!(fs::read(artifact).unwrap(), expected);
}

#[test]
fn uninstall_reports_a_typed_error_for_an_extension_that_is_not_installed() {
    let mother = DistributionMother::a_local_process_artifact();

    let error = uninstall_extension(&mother.home, &mother.id).unwrap_err();

    match error {
        DistributionError::NotInstalled { id } => assert_eq!(id, mother.id),
        other => panic!("expected NotInstalled, got {other}"),
    }
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

    let launch = expect_process(activate_installed(&mother.home, &mother.id).unwrap());
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
    assert!(launch.backend().is_none());
    assert!(launch.extension_capabilities().backend.is_none());

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

#[test]
fn activation_rejects_tampered_locked_launch_metadata() {
    let mother = DistributionMother::a_local_process_artifact();
    ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();
    let lock_path = mother.home.extensions_locks_dir().join("morphir-elm.json");
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();

    for (field, tampered) in [
        ("args", serde_json::json!(["--tampered"])),
        ("capabilities", serde_json::json!(["backend"])),
        ("mepVersions", serde_json::json!(["999.0"])),
    ] {
        let mut lock = original.clone();
        lock[field] = tampered;
        fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();

        let error = activate_installed(&mother.home, &mother.id).unwrap_err();
        match error {
            DistributionError::StateMismatch { id } => assert_eq!(id, mother.id),
            other => panic!("expected StateMismatch after tampering {field}, got {other}"),
        }
    }
}

#[test]
fn activation_and_listing_reject_consistently_unsupported_installed_mep_versions() {
    let mother = DistributionMother::a_local_process_artifact();
    ExtensionInstaller::new(&mother.home)
        .install(mother.selected())
        .unwrap();
    let lock_path = mother.home.extensions_locks_dir().join("morphir-elm.json");
    let mut lock: serde_json::Value =
        serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();
    lock["mepVersions"] = serde_json::json!(["999.0"]);
    fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();
    let catalog_path = mother.home.extensions_catalog_file();
    let mut catalog: serde_json::Value =
        serde_json::from_slice(&fs::read(&catalog_path).unwrap()).unwrap();
    catalog["extensions"][0]["mepVersions"] = serde_json::json!(["999.0"]);
    fs::write(catalog_path, serde_json::to_vec_pretty(&catalog).unwrap()).unwrap();

    for error in [
        activate_installed(&mother.home, &mother.id).unwrap_err(),
        list_installed(&mother.home).unwrap_err(),
    ] {
        match error {
            DistributionError::NoCompatibleMepVersion { supported, .. } => {
                assert!(supported.contains(morphir_extension_sdk::protocol::MEP_VERSION));
            }
            other => panic!("expected NoCompatibleMepVersion, got {other}"),
        }
    }
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

fn expect_process(artifact: VerifiedExtensionArtifact) -> VerifiedProcessArtifact {
    match artifact {
        VerifiedExtensionArtifact::Process(process) => process,
        VerifiedExtensionArtifact::Wasm(_) => panic!("expected process artifact"),
    }
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
