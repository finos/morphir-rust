//! Shared MEP conformance drivers for real extension guests.
//!
//! Each driver runs a guest through `morphir_host::Session`: the handshake,
//! typed calls, and an orderly close. A process guest runs behind
//! [`SharedProcess`], so a test can still ask whether the child exited after
//! the session released it.

use async_trait::async_trait;
use morphir_extension_sdk::protocol::{ExtensionResponse, PeerInfo};
use morphir_extension_sdk::{
    CompileRequest, DiagnosticSeverity, ExtensionType, GenerateRequest, GenerateResult,
};
use morphir_host::{
    CallError, Channel, ChannelError, ChannelState, ExpectedChecks, GuestConnection, HostConfig,
    HostError, JsonRpcConnection, Outgoing, Session,
};
use morphir_host_native::CheckedConnection;
use morphir_host_native::process::{ProcessChannel, ProcessLaunch};
use std::sync::Arc;
use tokio::sync::Mutex;

/// The host a test presents in the handshake.
pub fn host_config(name: &str, version: &str) -> HostConfig {
    HostConfig::new(PeerInfo {
        kind: Default::default(),
        name: name.into(),
        version: version.into(),
    })
}

/// The result of a call that must complete, or a panic that names how it did not.
pub fn completed<T>(what: &str, result: Result<T, CallError>) -> T {
    match result {
        Ok(value) => value,
        Err(CallError::Rejected(error)) => panic!("{what} was rejected: {error}"),
        Err(CallError::Invalid(error)) => panic!("{what} returned an invalid result: {error}"),
        Err(error) => panic!("{what} failed the MEP session: {error}"),
    }
}

/// A process channel the test keeps a handle to.
///
/// The session owns one clone and the test another, so the test can look at
/// the child after the session closed or aborted it.
#[derive(Clone)]
pub struct SharedProcess(Arc<Mutex<ProcessChannel>>);

impl SharedProcess {
    /// Start the process that `launch` describes.
    pub async fn spawn(launch: ProcessLaunch) -> Self {
        let channel = ProcessChannel::spawn(launch)
            .await
            .expect("the host should start the extension process");
        Self(Arc::new(Mutex::new(channel)))
    }

    /// A checked connection over this process, with the launch's expectation.
    pub async fn connection(
        &self,
    ) -> CheckedConnection<JsonRpcConnection<SharedProcess, ExpectedChecks>> {
        let expectation = self.0.lock().await.expectation();
        CheckedConnection::new(JsonRpcConnection::new(
            self.clone(),
            ExpectedChecks::new(expectation),
        ))
    }

    /// Whether the child is still running.
    pub async fn is_running(&self) -> bool {
        self.0
            .lock()
            .await
            .is_running()
            .expect("process status should be readable")
    }

    /// Standard error collected after the child exited.
    pub async fn stderr_output(&self) -> String {
        self.0.lock().await.stderr_output().to_owned()
    }

    /// Whether standard output has no unread bytes left.
    pub async fn stdout_is_exhausted(&self) -> bool {
        self.0
            .lock()
            .await
            .stdout_is_exhausted()
            .await
            .expect("extension stdout should be readable after shutdown")
    }
}

#[async_trait]
impl Channel for SharedProcess {
    async fn send(&mut self, message: Outgoing) -> Result<(), ChannelError> {
        self.0.lock().await.send(message).await
    }

    async fn receive(&mut self) -> Result<ExtensionResponse, ChannelError> {
        self.0.lock().await.receive().await
    }

    async fn close(&mut self) -> Result<ChannelState, ChannelError> {
        self.0.lock().await.close().await
    }

    async fn abort(&mut self) -> Result<ChannelState, ChannelError> {
        self.0.lock().await.abort().await
    }
}

/// The state a channel failure left the guest in, or a panic if the error
/// did not come from the channel.
pub fn channel_state(error: &HostError) -> ChannelState {
    match error {
        HostError::Channel { state, .. } => *state,
        other => panic!("expected a channel failure, got {other:?}"),
    }
}

/// Run a frontend process guest through one valid and one malformed compile.
///
/// Each compile gets a fresh process. After each orderly close the process
/// has exited and its stdout holds nothing but the frames the host read.
pub async fn frontend_conformance(
    launch: ProcessLaunch,
    valid_request: CompileRequest,
    malformed_request: CompileRequest,
) {
    let (process, mut session) = open_frontend(launch.clone()).await;
    let result = completed("valid Elm", session.compile(valid_request).await);
    assert!(result.success, "valid Elm should compile successfully");
    assert_eq!(result.ir_version.as_deref(), Some("3"));
    assert!(result.ir.is_some(), "a successful compile should return IR");
    assert!(
        result.modules.iter().any(|module| module == "Example"),
        "a successful compile should report the Example module"
    );
    close_frontend(process, session).await;

    let (process, mut session) = open_frontend(launch).await;
    let result = completed("malformed Elm", session.compile(malformed_request).await);
    assert!(!result.success, "malformed Elm should not compile");
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error),
        "malformed Elm should return an error diagnostic"
    );
    close_frontend(process, session).await;
}

async fn open_frontend(launch: ProcessLaunch) -> (SharedProcess, Session) {
    let process = SharedProcess::spawn(launch).await;
    let session = Session::open(
        process.connection().await,
        &host_config("morphir-conformance", "0.1.0"),
    )
    .await
    .unwrap_or_else(|error| panic!("MEP negotiation failed: {error}"));

    let negotiated = session.negotiated();
    assert_eq!(negotiated.protocol_version(), "0.1");
    assert_eq!(negotiated.extension().id, "morphir-elm");
    assert!(
        negotiated
            .extension()
            .types
            .contains(&ExtensionType::Frontend),
        "morphir-elm should declare the frontend capability"
    );
    let frontend = negotiated
        .capabilities()
        .frontend
        .as_ref()
        .expect("morphir-elm should advertise frontend details");
    assert!(
        frontend.compile,
        "morphir-elm should accept compile requests"
    );
    assert!(
        frontend
            .languages
            .iter()
            .any(|language| language.id == "elm"),
        "morphir-elm should advertise Elm"
    );
    assert!(
        frontend.ir_versions.iter().any(|version| version == "3"),
        "morphir-elm should advertise Morphir IR 3"
    );

    (process, session)
}

async fn close_frontend(process: SharedProcess, session: Session) {
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("MEP shutdown failed: {error}"));
    assert!(
        !process.is_running().await,
        "the frontend process should stop after shutdown"
    );
    assert!(
        process.stdout_is_exhausted().await,
        "frontend stdout should contain only the framed protocol responses"
    );
}

/// Run a backend guest through the MEP lifecycle in order.
///
/// The handshake must negotiate MEP 0.1 and the backend kind. A valid IR
/// generates artifacts, an invalid IR returns diagnostics as a result, and
/// the session then closes in order.
pub async fn backend_conformance<G: GuestConnection + 'static>(
    connection: G,
    target: &str,
    valid_ir: serde_json::Value,
    invalid_ir: serde_json::Value,
) {
    let mut session = Session::open(connection, &host_config("morphir-conformance", "0.1.0"))
        .await
        .unwrap_or_else(|error| panic!("MEP negotiation failed: {error}"));
    assert_eq!(session.negotiated().protocol_version(), "0.1");
    assert!(
        session
            .negotiated()
            .extension()
            .types
            .contains(&ExtensionType::Backend)
    );

    let generated: GenerateResult = completed(
        "generation",
        session
            .generate(GenerateRequest {
                ir: valid_ir,
                target: target.into(),
                options: Default::default(),
            })
            .await,
    );
    assert!(generated.success);
    assert!(!generated.artifacts.is_empty());

    let generated: GenerateResult = completed(
        "generation",
        session
            .generate(GenerateRequest {
                ir: invalid_ir,
                target: target.into(),
                options: Default::default(),
            })
            .await,
    );
    assert!(!generated.success);
    assert!(!generated.diagnostics.is_empty());

    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("MEP shutdown or transport termination failed: {error}"));
}
