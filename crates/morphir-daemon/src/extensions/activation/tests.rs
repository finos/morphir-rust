//! Activation transport tests.

use super::{BoxedMepTransport, activate_transport, wasm_host_functions};
use crate::extensions::{Loaded, MepTransport, PersistedExtensionCapabilities, Session};
use morphir_common::home::MorphirHome;
use morphir_distribution::{
    Channel, ExtensionId, ExtensionInstaller, LocalIndex, Platform, Selection, Sha256Digest,
    VerifiedExtensionArtifact, activate_installed,
};
use morphir_extension_sdk::protocol::{InitializeParams, PeerInfo};
use morphir_extension_sdk::{
    BackendCapability, ExtensionCapabilities, ExtensionType, FrontendCapability, LanguageCapability,
};
use std::fs;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use tempfile::TempDir;

struct RuntimeArtifact {
    _root: TempDir,
    artifact: VerifiedExtensionArtifact,
    installed_path: PathBuf,
    staging_directory: PathBuf,
    working_directory: PathBuf,
}

mod runtime_mother {
    use super::*;

    #[derive(Clone, Copy)]
    enum MetadataShape {
        LegacyBackend,
        FrontendBackend,
        FrontendWorkspace,
        MigratedFrontendBackend,
    }

    struct ArtifactSpec<'a> {
        id: &'a str,
        name: &'a str,
        runtime: &'a str,
        filename: &'a str,
        bytes: &'a [u8],
        args: &'a [String],
        metadata: MetadataShape,
    }

    #[cfg(unix)]
    pub fn process() -> (RuntimeArtifact, PathBuf, Vec<String>) {
        let root = tempfile::tempdir().unwrap();
        let capture = root.path().join("observed-launch.txt");
        let args = vec![
            capture.to_string_lossy().into_owned(),
            "first argument".to_owned(),
            "--flag=two".to_owned(),
            "café-東京".to_owned(),
            String::new(),
        ];
        let program = b"#!/bin/sh\nprintf '%s\\n' \"$PWD\" > \"$1\"\nprintf '%s\\n' \"$#\" >> \"$1\"\nfor argument do printf '<%s>\\n' \"$argument\" >> \"$1\"; done\nwhile IFS= read -r line; do :; done\n";
        let artifact = install(
            root,
            ArtifactSpec {
                id: "morphir-process",
                name: "Morphir Process",
                runtime: "process",
                filename: "morphir-process",
                bytes: program,
                args: &args,
                metadata: MetadataShape::FrontendBackend,
            },
        );
        (artifact, capture, args)
    }

    #[cfg(unix)]
    pub fn legacy_backend_process() -> RuntimeArtifact {
        install(
            tempfile::tempdir().unwrap(),
            ArtifactSpec {
                id: "legacy-backend",
                name: "Legacy Backend",
                runtime: "process",
                filename: "legacy-backend",
                bytes: b"#!/bin/sh\nwhile IFS= read -r line; do :; done\n",
                args: &[],
                metadata: MetadataShape::LegacyBackend,
            },
        )
    }

    #[cfg(unix)]
    pub fn process_with_capabilities(compile: bool) -> RuntimeArtifact {
        let guest_info = serde_json::json!({
            "id": "morphir-process-capabilities",
            "name": "Morphir Process Capabilities",
            "version": "1.2.3",
            "types": ["frontend", "backend"]
        });
        let guest_capabilities = capabilities_json(compile);
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "0.1",
                "extension": guest_info,
                "capabilities": guest_capabilities
            }
        })
        .to_string();
        let program = format!(
            "#!/bin/sh\nlength=0\nwhile IFS= read -r header; do\ncase \"$header\" in Content-Length:*) length=${{header#*: }}; length=$(printf '%s' \"$length\" | tr -d '\\r') ;; esac\n[ -z \"$(printf '%s' \"$header\" | tr -d '\\r')\" ] && break\ndone\ndd bs=1 count=\"$length\" of=/dev/null 2>/dev/null\nprintf 'Content-Length: %s\\r\\n\\r\\n%s' '{}' '{}'\nwhile IFS= read -r line; do :; done\n",
            response.len(),
            response
        );
        install(
            tempfile::tempdir().unwrap(),
            ArtifactSpec {
                id: "morphir-process-capabilities",
                name: "Morphir Process Capabilities",
                runtime: "process",
                filename: "morphir-process-capabilities",
                bytes: program.as_bytes(),
                args: &[],
                metadata: MetadataShape::FrontendBackend,
            },
        )
    }

    #[cfg(unix)]
    pub fn process_with_frontend_workspace(compile: bool) -> RuntimeArtifact {
        let guest_info = serde_json::json!({
            "id": "morphir-process-workspace",
            "name": "Morphir Process Workspace",
            "version": "1.2.3",
            "types": ["frontend", "workspace"]
        });
        let guest_capabilities = frontend_workspace_capabilities_json(compile);
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "0.1",
                "extension": guest_info,
                "capabilities": guest_capabilities
            }
        })
        .to_string();
        let program = format!(
            "#!/bin/sh\nlength=0\nwhile IFS= read -r header; do\ncase \"$header\" in Content-Length:*) length=${{header#*: }}; length=$(printf '%s' \"$length\" | tr -d '\\r') ;; esac\n[ -z \"$(printf '%s' \"$header\" | tr -d '\\r')\" ] && break\ndone\ndd bs=1 count=\"$length\" of=/dev/null 2>/dev/null\nprintf 'Content-Length: %s\\r\\n\\r\\n%s' '{}' '{}'\nwhile IFS= read -r line; do :; done\n",
            response.len(),
            response
        );
        install(
            tempfile::tempdir().unwrap(),
            ArtifactSpec {
                id: "morphir-process-workspace",
                name: "Morphir Process Workspace",
                runtime: "process",
                filename: "morphir-process-workspace",
                bytes: program.as_bytes(),
                args: &[],
                metadata: MetadataShape::FrontendWorkspace,
            },
        )
    }

    #[cfg(unix)]
    pub fn process_with_migrated_frontend_metadata() -> RuntimeArtifact {
        let guest_info = serde_json::json!({
            "id": "morphir-process-migrated",
            "name": "Morphir Process Migrated",
            "version": "1.2.3",
            "types": ["frontend", "backend"]
        });
        let guest_capabilities = capabilities_json(true);
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "0.1",
                "extension": guest_info,
                "capabilities": guest_capabilities
            }
        })
        .to_string();
        let program = format!(
            "#!/bin/sh\nlength=0\nwhile IFS= read -r header; do\ncase \"$header\" in Content-Length:*) length=${{header#*: }}; length=$(printf '%s' \"$length\" | tr -d '\\r') ;; esac\n[ -z \"$(printf '%s' \"$header\" | tr -d '\\r')\" ] && break\ndone\ndd bs=1 count=\"$length\" of=/dev/null 2>/dev/null\nprintf 'Content-Length: %s\\r\\n\\r\\n%s' '{}' '{}'\nwhile IFS= read -r line; do :; done\n",
            response.len(),
            response
        );
        install(
            tempfile::tempdir().unwrap(),
            ArtifactSpec {
                id: "morphir-process-migrated",
                name: "Morphir Process Migrated",
                runtime: "process",
                filename: "morphir-process-migrated",
                bytes: program.as_bytes(),
                args: &[],
                metadata: MetadataShape::MigratedFrontendBackend,
            },
        )
    }

    pub fn wasm() -> RuntimeArtifact {
        install(
            tempfile::tempdir().unwrap(),
            ArtifactSpec {
                id: "morphir-avro",
                name: "Morphir Avro",
                runtime: "wasm",
                filename: "morphir-avro.wasm",
                bytes: &valid_guest_bytes(),
                args: &[],
                metadata: MetadataShape::FrontendBackend,
            },
        )
    }

    pub fn wasm_with_capabilities(compile: bool) -> RuntimeArtifact {
        let guest_info = serde_json::json!({
            "id": "morphir-capabilities",
            "name": "Morphir Capabilities",
            "version": "1.2.3",
            "types": ["frontend", "backend"]
        });
        let guest_capabilities = capabilities_json(compile);
        let bytes = guest_bytes(guest_info, guest_capabilities);
        install(
            tempfile::tempdir().unwrap(),
            ArtifactSpec {
                id: "morphir-capabilities",
                name: "Morphir Capabilities",
                runtime: "wasm",
                filename: "morphir-capabilities.wasm",
                bytes: &bytes,
                args: &[],
                metadata: MetadataShape::FrontendBackend,
            },
        )
    }

    pub fn wasm_with_frontend_workspace(compile: bool) -> RuntimeArtifact {
        let guest_info = serde_json::json!({
            "id": "morphir-wasm-workspace",
            "name": "Morphir Wasm Workspace",
            "version": "1.2.3",
            "types": ["frontend", "workspace"]
        });
        let guest_capabilities = frontend_workspace_capabilities_json(compile);
        let bytes = guest_bytes(guest_info, guest_capabilities);
        install(
            tempfile::tempdir().unwrap(),
            ArtifactSpec {
                id: "morphir-wasm-workspace",
                name: "Morphir Wasm Workspace",
                runtime: "wasm",
                filename: "morphir-wasm-workspace.wasm",
                bytes: &bytes,
                args: &[],
                metadata: MetadataShape::FrontendWorkspace,
            },
        )
    }

    fn capabilities_json(compile: bool) -> serde_json::Value {
        serde_json::json!({
            "frontend": {
                "languages": [{"id": "gleam", "fileExtensions": [".gleam"]}],
                "irVersions": ["4"],
                "compile": compile,
                "incremental": false,
                "fragments": false
            },
            "backend": {
                "targets": ["avro", "json-schema"],
                "irVersions": ["3", "4"],
                "generate": true
            }
        })
    }

    fn frontend_workspace_capabilities_json(compile: bool) -> serde_json::Value {
        serde_json::json!({
            "frontend": {
                "languages": [{"id": "gleam", "fileExtensions": [".gleam"]}],
                "irVersions": ["4"],
                "compile": compile,
                "incremental": false,
                "fragments": false
            },
            "workspace": {
                "protocolVersions": [1],
                "discover": true
            }
        })
    }

    fn install(root: TempDir, spec: ArtifactSpec<'_>) -> RuntimeArtifact {
        let index = root.path().join("index");
        let source = index.join("artifacts").join(spec.filename);
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::create_dir_all(index.join("extensions")).unwrap();
        fs::write(&source, spec.bytes).unwrap();
        let digest = Sha256Digest::of_bytes(spec.bytes);
        let platform = Platform::current();
        let artifact = if spec.runtime == "process" {
            serde_json::json!({
                "runtime": spec.runtime,
                "platform": { "os": platform.os(), "arch": platform.arch() },
                "source": { "kind": "local-file", "path": format!("artifacts/{}", spec.filename) },
                "sha256": digest,
                "filename": spec.filename,
                "args": spec.args,
                "executable": true
            })
        } else {
            serde_json::json!({
                "runtime": spec.runtime,
                "source": { "kind": "local-file", "path": format!("artifacts/{}", spec.filename) },
                "sha256": digest,
                "filename": spec.filename
            })
        };
        let capabilities = match spec.metadata {
            MetadataShape::LegacyBackend => serde_json::json!(["backend"]),
            MetadataShape::FrontendBackend | MetadataShape::MigratedFrontendBackend => {
                serde_json::json!(["frontend", "backend"])
            }
            MetadataShape::FrontendWorkspace => serde_json::json!(["frontend", "workspace"]),
        };
        let mut record = serde_json::json!({
            "schemaVersion": if matches!(spec.metadata, MetadataShape::LegacyBackend) { 1 } else { 2 },
            "id": spec.id,
            "name": spec.name,
            "version": "1.2.3",
            "channels": ["stable"],
            "mepVersions": ["0.1"],
            "capabilities": capabilities,
            "artifacts": [artifact]
        });
        if !matches!(spec.metadata, MetadataShape::LegacyBackend) {
            record.as_object_mut().unwrap().insert(
                "frontend".into(),
                serde_json::json!({
                    "languages": [{"id": "gleam", "fileExtensions": [".gleam"]}],
                    "irVersions": ["4"],
                    "compile": true
                }),
            );
        }
        if matches!(
            spec.metadata,
            MetadataShape::FrontendBackend | MetadataShape::MigratedFrontendBackend
        ) {
            record.as_object_mut().unwrap().insert(
                "backend".into(),
                serde_json::json!({
                    "targets": ["avro", "json-schema"],
                    "irVersions": ["3", "4"]
                }),
            );
        }
        fs::write(
            index.join("extensions").join(format!("{}.jsonl", spec.id)),
            format!("{record}\n"),
        )
        .unwrap();

        let home =
            MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
        let extension_id = ExtensionId::parse(spec.id).unwrap();
        let selected = LocalIndex::open(&index)
            .unwrap()
            .resolve(
                &extension_id,
                Selection::Channel(Channel::Stable),
                &platform,
            )
            .unwrap();
        let installed = ExtensionInstaller::new(&home).install(selected).unwrap();
        let installed_path = home.root().join(installed.store_path());
        let staging_directory = home.temp_dir().join("extensions");
        if matches!(spec.metadata, MetadataShape::MigratedFrontendBackend) {
            let lock_path = home
                .extensions_locks_dir()
                .join(format!("{}.json", spec.id));
            let mut lock: serde_json::Value =
                serde_json::from_slice(&fs::read(&lock_path).unwrap()).unwrap();
            lock["schemaVersion"] = serde_json::json!(3);
            assert!(lock.as_object_mut().unwrap().remove("frontend").is_some());
            assert!(
                lock.as_object_mut()
                    .unwrap()
                    .remove("frontendMetadataScope")
                    .is_some()
            );
            assert!(
                lock.as_object_mut()
                    .unwrap()
                    .remove("backendMetadataScope")
                    .is_some()
            );
            fs::write(&lock_path, serde_json::to_vec_pretty(&lock).unwrap()).unwrap();

            let catalog_path = home.extensions_catalog_file();
            let mut catalog: serde_json::Value =
                serde_json::from_slice(&fs::read(&catalog_path).unwrap()).unwrap();
            catalog["schemaVersion"] = serde_json::json!(2);
            assert!(
                catalog["extensions"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("frontend")
                    .is_some()
            );
            assert!(
                catalog["extensions"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("frontendMetadataScope")
                    .is_some()
            );
            assert!(
                catalog["extensions"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("backendMetadataScope")
                    .is_some()
            );
            fs::write(&catalog_path, serde_json::to_vec_pretty(&catalog).unwrap()).unwrap();
        }
        let artifact = activate_installed(&home, &extension_id).unwrap();
        let working_directory = root.path().join("workspace");
        fs::create_dir(&working_directory).unwrap();

        RuntimeArtifact {
            _root: root,
            artifact,
            installed_path,
            staging_directory,
            working_directory,
        }
    }

    fn valid_guest_bytes() -> Vec<u8> {
        let guest_info = serde_json::json!({
            "id": "guest-self-report",
            "name": "Guest Self Report",
            "version": "9.9.9",
            "types": ["validator"]
        });
        guest_bytes(guest_info, serde_json::json!({}))
    }

    fn guest_bytes(
        guest_info: serde_json::Value,
        guest_capabilities: serde_json::Value,
    ) -> Vec<u8> {
        let info = guest_info.to_string();
        let initialize_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "0.1",
                "extension": guest_info,
                "capabilities": guest_capabilities
            }
        })
        .to_string();
        let writes = |output: &str| {
            output
                    .bytes()
                    .enumerate()
                    .map(|(index, byte)| {
                        format!(
                            "(call $store_u8 (i64.add (local.get $output) (i64.const {index})) (i32.const {byte}))"
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
        };
        let info_writes = writes(&info);
        let response_writes = writes(&initialize_response);
        let wat = format!(
            r#"(module
                    (import "extism:host/env" "alloc" (func $alloc (param i64) (result i64)))
                    (import "extism:host/env" "store_u8" (func $store_u8 (param i64 i32)))
                    (import "extism:host/env" "output_set" (func $output_set (param i64 i64)))
                    (func (export "morphir_extension_info") (result i32)
                        (local $output i64)
                        (local.set $output (call $alloc (i64.const {info_length})))
                        {info_writes}
                        (call $output_set (local.get $output) (i64.const {info_length}))
                        (i32.const 0))
                    (func (export "handle") (result i32)
                        (local $output i64)
                        (local.set $output (call $alloc (i64.const {response_length})))
                        {response_writes}
                        (call $output_set (local.get $output) (i64.const {response_length}))
                        (i32.const 0)))"#,
            info_length = info.len(),
            response_length = initialize_response.len(),
        );
        wat::parse_str(wat).unwrap()
    }
}

fn expected_backend_capability() -> BackendCapability {
    BackendCapability {
        targets: vec!["avro".into(), "json-schema".into()],
        ir_versions: vec!["3".into(), "4".into()],
        generate: true,
    }
}

fn expected_capabilities() -> ExtensionCapabilities {
    ExtensionCapabilities {
        frontend: Some(FrontendCapability {
            languages: vec![LanguageCapability {
                id: "gleam".into(),
                file_extensions: vec![".gleam".into()],
            }],
            ir_versions: vec!["4".into()],
            compile: true,
            incremental: false,
            fragments: false,
        }),
        backend: Some(expected_backend_capability()),
        ..ExtensionCapabilities::default()
    }
}

fn expected_persisted_capabilities() -> PersistedExtensionCapabilities {
    let capabilities = expected_capabilities();
    PersistedExtensionCapabilities::new(capabilities.frontend, capabilities.backend)
}

#[test]
fn installed_wasm_activation_uses_restricted_generation_host_policy() {
    let workspace = tempfile::tempdir().unwrap();

    let host = wasm_host_functions(workspace.path());

    assert!(host.workspace_info().output_dir.is_empty());
}

#[tokio::test]
#[cfg(unix)]
async fn process_activation_preserves_discovery_and_persisted_capabilities() {
    let (fixture, capture, args) = runtime_mother::process();
    let expected_working_directory = fs::canonicalize(&fixture.working_directory).unwrap();
    let staging_directory = fixture.staging_directory.clone();
    let session = activate_transport(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap();

    assert_eq!(fs::read_dir(staging_directory).unwrap().count(), 1);

    let expected = session.transport_internal().expected_extension();
    assert_eq!(expected.id(), "morphir-process");
    let discovered = expected.extension_info().unwrap();
    assert_eq!(discovered.name, "Morphir Process");
    assert_eq!(discovered.version, "1.2.3");
    assert_eq!(
        discovered.types,
        [ExtensionType::Frontend, ExtensionType::Backend]
    );
    assert_eq!(
        expected.backend_capability(),
        Some(&expected_backend_capability())
    );
    assert_eq!(
        expected.persisted_capabilities(),
        Some(&expected_persisted_capabilities())
    );
    assert!(expected.capabilities().is_none());

    let observed = wait_for_launch(&capture, args.len() + 2).await;
    let mut lines = observed.lines();
    assert_eq!(
        lines.next(),
        Some(expected_working_directory.to_str().unwrap())
    );
    assert_eq!(lines.next(), Some(args.len().to_string().as_str()));
    assert_eq!(
        lines.collect::<Vec<_>>(),
        args.iter()
            .map(|argument| format!("<{argument}>"))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
#[cfg(unix)]
async fn process_activation_uses_the_exact_bytes_verified_before_store_replacement() {
    let (fixture, capture, args) = runtime_mother::process();
    fs::write(
        &fixture.installed_path,
        b"#!/bin/sh\nprintf 'replacement\\n' > \"$1\"\nwhile IFS= read -r line; do :; done\n",
    )
    .unwrap();

    let _session = activate_transport(fixture.artifact, &fixture.working_directory)
        .await
        .expect("activation should stage and execute the already verified process bytes");

    let observed = wait_for_launch(&capture, args.len() + 2).await;
    assert_ne!(observed.trim(), "replacement");
    assert_eq!(
        observed.lines().next(),
        fs::canonicalize(&fixture.working_directory)
            .unwrap()
            .to_str()
    );
}

#[tokio::test]
#[cfg(unix)]
async fn legacy_process_activation_does_not_invent_a_backend_lock() {
    let fixture = runtime_mother::legacy_backend_process();
    let session = activate_transport(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap();

    let expected = session.transport_internal().expected_extension();
    assert_eq!(expected.id(), "legacy-backend");
    assert_eq!(
        expected.extension_info().unwrap().types,
        [ExtensionType::Backend]
    );
    assert!(expected.capabilities().is_none());
}

#[tokio::test]
#[cfg(unix)]
async fn process_activation_negotiates_persisted_frontend_and_backend_capabilities() {
    let fixture = runtime_mother::process_with_capabilities(true);
    let negotiation = activate_transport(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap()
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "activation-test".into(),
                version: "1.0.0".into(),
            },
        })
        .await;
    let ready = match negotiation {
        Ok(ready) => ready,
        Err(failure) => panic!(
            "the process should reproduce both installed capabilities: {}",
            failure.error()
        ),
    };

    assert_eq!(ready.negotiated().capabilities(), &expected_capabilities());
}

#[tokio::test]
#[cfg(unix)]
async fn process_activation_allows_unpersisted_workspace_capabilities() {
    let fixture = runtime_mother::process_with_frontend_workspace(true);
    let negotiation = activate_transport(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap()
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "activation-test".into(),
                version: "1.0.0".into(),
            },
        })
        .await;

    match negotiation {
        Ok(ready) => assert!(ready.negotiated().capabilities().workspace.is_some()),
        Err(failure) => panic!(
            "unpersisted workspace capabilities should remain negotiable: {}",
            failure.error()
        ),
    }
}

#[tokio::test]
#[cfg(unix)]
async fn process_activation_allows_unpersisted_frontend_from_migrated_state() {
    let fixture = runtime_mother::process_with_migrated_frontend_metadata();
    let negotiation = activate_transport(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap()
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "activation-test".into(),
                version: "1.0.0".into(),
            },
        })
        .await;

    match negotiation {
        Ok(ready) => {
            assert!(ready.negotiated().capabilities().frontend.is_some());
            assert!(ready.negotiated().capabilities().backend.is_some());
        }
        Err(failure) => panic!(
            "frontend metadata absent from migrated state should remain negotiable: {}",
            failure.error()
        ),
    }
}

#[tokio::test]
#[cfg(unix)]
async fn process_activation_rejects_frontend_capability_drift() {
    let fixture = runtime_mother::process_with_frontend_workspace(false);
    let failure = activate_transport(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap()
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "activation-test".into(),
                version: "1.0.0".into(),
            },
        })
        .await
        .err()
        .expect("frontend capability drift should fail initialization");

    assert!(
        failure
            .error()
            .to_string()
            .contains("capabilities disagreed with discovery")
    );
}

#[tokio::test]
async fn wasm_activation_uses_locked_identity_for_the_shared_loaded_transport() {
    let fixture = runtime_mother::wasm();
    let session: Session<BoxedMepTransport, Loaded> =
        activate_transport(fixture.artifact, &fixture.working_directory)
            .await
            .unwrap();

    let expected = session.transport_internal().expected_extension();
    assert_eq!(expected.id(), "morphir-avro");
    let discovered = expected.extension_info().unwrap();
    assert_eq!(discovered.name, "Morphir Avro");
    assert_eq!(discovered.version, "1.2.3");
    assert_eq!(
        discovered.types,
        [ExtensionType::Frontend, ExtensionType::Backend]
    );
    assert_eq!(
        expected.backend_capability(),
        Some(&expected_backend_capability())
    );
    assert_eq!(
        expected.persisted_capabilities(),
        Some(&expected_persisted_capabilities())
    );
    assert!(expected.capabilities().is_none());

    let failure = session
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "activation-test".into(),
                version: "1.0.0".into(),
            },
        })
        .await
        .err()
        .expect("guest identity drift should fail initialization");
    let error = failure.error().to_string();
    assert!(error.contains("identity changed during initialization"));
    assert!(error.contains("morphir-avro"));
    assert!(error.contains("guest-self-report"));
}

#[tokio::test]
async fn wasm_activation_negotiates_persisted_frontend_and_backend_capabilities() {
    let fixture = runtime_mother::wasm_with_capabilities(true);
    let negotiation = activate_transport(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap()
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "activation-test".into(),
                version: "1.0.0".into(),
            },
        })
        .await;
    let ready = match negotiation {
        Ok(ready) => ready,
        Err(failure) => panic!(
            "the guest should reproduce both installed capabilities: {}",
            failure.error()
        ),
    };

    assert_eq!(ready.negotiated().capabilities(), &expected_capabilities());
}

#[tokio::test]
async fn wasm_activation_allows_unpersisted_workspace_capabilities() {
    let fixture = runtime_mother::wasm_with_frontend_workspace(true);
    let negotiation = activate_transport(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap()
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "activation-test".into(),
                version: "1.0.0".into(),
            },
        })
        .await;

    match negotiation {
        Ok(ready) => assert!(ready.negotiated().capabilities().workspace.is_some()),
        Err(failure) => panic!(
            "unpersisted workspace capabilities should remain negotiable: {}",
            failure.error()
        ),
    }
}

#[tokio::test]
async fn wasm_activation_rejects_frontend_capability_drift() {
    let fixture = runtime_mother::wasm_with_frontend_workspace(false);
    let failure = activate_transport(fixture.artifact, &fixture.working_directory)
        .await
        .unwrap()
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "activation-test".into(),
                version: "1.0.0".into(),
            },
        })
        .await
        .err()
        .expect("frontend capability drift should fail initialization");

    assert!(
        failure
            .error()
            .to_string()
            .contains("capabilities disagreed with discovery")
    );
}

#[tokio::test]
async fn wasm_activation_uses_the_exact_bytes_verified_before_store_replacement() {
    let fixture = runtime_mother::wasm();
    fs::write(&fixture.installed_path, b"replacement wasm bytes").unwrap();

    let session = activate_transport(fixture.artifact, &fixture.working_directory)
        .await
        .expect("activation should consume the already verified wasm bytes");

    assert_eq!(
        session.transport_internal().expected_extension().id(),
        "morphir-avro"
    );
}

#[cfg(unix)]
async fn wait_for_launch(path: &Path, expected_lines: usize) -> String {
    for _ in 0..100 {
        if let Ok(output) = fs::read_to_string(path)
            && output.lines().count() >= expected_lines
        {
            return output;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("spawned process did not record its launch")
}
