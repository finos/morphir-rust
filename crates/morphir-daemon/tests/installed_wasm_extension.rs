//! Lifecycle conformance for a release-built installed WebAssembly extension.
//!
//! Build the fixture before running this ignored test:
//!
//! `cargo build --release -p morphir-avro-extension --target wasm32-unknown-unknown`
//! `cargo test -p morphir-daemon --test installed_wasm_extension -- --ignored`

use morphir_common::home::MorphirHome;
use morphir_daemon::extensions::{InvokeOutcome, activate_transport, protocol::methods};
use morphir_distribution::{
    Channel, ExtensionId, ExtensionInstaller, InstalledExtension, LocalIndex, Platform, Selection,
    Sha256Digest, activate_installed,
};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

use morphir_extension_sdk::{
    DiagnosticSeverity, ExtensionType, GenerateRequest, GenerateResult,
    protocol::{InitializeParams, PeerInfo},
};

struct InstalledWasmMother {
    _root: TempDir,
    home: MorphirHome,
    index: PathBuf,
    workspace: TempDir,
    extension_id: ExtensionId,
}

impl InstalledWasmMother {
    fn from_path(guest_path: impl AsRef<Path>) -> Self {
        let guest_path = guest_path.as_ref();
        let bytes = fs::read(guest_path).unwrap_or_else(|error| {
            panic!(
                "the release Avro WebAssembly guest must exist at {}: {error}",
                guest_path.display()
            )
        });
        let root = tempfile::tempdir().expect("fixture root should be created");
        let index = root.path().join("index");
        let artifact_name = "morphir_avro_extension.wasm";
        let artifact_path = index.join("artifacts").join(artifact_name);
        fs::create_dir_all(artifact_path.parent().expect("artifact has a parent"))
            .expect("fixture artifact directory should be created");
        fs::create_dir_all(index.join("extensions"))
            .expect("fixture index directory should be created");
        fs::write(&artifact_path, &bytes).expect("the guest should be copied into the index");

        let extension_id = ExtensionId::parse("morphir-avro").expect("fixture ID should be valid");
        let digest = Sha256Digest::of_bytes(&bytes);
        let record = json!({
            "schemaVersion": 2,
            "id": extension_id,
            "name": "Morphir Avro Backend",
            "version": "0.1.0",
            "channels": ["stable"],
            "mepVersions": ["0.1"],
            "capabilities": ["backend"],
            "backend": { "targets": ["avro"], "irVersions": ["3", "4"] },
            "artifacts": [{
                "runtime": "wasm",
                "source": { "kind": "local-file", "path": format!("artifacts/{artifact_name}") },
                "sha256": digest,
                "filename": artifact_name
            }]
        });
        fs::write(
            index.join("extensions/morphir-avro.jsonl"),
            format!("{record}\n"),
        )
        .expect("the controlled index should contain the release record");

        let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None)
            .expect("fixture Morphir home should resolve");
        Self {
            _root: root,
            home,
            index,
            workspace: tempfile::tempdir().expect("fixture workspace should be created"),
            extension_id,
        }
    }

    fn install(&self) -> morphir_distribution::Result<InstalledExtension> {
        let selected = LocalIndex::open(&self.index)?.resolve(
            &self.extension_id,
            Selection::Channel(Channel::Stable),
            &Platform::current(),
        )?;
        ExtensionInstaller::new(&self.home).install(selected)
    }
}

fn wasm_guest_path() -> PathBuf {
    cargo_target_directory()
        .join("wasm32-unknown-unknown")
        .join("release")
        .join("morphir_avro_extension.wasm")
}

fn cargo_target_directory() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .expect("cargo metadata should start");
    assert!(
        output.status.success(),
        "cargo metadata failed for {}: {}",
        manifest.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata should return JSON");
    let target = metadata["target_directory"]
        .as_str()
        .expect("cargo metadata should report target_directory");
    let target = PathBuf::from(target);
    assert!(
        target.is_absolute(),
        "cargo metadata should report an absolute target_directory: {}",
        target.display()
    );
    target
}

#[test]
fn cargo_metadata_resolves_an_absolute_target_directory() {
    assert!(cargo_target_directory().is_absolute());
}

#[tokio::test]
#[ignore = "requires a release wasm guest"]
async fn installed_wasm_runs_the_common_mep_lifecycle() {
    let fixture = InstalledWasmMother::from_path(wasm_guest_path());
    let installed = fixture
        .install()
        .expect("the release guest should install securely");
    let loaded = activate_transport(
        activate_installed(&fixture.home, installed.extension_id())
            .expect("the installed guest should activate offline"),
        fixture.workspace.path(),
    )
    .await
    .expect("the installed guest should load through the daemon");

    let ready = loaded
        .initialize(InitializeParams {
            protocol_versions: vec!["0.1".into()],
            host: PeerInfo {
                name: "morphir-installed-wasm-conformance".into(),
                version: "0.1.0".into(),
            },
        })
        .await
        .unwrap_or_else(|failure| panic!("MEP negotiation failed: {}", failure.error()));

    assert_eq!(ready.negotiated().protocol_version(), "0.1");
    let info = ready.negotiated().extension();
    assert_eq!(info.id, "morphir-avro");
    assert_eq!(info.name, "Morphir Avro Backend");
    assert_eq!(info.version, "0.1.0");
    assert_eq!(info.types, [ExtensionType::Backend]);
    let backend = ready
        .negotiated()
        .capabilities()
        .backend
        .as_ref()
        .expect("the locked backend capability should negotiate");
    assert_eq!(backend.targets, ["avro"]);
    assert_eq!(backend.ir_versions, ["3", "4"]);
    assert!(backend.generate);

    let valid_request = GenerateRequest {
        ir: supported_v4_distribution(),
        options: [
            ("representation".into(), json!("idl")),
            ("projection".into(), json!("protocol-public")),
            (
                "type_mappings".into(),
                json!({
                    "morphir/sdk:string#string": { "type": "bytes" },
                    "morphir/sdk:basics#int": { "type": "double" }
                }),
            ),
        ]
        .into_iter()
        .collect(),
    };
    let ready = match ready
        .invoke::<GenerateResult>(methods::GENERATE, valid_request.clone())
        .await
    {
        InvokeOutcome::Success(ready, generated) => {
            assert!(generated.success, "{:?}", generated.diagnostics);
            assert!(!generated.artifacts.is_empty());
            assert!(generated.artifacts.iter().all(|artifact| {
                artifact.path.ends_with(".avdl")
                    && !artifact.binary
                    && artifact.content.contains("protocol ")
            }));
            assert_configured_type_mappings(&generated);
            ready
        }
        InvokeOutcome::Rejected(_, error) => panic!("valid generation was rejected: {error}"),
        InvokeOutcome::Failed(failure) => {
            panic!("valid generation failed MEP: {}", failure.error())
        }
    };

    let invalid_request = GenerateRequest {
        ir: json!({ "formatVersion": 5, "distribution": null }),
        options: Default::default(),
    };
    let ready = match ready
        .invoke::<GenerateResult>(methods::GENERATE, invalid_request)
        .await
    {
        InvokeOutcome::Success(ready, generated) => {
            assert!(!generated.success);
            assert!(generated.artifacts.is_empty());
            assert_eq!(generated.diagnostics.len(), 1);
            assert_eq!(
                generated.diagnostics[0].code.as_deref(),
                Some("unsupported_format_version_major")
            );
            assert_eq!(generated.diagnostics[0].severity, DiagnosticSeverity::Error);
            ready
        }
        InvokeOutcome::Rejected(_, error) => {
            panic!("domain-invalid generation was rejected at MEP: {error}")
        }
        InvokeOutcome::Failed(failure) => {
            panic!("domain-invalid generation failed MEP: {}", failure.error())
        }
    };

    let ready = match ready
        .invoke::<GenerateResult>(methods::GENERATE, json!({ "options": "not an option map" }))
        .await
    {
        InvokeOutcome::Rejected(ready, error) => {
            assert!(
                error.to_string().contains("-32602"),
                "expected JSON-RPC invalid params, got {error}"
            );
            ready
        }
        InvokeOutcome::Success(_, _) => panic!("malformed generate params should be rejected"),
        InvokeOutcome::Failed(failure) => {
            panic!(
                "malformed params broke the MEP session: {}",
                failure.error()
            )
        }
    };

    let ready = match ready
        .invoke::<GenerateResult>(methods::GENERATE, valid_request)
        .await
    {
        InvokeOutcome::Success(ready, generated) => {
            assert!(generated.success, "{:?}", generated.diagnostics);
            assert!(generated.artifacts.iter().any(|artifact| {
                artifact.path.ends_with(".avdl")
                    && !artifact.binary
                    && artifact.content.contains("protocol ")
            }));
            assert_configured_type_mappings(&generated);
            ready
        }
        InvokeOutcome::Rejected(_, error) => {
            panic!("valid generation after rejection was rejected: {error}")
        }
        InvokeOutcome::Failed(failure) => {
            panic!(
                "valid generation after rejection failed MEP: {}",
                failure.error()
            )
        }
    };

    ready
        .shutdown()
        .await
        .unwrap_or_else(|failure| panic!("MEP shutdown failed: {}", failure.error()));
}

fn supported_v4_distribution() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../morphir-core/tests/fixtures/ir/v4/v4-library-distribution.json"
    ))
    .expect("the canonical v4 library distribution fixture should be valid JSON")
}

fn assert_configured_type_mappings(generated: &GenerateResult) {
    let idl = generated
        .artifacts
        .iter()
        .map(|artifact| artifact.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        idl.contains("bytes getUserName("),
        "configured SDK String mapping was not rendered: {idl}"
    );
    assert!(
        idl.contains("double nativeAdd(") && idl.contains("double a") && idl.contains("double b"),
        "configured SDK Int mapping was not rendered: {idl}"
    );
}
