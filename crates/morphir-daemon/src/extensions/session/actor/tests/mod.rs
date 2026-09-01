//! Fixtures shared by the session actor's tests.

mod idle;
mod invocation;
mod shutdown;

use kameo::actor::Spawn as _;

use super::lifecycle::SessionActor;
use super::messages::Shutdown;
use super::watchdog::{SessionActivity, spawn_idle_watchdog};
use super::{spawn_session, spawn_session_with_idle_timeout};

use crate::DaemonError;
use crate::extensions::protocol::{ExtensionRequest, ExtensionResponse, RpcError, methods};
use crate::extensions::session::tests::{
    FakeTransport, RequestLog, backend_initialization, params,
};
use crate::extensions::session::{
    ExpectedExtension, MepTransport, Ready, Session, TransportError, TransportState,
};
use std::collections::VecDeque;

/// A response envelope the fake transport hands back, or a transport failure.
type Exchange = std::result::Result<ExtensionResponse, TransportError>;

/// A well-formed `GenerateResult` distinguished by its single artifact path.
fn generate_result(path: &str) -> serde_json::Value {
    serde_json::json!({
        "success": true,
        "artifacts": [{"path": path, "content": "{}"}],
        "diagnostics": []
    })
}

fn generated_paths(value: &serde_json::Value) -> Vec<String> {
    value["artifacts"]
        .as_array()
        .expect("a generate result carries artifacts")
        .iter()
        .map(|artifact| artifact["path"].as_str().expect("a path").to_owned())
        .collect()
}

/// Negotiate a backend session whose transport replays `exchanges` afterwards.
///
/// Negotiation consumes response id 1, so the first replayed exchange must
/// answer id 2. Response ids are validated against the session's own request
/// counter, which makes the id sequence a proof that one session was reused.
///
/// The returned [`RequestLog`] is shared with the transport, so it stays
/// readable after the session is handed to an actor that never gives it back.
async fn ready_backend_session(
    exchanges: impl IntoIterator<Item = Exchange>,
) -> (Session<FakeTransport, Ready>, RequestLog) {
    let (transport, requests) = backend_transport(exchanges);
    (negotiated(transport).await, requests)
}

/// A transport that answers a backend negotiation and then `exchanges`.
fn backend_transport(exchanges: impl IntoIterator<Item = Exchange>) -> (FakeTransport, RequestLog) {
    let mut responses: VecDeque<Exchange> = VecDeque::new();
    responses.push_back(Ok(ExtensionResponse::success(
        1,
        backend_initialization(true),
    )
    .expect("a valid envelope")));
    responses.extend(exchanges);
    let requests = RequestLog::default();
    let transport = FakeTransport {
        expected: ExpectedExtension::identified("example"),
        responses,
        termination: TransportState::Stopped,
        requests: requests.clone(),
    };
    (transport, requests)
}

/// Drive `transport` through negotiation into a ready session.
async fn negotiated<T: MepTransport + Send + 'static>(transport: T) -> Session<T, Ready> {
    match Session::loaded(transport).initialize(params()).await {
        Ok(session) => session,
        Err(failure) => panic!("negotiation should succeed: {}", failure.error()),
    }
}

/// The successful exchanges that answer `results`, in request-id order.
fn answering(results: &[serde_json::Value]) -> Vec<Exchange> {
    results
        .iter()
        .enumerate()
        .map(|(index, result)| {
            Ok(ExtensionResponse::success(index as u64 + 2, result).expect("a valid envelope"))
        })
        .collect()
}

async fn ready_session_answering(
    results: &[serde_json::Value],
) -> (Session<FakeTransport, Ready>, RequestLog) {
    ready_backend_session(answering(results)).await
}

/// A transport that takes `delay` to answer every exchange.
///
/// Every other fake here answers instantly, so no invocation can ever
/// outlive an idle window in a test. Real MEP work does: a compile of a
/// large package can easily run longer than the idle period a caller
/// picked for a session it expects to be interactive.
struct SlowTransport {
    inner: FakeTransport,
    delay: std::time::Duration,
}

#[async_trait::async_trait]
impl MepTransport for SlowTransport {
    fn expected_extension(&self) -> ExpectedExtension {
        self.inner.expected_extension()
    }

    async fn exchange(
        &mut self,
        request: ExtensionRequest,
    ) -> std::result::Result<ExtensionResponse, TransportError> {
        tokio::time::sleep(self.delay).await;
        self.inner.exchange(request).await
    }

    async fn terminate(&mut self) -> std::result::Result<TransportState, TransportError> {
        self.inner.terminate().await
    }
}

/// A ready session whose every exchange takes `delay` to answer.
async fn ready_slow_session_answering(
    results: &[serde_json::Value],
    delay: std::time::Duration,
) -> (Session<SlowTransport, Ready>, RequestLog) {
    let (inner, requests) = backend_transport(answering(results));
    // Negotiation is slow too, but it completes before any actor -- and so
    // before any idle watchdog -- exists, so it cannot affect the timing
    // under test.
    (negotiated(SlowTransport { inner, delay }).await, requests)
}

async fn ready_session_rejecting_then_answering(
    result: serde_json::Value,
) -> (Session<FakeTransport, Ready>, RequestLog) {
    ready_backend_session([
        Ok(ExtensionResponse::error(
            2,
            RpcError::extension_error("the extension refused this request"),
        )),
        Ok(ExtensionResponse::success(3, result).expect("a valid envelope")),
    ])
    .await
}

/// A session whose next exchange fails at the transport with an `Io` cause.
///
/// The cause is deliberately not an `Extension` error so that a test can
/// prove the original variant survives being reported to the caller.
async fn ready_session_failing_transport() -> (Session<FakeTransport, Ready>, RequestLog) {
    ready_backend_session([Err(TransportError::new(
        DaemonError::Io(std::io::Error::other("the transport pipe broke")),
        TransportState::Stopped,
    ))])
    .await
}
