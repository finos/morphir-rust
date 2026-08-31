//! Runtime-neutral activation for verified extension artifacts.

use crate::Result;
use crate::extensions::host_functions::MorphirHostFunctions;
use crate::extensions::session::ExtismTransport;
use crate::extensions::{
    ExtensionContainer, Loaded, MepTransport, ProcessLaunch, Session, SpawnedProcessTransport,
};
use morphir_distribution::VerifiedExtensionArtifact;
use std::path::Path;

/// A runtime-erased transport shared by native and WebAssembly extensions.
pub type BoxedMepTransport = Box<dyn MepTransport>;

/// Activate a verified artifact without starting MEP negotiation.
pub async fn activate_transport(
    artifact: VerifiedExtensionArtifact,
    working_directory: &Path,
) -> Result<Session<BoxedMepTransport, Loaded>> {
    let transport: BoxedMepTransport = match artifact {
        VerifiedExtensionArtifact::Process(process) => {
            let launch = if let Some(backend) = process.extension_capabilities().backend {
                ProcessLaunch::from_verified_bytes_with_backend_capability_in(
                    process.extension_info().clone(),
                    backend,
                    process.filename(),
                    process.bytes(),
                    process.staging_directory(),
                    working_directory,
                )
            } else {
                ProcessLaunch::from_verified_bytes_in(
                    process.extension_info().clone(),
                    process.filename(),
                    process.bytes(),
                    process.staging_directory(),
                    working_directory,
                )
            };
            let launch = process
                .args()
                .iter()
                .fold(launch, |launch, argument| launch.arg(argument));
            Box::new(SpawnedProcessTransport::spawn(launch).await?)
        }
        VerifiedExtensionArtifact::Wasm(wasm) => {
            let container = ExtensionContainer::from_bytes_async(
                wasm.extension_info().id.clone(),
                wasm.bytes().to_vec(),
                wasm_host_functions(working_directory),
            )
            .await?;
            Box::new(ExtismTransport::new_with_expected_backend_capability(
                container,
                wasm.extension_info().clone(),
                wasm.extension_capabilities().backend,
            ))
        }
    };
    Ok(Session::loaded(transport))
}

fn wasm_host_functions(working_directory: &Path) -> MorphirHostFunctions {
    MorphirHostFunctions::for_restricted_generation(working_directory.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{BoxedMepTransport, activate_transport, wasm_host_functions};
    use crate::extensions::{Loaded, MepTransport, Session};
    use morphir_common::home::MorphirHome;
    use morphir_distribution::{
        Channel, ExtensionId, ExtensionInstaller, LocalIndex, Platform, Selection, Sha256Digest,
        VerifiedExtensionArtifact, activate_installed,
    };
    use morphir_extension_sdk::protocol::{InitializeParams, PeerInfo};
    use morphir_extension_sdk::{BackendCapability, ExtensionType};
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

        struct ArtifactSpec<'a> {
            id: &'a str,
            name: &'a str,
            runtime: &'a str,
            filename: &'a str,
            bytes: &'a [u8],
            args: &'a [String],
            lock_backend: bool,
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
                    lock_backend: true,
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
                    lock_backend: false,
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
                    lock_backend: true,
                },
            )
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
            let mut record = serde_json::json!({
                "schemaVersion": if spec.lock_backend { 2 } else { 1 },
                "id": spec.id,
                "name": spec.name,
                "version": "1.2.3",
                "channels": ["stable"],
                "mepVersions": ["0.1"],
                "capabilities": ["backend"],
                "artifacts": [artifact]
            });
            if spec.lock_backend {
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

            let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None)
                .unwrap();
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
            let info = guest_info.to_string();
            let initialize_response = serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": {
                    "protocolVersion": "0.1",
                    "extension": guest_info,
                    "capabilities": {}
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

    #[test]
    fn installed_wasm_activation_uses_restricted_generation_host_policy() {
        let workspace = tempfile::tempdir().unwrap();

        let host = wasm_host_functions(workspace.path());

        assert!(host.workspace_info().output_dir.is_empty());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn process_activation_preserves_discovery_capabilities_and_exact_launch() {
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
        assert_eq!(discovered.types, [ExtensionType::Backend]);
        assert_eq!(
            expected.backend_capability(),
            Some(&expected_backend_capability())
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
        assert_eq!(discovered.types, [ExtensionType::Backend]);
        assert_eq!(
            expected.backend_capability(),
            Some(&expected_backend_capability())
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
}
