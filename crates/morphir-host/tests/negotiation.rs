//! The handshake held to what discovery recorded, through `ExpectedChecks`.
//!
//! Ported from the daemon's session tests. Each guest answers only
//! `initialize`, so a refused handshake proves nothing else was sent.

use morphir_extension_sdk::protocol::{
    ExtensionResponse, InitializeResult, PeerInfo, PeerKind, methods,
};
use morphir_extension_sdk::{
    BackendCapability, ExtensionCapabilities, ExtensionInfo, ExtensionType, FrontendCapability,
    LanguageCapability, WorkspaceCapability,
};
use morphir_host::testing::{MemoryChannel, SentLog};
use morphir_host::{
    ExpectedChecks, ExpectedExtension, HostConfig, HostError, JsonRpcConnection,
    PersistedExtensionCapabilities, Session,
};

fn config() -> HostConfig {
    HostConfig::new(PeerInfo {
        kind: PeerKind::Unspecified,
        name: "test".into(),
        version: "1".into(),
    })
}

fn extension(types: Vec<ExtensionType>) -> ExtensionInfo {
    ExtensionInfo {
        id: "example".into(),
        name: "Example".into(),
        version: "1.0.0".into(),
        types,
        ..Default::default()
    }
}

fn initialization(info: ExtensionInfo) -> InitializeResult {
    InitializeResult {
        protocol_version: "0.1".into(),
        extension: info,
        capabilities: ExtensionCapabilities::default(),
    }
}

fn backend_initialization() -> InitializeResult {
    let mut result = initialization(extension(vec![ExtensionType::Backend]));
    result.capabilities.backend = Some(BackendCapability {
        targets: vec!["avro".into()],
        ir_versions: vec!["3".into(), "4".into()],
        generate: true,
    });
    result
}

/// Open a session on a guest that answers `initialize` with `initialized`.
async fn open(
    expected: ExpectedExtension,
    initialized: InitializeResult,
) -> (Result<Session, HostError>, SentLog) {
    let channel = MemoryChannel::new().respond(ExtensionResponse::success(1, initialized).unwrap());
    let log = channel.log();
    let connection = JsonRpcConnection::new(channel, ExpectedChecks::new(expected));
    (Session::open(connection, &config()).await, log)
}

/// The error of a handshake that must be refused. The refusal aborts the
/// guest after `initialize`, and nothing else is sent.
async fn refused(expected: ExpectedExtension, initialized: InitializeResult) -> String {
    let (result, log) = open(expected, initialized).await;
    let Err(error) = result else {
        panic!("the handshake should be refused");
    };
    assert_eq!(log.methods(), [methods::INITIALIZE]);
    assert_eq!(log.aborts(), 1, "a refused handshake aborts the guest");
    error.to_string()
}

#[tokio::test]
async fn rejects_capability_drift_from_discovery() {
    let message = refused(
        ExpectedExtension::discovered(extension(vec![ExtensionType::Backend])),
        initialization(extension(vec![ExtensionType::Frontend])),
    )
    .await;

    assert!(message.contains("disagreed with discovery"), "{message}");
    assert!(message.contains("capability kinds changed"), "{message}");
}

#[tokio::test]
async fn accepts_display_name_drift_from_discovery() {
    let mut initialized = backend_initialization();
    initialized.extension.name = "Example Display Name".into();

    let (result, _) = open(
        ExpectedExtension::discovered(extension(vec![ExtensionType::Backend])),
        initialized,
    )
    .await;

    if let Err(error) = result {
        panic!("a display name is presentation metadata, not a negotiation term: {error}");
    }
}

#[tokio::test]
async fn rejects_duplicate_capability_kinds() {
    let message = refused(
        ExpectedExtension::identified("example"),
        initialization(extension(vec![
            ExtensionType::Validator,
            ExtensionType::Validator,
        ])),
    )
    .await;

    assert!(message.contains("repeated a capability"), "{message}");
}

#[tokio::test]
async fn rejects_declared_frontend_without_frontend_capabilities() {
    let message = refused(
        ExpectedExtension::identified("example"),
        initialization(extension(vec![ExtensionType::Frontend])),
    )
    .await;

    assert!(
        message.contains("declared Frontend without frontend capabilities"),
        "{message}"
    );
}

#[tokio::test]
async fn rejects_frontend_capabilities_without_declared_frontend() {
    let mut initialized = initialization(extension(vec![ExtensionType::Validator]));
    initialized.capabilities.frontend = Some(FrontendCapability::default());

    let message = refused(ExpectedExtension::identified("example"), initialized).await;

    assert!(
        message.contains("frontend capabilities without declaring Frontend"),
        "{message}"
    );
}

#[tokio::test]
async fn rejects_declared_backend_without_backend_capabilities() {
    let message = refused(
        ExpectedExtension::identified("example"),
        initialization(extension(vec![ExtensionType::Backend])),
    )
    .await;

    assert!(
        message.contains("declared Backend without backend capabilities"),
        "{message}"
    );
}

#[tokio::test]
async fn rejects_backend_capabilities_without_declared_backend() {
    let mut initialized = initialization(extension(vec![ExtensionType::Validator]));
    initialized.capabilities.backend = Some(BackendCapability {
        targets: vec!["avro".into()],
        ir_versions: vec!["3".into(), "4".into()],
        generate: true,
    });

    let message = refused(ExpectedExtension::identified("example"), initialized).await;

    assert!(
        message.contains("backend capabilities without declaring Backend"),
        "{message}"
    );
}

#[tokio::test]
async fn rejects_declared_workspace_without_workspace_capabilities() {
    let message = refused(
        ExpectedExtension::identified("example"),
        initialization(extension(vec![ExtensionType::Workspace])),
    )
    .await;

    assert!(
        message.contains("declared Workspace without workspace capabilities"),
        "{message}"
    );
}

#[tokio::test]
async fn rejects_workspace_capabilities_without_declared_workspace() {
    let mut initialized = initialization(extension(vec![ExtensionType::Validator]));
    initialized.capabilities.workspace = Some(WorkspaceCapability {
        protocol_versions: vec![morphir_workspace::workspace_discovery_protocol()],
        discover: true,
    });

    let message = refused(ExpectedExtension::identified("example"), initialized).await;

    assert!(
        message.contains("workspace capabilities without declaring Workspace"),
        "{message}"
    );
}

#[tokio::test]
async fn rejects_each_backend_metadata_drift_before_generate() {
    let locked_backend = BackendCapability {
        targets: vec!["avro".into(), "json-schema".into()],
        ir_versions: vec!["3".into(), "4".into()],
        generate: true,
    };
    let cases = [
        ("missing backend", None),
        (
            "changed target",
            Some(BackendCapability {
                targets: vec!["avro".into(), "protobuf".into()],
                ..locked_backend.clone()
            }),
        ),
        (
            "changed target order",
            Some(BackendCapability {
                targets: vec!["json-schema".into(), "avro".into()],
                ..locked_backend.clone()
            }),
        ),
        (
            "changed IR version",
            Some(BackendCapability {
                ir_versions: vec!["3".into(), "5".into()],
                ..locked_backend.clone()
            }),
        ),
        (
            "changed IR version order",
            Some(BackendCapability {
                ir_versions: vec!["4".into(), "3".into()],
                ..locked_backend.clone()
            }),
        ),
        (
            "changed generate flag",
            Some(BackendCapability {
                generate: false,
                ..locked_backend.clone()
            }),
        ),
    ];

    for (case, initialized_backend) in cases {
        let locked = ExtensionCapabilities {
            backend: Some(locked_backend.clone()),
            ..ExtensionCapabilities::default()
        };
        let mut initialized = initialization(extension(vec![ExtensionType::Backend]));
        initialized.capabilities.backend = initialized_backend;

        let message = refused(
            ExpectedExtension::discovered_with_capabilities(
                extension(vec![ExtensionType::Backend]),
                locked,
            ),
            initialized,
        )
        .await;

        assert!(
            message.contains("backend capabilities disagreed with discovery"),
            "{case}: {message}"
        );
    }
}

#[tokio::test]
async fn rejects_each_non_backend_locked_capability_drift() {
    let locked = ExtensionCapabilities {
        frontend: Some(FrontendCapability {
            compile: true,
            ..FrontendCapability::default()
        }),
        streaming: true,
        extra: [("vendor.feature".to_owned(), serde_json::json!("locked"))]
            .into_iter()
            .collect(),
        ..ExtensionCapabilities::default()
    };
    let mut changed_frontend = locked.clone();
    changed_frontend.frontend.as_mut().unwrap().compile = false;
    let mut changed_streaming = locked.clone();
    changed_streaming.streaming = false;
    let mut changed_extra = locked.clone();
    changed_extra
        .extra
        .insert("vendor.feature".to_owned(), serde_json::json!("changed"));

    for (case, capabilities) in [
        ("frontend", changed_frontend),
        ("streaming", changed_streaming),
        ("extension-specific", changed_extra),
    ] {
        let mut initialized = initialization(extension(vec![ExtensionType::Frontend]));
        initialized.capabilities = capabilities;

        let message = refused(
            ExpectedExtension::discovered_with_capabilities(
                extension(vec![ExtensionType::Frontend]),
                locked.clone(),
            ),
            initialized,
        )
        .await;

        assert!(
            message.contains("capabilities disagreed with discovery"),
            "{case}: {message}"
        );
    }
}

/// A refused session says which member disagreed. An installed record that
/// predates incremental frontends reads back as `incremental: false`, and the
/// guest that says otherwise is stopped; the message has to name the member, or
/// a reader is left comparing two structures by hand.
#[tokio::test]
async fn a_capability_mismatch_names_the_members_that_differ() {
    let persisted = FrontendCapability {
        languages: vec![LanguageCapability {
            id: "elm".into(),
            file_extensions: vec![".elm".into()],
        }],
        ir_versions: vec!["3".into(), "4".into()],
        compile: true,
        incremental: false,
        fragments: false,
        multi_document: false,
    };
    let mut advertised = persisted.clone();
    advertised.incremental = true;
    let mut initialized = initialization(extension(vec![ExtensionType::Frontend]));
    initialized.capabilities = ExtensionCapabilities {
        frontend: Some(advertised),
        ..ExtensionCapabilities::default()
    };

    let message = refused(
        ExpectedExtension::discovered_with_persisted_capabilities(
            extension(vec![ExtensionType::Frontend]),
            PersistedExtensionCapabilities::new(Some(persisted), None),
        ),
        initialized,
    )
    .await;

    assert!(
        message.contains("frontend capabilities disagreed with discovery"),
        "{message}"
    );
    assert!(message.contains("incremental"), "{message}");
    // The members that agree are not named.
    assert!(!message.contains("languages"), "{message}");
    assert!(!message.contains("compile"), "{message}");
}

#[tokio::test]
async fn backend_only_lock_accepts_unpersisted_capabilities() {
    let backend = BackendCapability {
        targets: vec!["avro".into()],
        ir_versions: vec!["4".into()],
        generate: true,
    };
    let mut initialized = initialization(extension(vec![
        ExtensionType::Backend,
        ExtensionType::Frontend,
    ]));
    initialized.capabilities = ExtensionCapabilities {
        backend: Some(backend.clone()),
        frontend: Some(FrontendCapability {
            compile: true,
            ..FrontendCapability::default()
        }),
        streaming: true,
        extra: [("vendor.feature".to_owned(), serde_json::json!("guest"))]
            .into_iter()
            .collect(),
        ..ExtensionCapabilities::default()
    };

    let (result, _) = open(
        ExpectedExtension::discovered_with_backend_capability(
            extension(vec![ExtensionType::Backend, ExtensionType::Frontend]),
            backend,
        ),
        initialized,
    )
    .await;

    let session = result.unwrap_or_else(|error| {
        panic!("capabilities absent from installed metadata must remain negotiable: {error}")
    });
    assert!(session.negotiated().capabilities().streaming);
    assert!(session.negotiated().capabilities().frontend.is_some());
}

#[tokio::test]
async fn backend_only_lock_rejects_backend_drift() {
    let locked = BackendCapability {
        targets: vec!["avro".into()],
        ir_versions: vec!["4".into()],
        generate: true,
    };
    let mut initialized = backend_initialization();
    initialized.capabilities.backend.as_mut().unwrap().targets = vec!["json-schema".into()];

    let message = refused(
        ExpectedExtension::discovered_with_backend_capability(
            extension(vec![ExtensionType::Backend]),
            locked,
        ),
        initialized,
    )
    .await;

    assert!(
        message.contains("backend capabilities disagreed with discovery"),
        "{message}"
    );
}

#[tokio::test]
async fn discovered_backend_without_locked_metadata_accepts_valid_initialization() {
    let (result, log) = open(
        ExpectedExtension::discovered(extension(vec![ExtensionType::Backend])),
        backend_initialization(),
    )
    .await;

    result.unwrap_or_else(|error| panic!("initialization failed: {error}"));
    assert_eq!(log.methods(), [methods::INITIALIZE]);
}

#[tokio::test]
async fn ordinary_discovery_rejects_a_backend_without_typed_capabilities() {
    let message = refused(
        ExpectedExtension::discovered(extension(vec![ExtensionType::Backend])),
        initialization(extension(vec![ExtensionType::Backend])),
    )
    .await;

    assert!(
        message.contains("declared Backend without backend capabilities"),
        "ordinary discovery must not enable schema-v1 compatibility: {message}"
    );
}
