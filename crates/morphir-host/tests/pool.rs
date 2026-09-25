//! `Pool` behaviours, ported from the CLI's `SessionReuseInvoker` tests
//! (`crates/morphir/src/commands/ui/provider/sessions.rs`). The sessionless
//! (direct-provider) behaviour stays there: a `Pool` always opens a session.

use async_trait::async_trait;
use morphir_extension_sdk::protocol::{ExtensionResponse, PeerInfo, PeerKind, RpcError};
use morphir_host::testing::{MemoryChannel, frontend_initialize_result};
use morphir_host::{
    BasicChecks, CallError, Channel, ChannelCause, ChannelError, ChannelState, GuestConnection,
    HostConfig, HostError, JsonRpcConnection, Outgoing, Pool,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::future::Ready;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::oneshot;
use tokio::time::{Duration, timeout};

fn config() -> HostConfig {
    HostConfig::new(PeerInfo {
        kind: PeerKind::Unspecified,
        name: "test".into(),
        version: "1.0.0".into(),
    })
}

fn ok(id: u64, value: impl Serialize) -> ExtensionResponse {
    ExtensionResponse::success(id, value).unwrap()
}

fn rejected(id: u64) -> ExtensionResponse {
    ExtensionResponse::error(
        id,
        RpcError {
            code: -32000,
            message: "scripted rejection".into(),
            data: None,
        },
    )
}

fn transport_failure() -> ChannelError {
    ChannelError {
        message: "scripted transport failure".into(),
        state: ChannelState::Stopped,
        cause: ChannelCause::Transport,
    }
}

/// Build an `open` closure that counts every attempt and answers the
/// `attempt`-th (0-indexed) one with `scripts(attempt)` over a fresh
/// [`MemoryChannel`].
fn opener<S>(
    id: &'static str,
    opens: Arc<AtomicUsize>,
    scripts: S,
) -> impl Fn() -> Ready<Result<Box<dyn GuestConnection>, HostError>> + Clone
where
    S: Fn(usize) -> Vec<Result<ExtensionResponse, ChannelError>> + Clone,
{
    move || {
        let attempt = opens.fetch_add(1, Ordering::SeqCst);
        let mut channel = MemoryChannel::new();
        for answer in scripts(attempt) {
            channel = match answer {
                Ok(response) => channel.respond(response),
                Err(error) => channel.fail(error),
            };
        }
        let connection: Box<dyn GuestConnection> =
            Box::new(JsonRpcConnection::new(channel, BasicChecks::new(id)));
        std::future::ready(Ok(connection))
    }
}

// Requirement: the pool's whole purpose. Two calls for the same key must ride
// one session, proven by one open and by the session's own state (an
// incrementing marker) carrying across calls.
#[tokio::test]
async fn two_calls_share_one_open() {
    let opens = Arc::new(AtomicUsize::new(0));
    let open = opener("guest", Arc::clone(&opens), |_attempt| {
        vec![
            Ok(ok(1, frontend_initialize_result("guest"))),
            Ok(ok(2, json!({"call": 1}))),
            Ok(ok(3, json!({"call": 2}))),
        ]
    });
    let pool: Pool<String> = Pool::new(config());

    let first: Value = pool
        .call(&"provider".to_owned(), "fp", open.clone(), "compile", &())
        .await
        .unwrap();
    let second: Value = pool
        .call(&"provider".to_owned(), "fp", open.clone(), "compile", &())
        .await
        .unwrap();

    assert_eq!(first, json!({"call": 1}));
    assert_eq!(second, json!({"call": 2}));
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

// A session belongs to a key, not to a method: a call that follows a
// different-named call for the same key still rides the session that call
// opened.
#[tokio::test]
async fn a_call_reuses_the_session_a_different_method_opened() {
    let opens = Arc::new(AtomicUsize::new(0));
    let open = opener("guest", Arc::clone(&opens), |_attempt| {
        vec![
            Ok(ok(1, frontend_initialize_result("guest"))),
            Ok(ok(2, json!({"call": 1}))),
            Ok(ok(3, json!({"call": 2}))),
        ]
    });
    let pool: Pool<String> = Pool::new(config());

    let compiled: Value = pool
        .call(&"provider".to_owned(), "fp", open.clone(), "compile", &())
        .await
        .unwrap();
    let generated: Value = pool
        .call(&"provider".to_owned(), "fp", open.clone(), "generate", &())
        .await
        .unwrap();

    assert_eq!(compiled, json!({"call": 1}));
    assert_eq!(generated, json!({"call": 2}));
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

// Requirement: a failure is an eviction signal, not an answer. A session that
// breaks mid-call costs the caller nothing: the call runs again on a fresh
// guest.
#[tokio::test]
async fn a_failed_call_is_retried_once_on_a_fresh_guest() {
    let opens = Arc::new(AtomicUsize::new(0));
    let open = opener("guest", Arc::clone(&opens), |attempt| match attempt {
        0 => vec![
            Ok(ok(1, frontend_initialize_result("guest"))),
            Err(transport_failure()),
        ],
        _ => vec![
            Ok(ok(1, frontend_initialize_result("guest"))),
            Ok(ok(2, json!({"call": "retried"}))),
        ],
    });
    let pool: Pool<String> = Pool::new(config());

    let result: Value = pool
        .call(&"provider".to_owned(), "fp", open.clone(), "compile", &())
        .await
        .unwrap();

    assert_eq!(result, json!({"call": "retried"}));
    assert_eq!(opens.load(Ordering::SeqCst), 2);
}

// One retry, not a loop: a guest whose replacement also fails surfaces as an
// error, and the slot is left empty so the next call opens fresh rather than
// reusing a broken reference.
#[tokio::test]
async fn a_retry_that_also_fails_surfaces_and_clears_the_slot() {
    let opens = Arc::new(AtomicUsize::new(0));
    let open = opener("guest", Arc::clone(&opens), |attempt| match attempt {
        0 | 1 => vec![
            Ok(ok(1, frontend_initialize_result("guest"))),
            Err(transport_failure()),
        ],
        _ => vec![
            Ok(ok(1, frontend_initialize_result("guest"))),
            Ok(ok(2, json!({"call": "fresh"}))),
        ],
    });
    let pool: Pool<String> = Pool::new(config());

    let error = pool
        .call::<_, Value, _, _>(&"provider".to_owned(), "fp", open.clone(), "compile", &())
        .await
        .unwrap_err();
    assert!(matches!(error, CallError::Failed(_)));
    assert_eq!(opens.load(Ordering::SeqCst), 2);

    // The cache is empty again: the next call opens fresh instead of
    // reusing the dead retry.
    let result: Value = pool
        .call(&"provider".to_owned(), "fp", open.clone(), "compile", &())
        .await
        .unwrap();
    assert_eq!(result, json!({"call": "fresh"}));
    assert_eq!(opens.load(Ordering::SeqCst), 3);
}

// A guest rejecting one call is the guest answering, not the session dying.
// The session stays cached and the next call reuses it.
#[tokio::test]
async fn a_rejection_surfaces_but_keeps_the_session() {
    let opens = Arc::new(AtomicUsize::new(0));
    let open = opener("guest", Arc::clone(&opens), |_attempt| {
        vec![
            Ok(ok(1, frontend_initialize_result("guest"))),
            Ok(rejected(2)),
            Ok(rejected(3)),
        ]
    });
    let pool: Pool<String> = Pool::new(config());

    let first = pool
        .call::<_, Value, _, _>(&"provider".to_owned(), "fp", open.clone(), "compile", &())
        .await
        .unwrap_err();
    assert!(matches!(first, CallError::Rejected(_)));

    let second = pool
        .call::<_, Value, _, _>(&"provider".to_owned(), "fp", open.clone(), "compile", &())
        .await
        .unwrap_err();
    assert!(matches!(second, CallError::Rejected(_)));
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

// Requirement: abandoning a provider forgets its session, so a later call
// opens fresh instead of reusing a possibly-wedged guest.
#[tokio::test]
async fn abandoning_forgets_the_guest() {
    let opens = Arc::new(AtomicUsize::new(0));
    let open = opener("guest", Arc::clone(&opens), |_attempt| {
        vec![
            Ok(ok(1, frontend_initialize_result("guest"))),
            Ok(ok(2, json!({"call": "one"}))),
        ]
    });
    let pool: Pool<String> = Pool::new(config());
    let key = "provider".to_owned();

    let _: Value = pool
        .call(&key, "fp", open.clone(), "compile", &())
        .await
        .unwrap();
    pool.abandon(&key).await;
    let result: Value = pool
        .call(&key, "fp", open.clone(), "compile", &())
        .await
        .unwrap();

    assert_eq!(result, json!({"call": "one"}));
    assert_eq!(opens.load(Ordering::SeqCst), 2);
}

// Requirement: a session is reused only while the resolution still names the
// incarnation that opened it. A changed fingerprint must not keep serving
// the old guest under the same key.
#[tokio::test]
async fn a_changed_fingerprint_replaces_the_cached_session() {
    let opens = Arc::new(AtomicUsize::new(0));
    let open = opener("guest", Arc::clone(&opens), |attempt| {
        vec![
            Ok(ok(1, frontend_initialize_result("guest"))),
            Ok(ok(2, json!({"opened": attempt}))),
        ]
    });
    let pool: Pool<String> = Pool::new(config());
    let key = "provider".to_owned();

    let first: Value = pool
        .call(&key, "fp-1", open.clone(), "compile", &())
        .await
        .unwrap();
    let second: Value = pool
        .call(&key, "fp-2", open.clone(), "compile", &())
        .await
        .unwrap();

    assert_eq!(first, json!({"opened": 0}));
    assert_eq!(second, json!({"opened": 1}));
    assert_eq!(opens.load(Ordering::SeqCst), 2);
}

/// A channel whose second `receive` (the one answering a real call) waits on
/// a [`oneshot::Receiver`] before it hands back the scripted answer. The
/// first `receive` (the handshake) answers right away.
struct GatedChannel {
    handshake: Option<ExtensionResponse>,
    answer: Option<ExtensionResponse>,
    gate: Option<oneshot::Receiver<()>>,
}

impl GatedChannel {
    fn new(
        handshake: ExtensionResponse,
        answer: ExtensionResponse,
        gate: oneshot::Receiver<()>,
    ) -> Self {
        Self {
            handshake: Some(handshake),
            answer: Some(answer),
            gate: Some(gate),
        }
    }
}

#[async_trait]
impl Channel for GatedChannel {
    async fn send(&mut self, _message: Outgoing) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn receive(&mut self) -> Result<ExtensionResponse, ChannelError> {
        if let Some(response) = self.handshake.take() {
            return Ok(response);
        }
        if let Some(gate) = self.gate.take() {
            let _ = gate.await;
        }
        self.answer.take().ok_or_else(|| ChannelError {
            message: "GatedChannel has no scripted answer".into(),
            state: ChannelState::Stopped,
            cause: ChannelCause::Transport,
        })
    }

    async fn close(&mut self) -> Result<ChannelState, ChannelError> {
        Ok(ChannelState::Stopped)
    }

    async fn abort(&mut self) -> Result<ChannelState, ChannelError> {
        Ok(ChannelState::Stopped)
    }
}

// Requirement: the map lock guards the map only. A call on one key that is
// stuck waiting on its guest must not delay a call on another key.
#[tokio::test]
async fn a_slow_provider_does_not_block_another() {
    let (release_tx, release_rx) = oneshot::channel();
    let pool: Pool<String> = Pool::new(config());

    let open_a = {
        let gate = std::sync::Mutex::new(Some(release_rx));
        move || {
            let gate = gate.lock().unwrap().take().expect("opened once");
            let channel = GatedChannel::new(
                ok(1, frontend_initialize_result("slow")),
                ok(2, json!({"provider": "a"})),
                gate,
            );
            let connection: Box<dyn GuestConnection> =
                Box::new(JsonRpcConnection::new(channel, BasicChecks::new("slow")));
            std::future::ready(Ok(connection))
        }
    };
    let opens_b = Arc::new(AtomicUsize::new(0));
    let open_b = opener("fast", Arc::clone(&opens_b), |_attempt| {
        vec![
            Ok(ok(1, frontend_initialize_result("fast"))),
            Ok(ok(2, json!({"provider": "b"}))),
        ]
    });

    timeout(Duration::from_secs(5), async {
        let key_a = "a".to_owned();
        let key_b = "b".to_owned();
        let call_a = pool.call::<_, Value, _, _>(&key_a, "fp", open_a, "compile", &());
        let call_b = pool.call::<_, Value, _, _>(&key_b, "fp", open_b, "compile", &());
        tokio::pin!(call_a);
        tokio::pin!(call_b);

        // Poll both concurrently. Key A's call is stuck on the gate, so only
        // B's branch can become ready here.
        let b_result = tokio::select! {
            a = &mut call_a => panic!("the gated call must not finish before the other one: {a:?}"),
            b = &mut call_b => b,
        };
        assert_eq!(b_result.unwrap(), json!({"provider": "b"}));
        assert_eq!(opens_b.load(Ordering::SeqCst), 1);

        release_tx
            .send(())
            .expect("call_a is still awaiting the gate");
        let a_result = call_a.await;
        assert_eq!(a_result.unwrap(), json!({"provider": "a"}));
    })
    .await
    .expect("the slow call must not hang once released");
}
