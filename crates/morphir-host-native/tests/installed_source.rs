//! `InstalledSource`: runtime-to-mode mapping, claims held from a real v2
//! record, IR version and enablement gating, workspace discovery from
//! persisted types, duplicate installed IDs, and a real session opened
//! through `Registry::resolve_frontend` -> `Resolved::connect`.
//!
//! The plain-record fixtures below are ported from the daemon's
//! `provider_registry` tests (`crates/morphir-daemon/tests/provider_registry.rs`)
//! without a dependency on `morphir-daemon`. The live-guest tests reuse
//! `tests/support/activation.rs`, shared with `tests/activation.rs`.

#[allow(dead_code)]
#[path = "support/activation.rs"]
mod runtime_mother;

use morphir_common::home::MorphirHome;
use morphir_distribution::{
    Channel, ExtensionId, ExtensionInstaller, InstalledExtensionSnapshot, LocalIndex, Platform,
    Selection, Sha256Digest, list_installed,
};
use morphir_extension_sdk::ExtensionType;
use morphir_extension_sdk::protocol::{PeerInfo, PeerKind};
use morphir_host::{
    CapabilityMetadataScope, GuestSource, HostConfig, HostError, InvocationMode, InvocationPolicy,
    ProviderOrigin, Registry, Session,
};
use morphir_host_native::{InstalledSource, InstalledSourceError};
use runtime_mother::InstalledFrontend;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

#[derive(Clone, Copy)]
enum Runtime {
    Process,
    Wasm,
}

struct Spec<'a> {
    id: &'a str,
    runtime: Runtime,
    frontend: Option<(&'a str, &'a str, bool)>,
    backend: Option<(&'a str, &'a str, bool)>,
}

/// A real installed snapshot below its own Morphir home, kept alive by
/// holding the temporary directory it was installed into.
struct Fixture {
    _root: TempDir,
    home: MorphirHome,
    snapshot: InstalledExtensionSnapshot,
    working_directory: std::path::PathBuf,
}

/// Build a real installed snapshot from a minimal schema `"1.0"` record, the
/// way the daemon's `provider_registry` fixtures do.
fn installed(spec: Spec<'_>) -> Fixture {
    installed_with_record(spec, |_| {})
}

fn installed_with_record(spec: Spec<'_>, update: impl FnOnce(&mut serde_json::Value)) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let index = root.path().join("index");
    let filename = match spec.runtime {
        Runtime::Process => spec.id.to_owned(),
        Runtime::Wasm => format!("{}.wasm", spec.id),
    };
    let source = index.join("artifacts").join(&filename);
    fs::create_dir_all(source.parent().unwrap()).unwrap();
    fs::create_dir_all(index.join("extensions")).unwrap();
    let bytes = match spec.runtime {
        Runtime::Process => b"#!/bin/sh\nexit 0\n".as_slice(),
        Runtime::Wasm => b"portable test wasm".as_slice(),
    };
    fs::write(&source, bytes).unwrap();
    let digest = Sha256Digest::of_bytes(bytes);
    let platform = Platform::current();
    let artifact = match spec.runtime {
        Runtime::Process => serde_json::json!({
            "runtime": "process",
            "platform": { "os": platform.os(), "arch": platform.arch() },
            "source": { "kind": "local-file", "path": format!("artifacts/{filename}") },
            "sha256": digest,
            "filename": filename,
            "args": [],
            "executable": true
        }),
        Runtime::Wasm => serde_json::json!({
            "runtime": "wasm",
            "source": { "kind": "local-file", "path": format!("artifacts/{filename}") },
            "sha256": digest,
            "filename": filename
        }),
    };
    let capabilities: Vec<&str> = [
        spec.frontend.map(|_| "frontend"),
        spec.backend.map(|_| "backend"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let mut record = serde_json::json!({
        "schemaVersion": "1.0",
        "id": spec.id,
        "name": format!("Installed {}", spec.id),
        "version": "2.0.0",
        "channels": ["stable"],
        "mepVersions": ["0.1"],
        "capabilities": capabilities,
        "artifacts": [artifact]
    });
    if let Some((language, ir_version, compile)) = spec.frontend {
        record.as_object_mut().unwrap().insert(
            "frontend".into(),
            serde_json::json!({
                "languages": [{
                    "id": language,
                    "fileExtensions": [format!(".{language}")]
                }],
                "irVersions": [ir_version],
                "compile": compile
            }),
        );
    }
    if let Some((target, ir_version, generate)) = spec.backend {
        record.as_object_mut().unwrap().insert(
            "backend".into(),
            serde_json::json!({
                "targets": [target],
                "irVersions": [ir_version],
                "generate": generate
            }),
        );
    }
    update(&mut record);
    fs::write(
        index.join("extensions").join(format!("{}.jsonl", spec.id)),
        format!("{record}\n"),
    )
    .unwrap();

    let home = MorphirHome::resolve_from(Some(root.path().join("home").as_os_str()), None).unwrap();
    let id = ExtensionId::parse(spec.id).unwrap();
    let selected = LocalIndex::open(&index)
        .unwrap()
        .resolve(
            &id,
            Selection::Channel(Channel::Stable),
            &platform,
            &"0.4.0".parse().unwrap(),
        )
        .unwrap();
    ExtensionInstaller::new(&home)
        .install(selected, &"0.4.0".parse().unwrap())
        .unwrap();
    let snapshot = list_installed(&home).unwrap().pop().unwrap();
    let working_directory = root.path().join("workspace");
    fs::create_dir(&working_directory).unwrap();

    Fixture {
        _root: root,
        home,
        snapshot,
        working_directory,
    }
}

fn process_provider(id: &str, language: &str, target: &str) -> Fixture {
    installed(Spec {
        id,
        runtime: Runtime::Process,
        frontend: Some((language, "4", true)),
        backend: Some((target, "4.0.0", true)),
    })
}

fn source_of(fixture: &Fixture) -> Arc<InstalledSource> {
    Arc::new(InstalledSource::new(
        fixture.home.clone(),
        fixture.snapshot.clone(),
    ))
}

fn host_config() -> HostConfig {
    HostConfig::new(PeerInfo {
        kind: PeerKind::Unspecified,
        name: "installed-source-test".into(),
        version: "1.0.0".into(),
    })
}

// -- Pickup item: runtime -> mode, under both policies, and
// `ProviderMetadata::preferred_invocation_mode()` agrees. --

#[test]
fn process_runtime_reports_process_mep_under_every_policy() {
    let fixture = process_provider("runtime-process", "runtime-lang", "runtime-target");
    let source = InstalledSource::new(fixture.home.clone(), fixture.snapshot.clone());

    assert_eq!(
        source.invocation_mode(InvocationPolicy::PreferDirect),
        InvocationMode::ProcessMep
    );
    assert_eq!(
        source.invocation_mode(InvocationPolicy::ProtocolOnly),
        InvocationMode::ProcessMep
    );

    let mut registry = Registry::new();
    registry.register(Arc::new(source)).unwrap();
    let listed = registry.providers();
    assert_eq!(
        listed[0].preferred_invocation_mode(),
        InvocationMode::ProcessMep
    );
}

#[test]
fn wasm_runtime_reports_wasm_mep_under_every_policy() {
    let fixture = installed(Spec {
        id: "runtime-wasm",
        runtime: Runtime::Wasm,
        frontend: Some(("wasm-lang", "4", true)),
        backend: Some(("wasm-target", "4.0.0", true)),
    });
    let source = InstalledSource::new(fixture.home.clone(), fixture.snapshot.clone());

    assert_eq!(
        source.invocation_mode(InvocationPolicy::PreferDirect),
        InvocationMode::WasmMep
    );
    assert_eq!(
        source.invocation_mode(InvocationPolicy::ProtocolOnly),
        InvocationMode::WasmMep
    );

    let mut registry = Registry::new();
    registry.register(Arc::new(source)).unwrap();
    let listed = registry.providers();
    assert_eq!(
        listed[0].preferred_invocation_mode(),
        InvocationMode::WasmMep
    );
}

// -- Pickup item: origin Installed, scope PersistedFrontendBackend, and the
// snapshot() accessor, for a real snapshot. --

#[test]
fn a_real_installed_source_reports_installed_origin_and_persisted_scope() {
    let fixture = process_provider("origin-scope", "origin-lang", "origin-target");
    let source = InstalledSource::new(fixture.home.clone(), fixture.snapshot.clone());

    assert_eq!(source.origin(), ProviderOrigin::Installed);
    assert_eq!(
        source.capability_metadata_scope(),
        CapabilityMetadataScope::PersistedFrontendBackend
    );
    assert_eq!(source.snapshot(), &fixture.snapshot);
}

// -- Pickup item: duplicate Installed id text for real snapshots. --

#[test]
fn duplicate_installed_ids_are_rejected_for_real_snapshots() {
    let fixture = process_provider("duplicate-real", "dup-lang", "dup-target");
    let mut registry = Registry::new();
    registry.register(source_of(&fixture)).unwrap();

    let error = registry
        .register(source_of(&fixture))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("duplicate Installed provider ID 'duplicate-real'"),
        "{error}"
    );
}

// -- Pickup item: compile=false / generate=false installed providers do not
// resolve; irVersions "4" and "4.0.0" through a real record. --

#[test]
fn disabled_compile_and_generate_do_not_resolve_for_a_real_snapshot() {
    let fixture = installed(Spec {
        id: "disabled-real",
        runtime: Runtime::Process,
        frontend: Some(("disabled-lang", "4", false)),
        backend: Some(("disabled-target", "4", false)),
    });
    let mut registry = Registry::new();
    registry.register(source_of(&fixture)).unwrap();

    assert!(
        registry
            .resolve_frontend("disabled-lang", "4", InvocationPolicy::PreferDirect)
            .is_err()
    );
    assert!(
        registry
            .resolve_backend("disabled-target", "4", InvocationPolicy::PreferDirect)
            .is_err()
    );
}

#[test]
fn major_alias_and_exact_ir_versions_resolve_in_both_directions_for_real_snapshots() {
    let alias = installed(Spec {
        id: "alias-real",
        runtime: Runtime::Process,
        frontend: Some(("alias-lang", "4", true)),
        backend: Some(("alias-target", "4", true)),
    });
    let exact = installed(Spec {
        id: "exact-real",
        runtime: Runtime::Process,
        frontend: Some(("exact-lang", "4.0.0", true)),
        backend: Some(("exact-target", "4.0.0", true)),
    });
    let mut registry = Registry::new();
    registry.register(source_of(&alias)).unwrap();
    registry.register(source_of(&exact)).unwrap();

    assert_eq!(
        registry
            .resolve_frontend("exact-lang", "4", InvocationPolicy::PreferDirect)
            .unwrap()
            .info()
            .id,
        "exact-real"
    );
    assert_eq!(
        registry
            .resolve_backend("alias-target", "4.0.0", InvocationPolicy::PreferDirect)
            .unwrap()
            .info()
            .id,
        "alias-real"
    );
}

// -- Review item: a reinstall under the same id and version with other
// args opens a new pooled guest, so its fingerprint differs. --

#[test]
fn a_same_version_reinstall_with_other_args_has_another_fingerprint() {
    let spec = || Spec {
        id: "reinstalled",
        runtime: Runtime::Process,
        frontend: Some(("reinstalled-lang", "4", true)),
        backend: None,
    };
    let before = installed(spec());
    let after = installed_with_record(spec(), |record| {
        record["artifacts"][0]["args"] = serde_json::json!(["--verbose"]);
    });
    assert_eq!(
        before.snapshot.installed().extension_info().version,
        after.snapshot.installed().extension_info().version
    );
    assert_ne!(before.snapshot.installed(), after.snapshot.installed());

    let fingerprint_of = |fixture: &Fixture| {
        let mut registry = Registry::new();
        registry.register(source_of(fixture)).unwrap();
        registry
            .resolve_frontend("reinstalled-lang", "4", InvocationPolicy::PreferDirect)
            .unwrap()
            .fingerprint()
    };

    assert_ne!(fingerprint_of(&before), fingerprint_of(&after));
    assert_eq!(fingerprint_of(&before), fingerprint_of(&before));
}

// -- Pickup item: `info.types` from the record so
// `supports_workspace_discovery()` answers correctly for a real installed
// provider. --

#[test]
fn workspace_discovery_follows_the_persisted_types_for_a_real_snapshot() {
    let plain = process_provider("workspace-plain", "plain-lang", "plain-target");
    let discovering = installed_with_record(
        Spec {
            id: "workspace-discovering",
            runtime: Runtime::Process,
            frontend: Some(("discovering-lang", "4", true)),
            backend: None,
        },
        |record| {
            record["capabilities"] = serde_json::json!(["frontend", "workspace"]);
        },
    );
    assert!(
        discovering
            .snapshot
            .installed()
            .extension_info()
            .types
            .contains(&ExtensionType::Workspace)
    );

    let mut registry = Registry::new();
    registry.register(source_of(&plain)).unwrap();
    registry.register(source_of(&discovering)).unwrap();

    assert!(
        !registry
            .resolve_frontend("plain-lang", "4", InvocationPolicy::PreferDirect)
            .unwrap()
            .supports_workspace_discovery()
    );
    assert!(
        registry
            .resolve_frontend("discovering-lang", "4", InvocationPolicy::PreferDirect)
            .unwrap()
            .supports_workspace_discovery()
    );
}

// -- Pickup item: claims from a `schemaVersion 2.0.0-draft.2` record with
// `multiDocument`, `fragments`, `incremental` kept and a `futureMember`
// accepted, for both runtimes, resolvable for multiple documents. --

#[cfg(unix)]
#[test]
fn process_claims_keep_multi_document_fragments_incremental_and_accept_a_future_member() {
    let fixture = runtime_mother::process_with_multi_document(InstalledFrontend::ClaimsWithExtras);
    let mut registry = Registry::new();
    registry.register(source_of_runtime(&fixture)).unwrap();

    let frontend = registry
        .resolve_frontend("gleam", "4", InvocationPolicy::ProtocolOnly)
        .unwrap();
    assert_eq!(frontend.origin(), ProviderOrigin::Installed);
    let capability = frontend.frontend().unwrap();
    assert!(capability.multi_document, "multiDocument should be kept");
    assert!(capability.fragments, "fragments should be kept");
    assert!(capability.incremental, "incremental should be kept");
}

#[test]
fn wasm_claims_keep_multi_document_fragments_incremental_and_accept_a_future_member() {
    let fixture = runtime_mother::wasm_with_multi_document(InstalledFrontend::ClaimsWithExtras);
    let mut registry = Registry::new();
    registry.register(source_of_runtime(&fixture)).unwrap();

    let frontend = registry
        .resolve_frontend("gleam", "4", InvocationPolicy::ProtocolOnly)
        .unwrap();
    assert_eq!(frontend.origin(), ProviderOrigin::Installed);
    let capability = frontend.frontend().unwrap();
    assert!(capability.multi_document, "multiDocument should be kept");
    assert!(capability.fragments, "fragments should be kept");
    assert!(capability.incremental, "incremental should be kept");
}

fn source_of_runtime(fixture: &runtime_mother::RuntimeArtifact) -> Arc<InstalledSource> {
    Arc::new(InstalledSource::new(
        fixture.home.clone(),
        fixture.snapshot.clone(),
    ))
}

// -- Pickup item: register a real `InstalledSource` in a `Registry`, resolve
// it, and open a `Session` through `resolved.connect(..)` (unix process
// fixture), checking negotiated capabilities equal the installed metadata;
// and reject persisted-capability drift with the same text `activate`
// produces. --

#[cfg(unix)]
#[tokio::test]
async fn a_real_installed_source_connects_and_negotiates_the_installed_metadata() {
    let fixture =
        runtime_mother::process_with_multi_document(InstalledFrontend::ClaimsWithMultiDocument);
    let mut registry = Registry::new();
    registry.register(source_of_runtime(&fixture)).unwrap();

    let resolved = registry
        .resolve_frontend("gleam", "4", InvocationPolicy::ProtocolOnly)
        .unwrap();
    assert_eq!(resolved.invocation_mode(), InvocationMode::ProcessMep);

    let connection = resolved
        .connect(&fixture.working_directory)
        .await
        .expect("a real installed process should activate and connect");

    // The fixture's script answers exactly one request, so the test stops at
    // the negotiated handshake rather than a full shutdown round trip.
    let session = Session::open(connection, &host_config())
        .await
        .expect("the handshake should negotiate against the persisted metadata");

    assert_eq!(
        session.negotiated().capabilities(),
        &fixture.snapshot.installed().extension_capabilities()
    );
}

/// A persisted-capability drift is caught by the handshake, not by
/// `connect` itself: `connect` only starts the guest. `InstalledSource`
/// wires the same artifact and workspace into `activate` that a direct call
/// would, so the handshake through `Session::open` fails with the identical
/// text either way.
#[cfg(unix)]
#[tokio::test]
async fn a_persisted_capability_drift_is_rejected_with_the_same_text_activate_produces() {
    let fixture =
        runtime_mother::process_with_multi_document(InstalledFrontend::ClaimsWithoutMultiDocument);

    // The reference failure: verify, activate, and open the same snapshot
    // directly, exactly as `crates/morphir-host-native/tests/activation.rs` does.
    let reference_artifact =
        morphir_distribution::activate_installed_snapshot(&fixture.home, &fixture.snapshot)
            .unwrap();
    let reference_guest =
        morphir_host_native::activate(reference_artifact, &fixture.working_directory)
            .await
            .unwrap();
    let reference_error = match Session::open(reference_guest.connection, &host_config()).await {
        Ok(_) => panic!("supplied claims default multiDocument to false and lock it"),
        Err(error) => error.to_string(),
    };

    // `InstalledSource::connect` followed by `Session::open` should fail
    // with the same text.
    let source = InstalledSource::new(fixture.home.clone(), fixture.snapshot.clone());
    let connection = source.connect(&fixture.working_directory).await.unwrap();
    let error = match Session::open(connection, &host_config()).await {
        Ok(_) => panic!("the same drift should be rejected through InstalledSource::connect"),
        Err(error) => error.to_string(),
    };

    assert_eq!(error, reference_error);
    assert!(
        error.contains("capabilities disagreed with discovery"),
        "{error}"
    );
    assert!(error.contains("multiDocument"), "{error}");
}

/// A failure inside `activate` itself -- not the handshake -- is wrapped
/// with the provider id, distinct from the verify-failure text.
#[tokio::test]
async fn connect_wraps_an_activate_failure_with_the_provider_id() {
    let fixture = installed(Spec {
        id: "activate-failure",
        runtime: Runtime::Wasm,
        frontend: Some(("broken-lang", "4", true)),
        backend: None,
    });
    let id = fixture.snapshot.installed().extension_info().id.clone();

    let source = InstalledSource::new(fixture.home.clone(), fixture.snapshot.clone());
    let error = match source.connect(&fixture.working_directory).await {
        Ok(_) => panic!("bytes that are not a WebAssembly module should fail to activate"),
        Err(error) => error.to_string(),
    };

    assert!(
        error.starts_with(&format!("Failed to activate installed provider '{id}': ")),
        "{error}"
    );
}

// -- The verify-failure text: "Failed to verify installed provider '{id}':
// {error}", for a snapshot whose stored bytes no longer match its digest. --

#[cfg(unix)]
#[tokio::test]
async fn connect_wraps_a_verify_failure_with_the_provider_id() {
    let (fixture, _capture, _args) = runtime_mother::process();
    let id = fixture.snapshot.installed().extension_info().id.clone();
    fs::write(&fixture.installed_path, b"corrupted after installation").unwrap();

    let source = InstalledSource::new(fixture.home.clone(), fixture.snapshot.clone());
    let error = match source.connect(&fixture.working_directory).await {
        Ok(_) => panic!("corrupted installed bytes should fail verification"),
        Err(error) => error.to_string(),
    };

    assert!(
        error.starts_with(&format!("Failed to verify installed provider '{id}': ")),
        "{error}"
    );
    assert!(error.contains("digest mismatch"), "{error}");
}

// -- `InstalledSource::activate` keeps the typed failure `connect` flattens:
// a caller that words its own texts (the CLI's workspace provider) needs the
// inner `HostError` variant, its channel state and cause, and the
// distribution error, not a pre-formatted string. --

#[tokio::test]
async fn activate_keeps_an_invalid_activation_error_as_its_own_variant() {
    let fixture = installed(Spec {
        id: "typed-activate-failure",
        runtime: Runtime::Wasm,
        frontend: Some(("broken-lang", "4", true)),
        backend: None,
    });
    let id = fixture.snapshot.installed().extension_info().id.clone();

    let source = InstalledSource::new(fixture.home.clone(), fixture.snapshot.clone());
    let error = match source.activate(&fixture.working_directory).await {
        Ok(_) => panic!("bytes that are not a WebAssembly module should fail to activate"),
        Err(error) => error,
    };

    let text = error.to_string();
    match error {
        InstalledSourceError::Activate { id: failed, error } => {
            let HostError::Invalid(message) = *error else {
                panic!("expected the inner Invalid variant, got {error:?}");
            };
            assert_eq!(failed, id);
            assert_eq!(
                text,
                format!("Failed to activate installed provider '{id}': {message}")
            );
        }
        other => panic!("expected Activate(Invalid), got {other:?}"),
    }
}

#[cfg(unix)]
#[tokio::test]
async fn activate_reports_a_verify_failure_as_verify() {
    let (fixture, _capture, _args) = runtime_mother::process();
    let id = fixture.snapshot.installed().extension_info().id.clone();
    fs::write(&fixture.installed_path, b"corrupted after installation").unwrap();

    let source = InstalledSource::new(fixture.home.clone(), fixture.snapshot.clone());
    let error = match source.activate(&fixture.working_directory).await {
        Ok(_) => panic!("corrupted installed bytes should fail verification"),
        Err(error) => error,
    };

    let text = error.to_string();
    match error {
        InstalledSourceError::Verify { id: failed, error } => {
            assert_eq!(failed, id);
            assert_eq!(
                text,
                format!("Failed to verify installed provider '{id}': {error}")
            );
            assert!(error.to_string().contains("digest mismatch"), "{error}");
        }
        other => panic!("expected Verify, got {other:?}"),
    }
}
