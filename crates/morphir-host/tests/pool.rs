//! `Pool` behaviours, ported from the CLI's `SessionReuseInvoker` tests
//! (`crates/morphir/src/commands/ui/provider/sessions.rs`). The sessionless
//! (direct-provider) behaviour stays there: a `Pool` always opens a session.

use async_trait::async_trait;
use morphir_extension_sdk::protocol::{ExtensionResponse, PeerInfo, PeerKind, RpcError, methods};
use morphir_host::testing::{MemoryChannel, frontend_initialize_result};
use morphir_host::{
    BasicChecks, CallError, Channel, ChannelCause, ChannelError, ChannelState, GuestConnection,
    HostConfig, HostError, JsonRpcConnection, Outgoing, Pool,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::future::{Future, Ready};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll};
use tokio::sync::oneshot;
use tokio::time::{Duration, timeout};

/// Poll `future` once with a no-op waker and assert it is still
/// [`Poll::Pending`]. Proves the future was actually attempted, not merely
/// skipped by whatever drove it, so a later "the other one completed
/// meanwhile" assertion means what it says.
fn assert_pending<F: Future>(future: Pin<&mut F>) {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    match future.poll(&mut cx) {
        Poll::Pending => {}
        Poll::Ready(_) => panic!("expected the future to still be pending"),
    }
}

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

/// A channel whose first post-handshake `receive` (the one answering the
/// first real call) waits on a [`oneshot::Receiver`] before it hands back
/// its scripted answer. The handshake answers right away, and any further
/// calls after the gated one answer right away too, from `answers` in
/// order: a session a gated call shares with another call (same key,
/// cached) must still be able to serve that second call once the gate
/// opens.
///
/// It counts every `send` and raises a flag when it is dropped, so a test
/// can see whether the pool still holds it.
struct GatedChannel {
    handshake: Option<ExtensionResponse>,
    answers: VecDeque<ExtensionResponse>,
    gate: Option<oneshot::Receiver<()>>,
    probe: Probe,
}

/// What a test can observe about a [`GatedChannel`] after the pool owns it.
#[derive(Clone, Default)]
struct Probe {
    sends: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
}

impl Probe {
    fn sends(&self) -> usize {
        self.sends.load(Ordering::SeqCst)
    }

    fn dropped(&self) -> bool {
        self.dropped.load(Ordering::SeqCst)
    }
}

impl Drop for GatedChannel {
    fn drop(&mut self) {
        self.probe.dropped.store(true, Ordering::SeqCst);
    }
}

impl GatedChannel {
    fn new(
        handshake: ExtensionResponse,
        answers: impl IntoIterator<Item = ExtensionResponse>,
        gate: oneshot::Receiver<()>,
    ) -> Self {
        Self {
            handshake: Some(handshake),
            answers: answers.into_iter().collect(),
            gate: Some(gate),
            probe: Probe::default(),
        }
    }

    /// Report sends and the drop to `probe`.
    fn with_probe(mut self, probe: Probe) -> Self {
        self.probe = probe;
        self
    }
}

#[async_trait]
impl Channel for GatedChannel {
    async fn send(&mut self, _message: Outgoing) -> Result<(), ChannelError> {
        self.probe.sends.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn receive(&mut self) -> Result<ExtensionResponse, ChannelError> {
        if let Some(response) = self.handshake.take() {
            return Ok(response);
        }
        if let Some(gate) = self.gate.take() {
            let _ = gate.await;
        }
        self.answers.pop_front().ok_or_else(|| ChannelError {
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
//
// This deliberately does not use `tokio::select!` to race the two calls:
// `select!` polls its branches in a random order by default, so a run that
// happens to poll `call_b` first would let it complete before `call_a` ever
// takes a lock, silently skipping the very interleaving this test exists to
// prove. Polling `call_a` once and asserting it is still `Pending` makes the
// interleaving explicit and the test deterministic: if a bug held some lock
// across the whole call, `call_b` would also observe `Pending` here, and the
// surrounding `timeout` would fail the test instead of it passing by luck.
#[tokio::test]
async fn a_slow_provider_does_not_block_another() {
    let (release_tx, release_rx) = oneshot::channel();
    let pool: Pool<String> = Pool::new(config());

    let opens_a = Arc::new(AtomicUsize::new(0));
    let open_a = {
        let opens_a = Arc::clone(&opens_a);
        let gate = std::sync::Mutex::new(Some(release_rx));
        move || {
            opens_a.fetch_add(1, Ordering::SeqCst);
            let gate = gate.lock().unwrap().take().expect("opened once");
            let channel = GatedChannel::new(
                ok(1, frontend_initialize_result("slow")),
                [ok(2, json!({"provider": "a"}))],
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

        // Prove A is genuinely parked on its gate, not merely unpolled.
        assert_pending(call_a.as_mut());
        assert_eq!(opens_a.load(Ordering::SeqCst), 1);

        // B, a different key, must complete even though A is still stuck.
        let b_result = call_b.await;
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

// Requirement (item 2, cancellation safety): dropping a call's future before
// it completes (a caller's own timeout) must not leave a session cached with
// a request permanently in flight.
//
// The next call's result alone cannot prove this: a stale cached session
// would fail that call, and the pool's one retry on a fresh guest would
// still answer it. So the test watches the hung channel itself. It must be
// dropped as soon as the timeout drops the call, before the next call runs,
// and it must see no further send.
#[tokio::test]
async fn a_dropped_call_lets_the_next_call_open_a_fresh_guest() {
    let pool: Pool<String> = Pool::new(config());
    let key = "provider".to_owned();

    // Kept alive for the whole hung call: dropping the sender would make the
    // gated receive resolve with an error immediately instead of hanging.
    let (_keep_alive, never_rx) = oneshot::channel::<()>();
    let hung_gate = std::sync::Mutex::new(Some(never_rx));
    let hung_probe = Probe::default();
    let open_hung = {
        let hung_probe = hung_probe.clone();
        move || {
            let gate = hung_gate.lock().unwrap().take().expect("opened once");
            let channel = GatedChannel::new(
                ok(1, frontend_initialize_result("guest")),
                [ok(2, json!({"call": "never answered"}))],
                gate,
            )
            .with_probe(hung_probe.clone());
            let connection: Box<dyn GuestConnection> =
                Box::new(JsonRpcConnection::new(channel, BasicChecks::new("guest")));
            std::future::ready(Ok(connection))
        }
    };

    let hung = timeout(
        Duration::from_millis(50),
        pool.call::<_, Value, _, _>(&key, "fp", open_hung, "compile", &()),
    )
    .await;
    assert!(
        hung.is_err(),
        "the call must still be pending when the short timeout fires"
    );
    assert!(
        hung_probe.dropped(),
        "the timed-out call must drop its session, not leave it cached"
    );
    let sends_at_timeout = hung_probe.sends();

    let opens = Arc::new(AtomicUsize::new(0));
    let open_fresh = opener("guest", Arc::clone(&opens), |_attempt| {
        vec![
            Ok(ok(1, frontend_initialize_result("guest"))),
            Ok(ok(2, json!({"call": "fresh"}))),
        ]
    });
    let result: Value = pool
        .call(&key, "fp", open_fresh, "compile", &())
        .await
        .unwrap();

    assert_eq!(result, json!({"call": "fresh"}));
    assert_eq!(opens.load(Ordering::SeqCst), 1);
    assert_eq!(
        hung_probe.sends(),
        sends_at_timeout,
        "the next call must not reach the hung guest"
    );
}

// (Minor, item 3a) An open failure is not cached: the failed attempt leaves
// nothing behind, so the next call opens again rather than reusing a slot
// that was never actually populated. It is reported as `CallError::Open`,
// not as a call failure, and is not retried.
#[tokio::test]
async fn an_open_failure_is_not_cached() {
    let opens = Arc::new(AtomicUsize::new(0));
    let opens_clone = Arc::clone(&opens);
    let open = move || {
        let attempt = opens_clone.fetch_add(1, Ordering::SeqCst);
        let result: Result<Box<dyn GuestConnection>, HostError> = if attempt == 0 {
            Err(HostError::Invalid("scripted open failure".into()))
        } else {
            let channel = MemoryChannel::new()
                .respond(ok(1, frontend_initialize_result("guest")))
                .respond(ok(2, json!({"call": "opened"})));
            let connection: Box<dyn GuestConnection> =
                Box::new(JsonRpcConnection::new(channel, BasicChecks::new("guest")));
            Ok(connection)
        };
        std::future::ready(result)
    };
    let pool: Pool<String> = Pool::new(config());
    let key = "provider".to_owned();

    let error = pool
        .call::<_, Value, _, _>(&key, "fp", &open, "compile", &())
        .await
        .unwrap_err();
    // An open failure is its own variant, carries the open's error as it
    // was, and is not retried.
    assert!(
        matches!(&error, CallError::Open(HostError::Invalid(message)) if message == "scripted open failure"),
        "{error:?}"
    );
    assert_eq!(opens.load(Ordering::SeqCst), 1);

    let result: Value = pool.call(&key, "fp", &open, "compile", &()).await.unwrap();
    assert_eq!(result, json!({"call": "opened"}));
    assert_eq!(opens.load(Ordering::SeqCst), 2);
}

// An open failure while replacing a broken session is reported as
// `CallError::Open` with the open's own error, not as the call failure that
// led to it, and is not retried again.
#[tokio::test]
async fn a_replacement_that_fails_to_open_is_reported_as_an_open_failure() {
    let opens = Arc::new(AtomicUsize::new(0));
    let open = {
        let opens = Arc::clone(&opens);
        move || {
            let attempt = opens.fetch_add(1, Ordering::SeqCst);
            let result: Result<Box<dyn GuestConnection>, HostError> = if attempt == 0 {
                let channel = MemoryChannel::new()
                    .respond(ok(1, frontend_initialize_result("guest")))
                    .fail(transport_failure());
                Ok(Box::new(JsonRpcConnection::new(
                    channel,
                    BasicChecks::new("guest"),
                )))
            } else {
                Err(HostError::Invalid("scripted reopen failure".into()))
            };
            std::future::ready(result)
        }
    };
    let pool: Pool<String> = Pool::new(config());

    let error = pool
        .call::<_, Value, _, _>(&"provider".to_owned(), "fp", open, "compile", &())
        .await
        .unwrap_err();

    assert!(
        matches!(&error, CallError::Open(HostError::Invalid(message)) if message == "scripted reopen failure"),
        "{error:?}"
    );
    assert_eq!(opens.load(Ordering::SeqCst), 2);
}

// (Minor, item 3b) A fingerprint change does not just forget the old
// session: it closes it. The old channel must see the MEP shutdown sequence
// (a `morphir.shutdown` request answered, then `morphir.exit`) and its
// channel-level `close`.
#[tokio::test]
async fn a_changed_fingerprint_closes_the_old_session() {
    let first_channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .respond(ok(2, json!({"opened": "first"})))
        .respond(ok(3, json!({})));
    let first_log = first_channel.log();
    let first_connection: Box<dyn GuestConnection> = Box::new(JsonRpcConnection::new(
        first_channel,
        BasicChecks::new("guest"),
    ));

    let second_channel = MemoryChannel::new()
        .respond(ok(1, frontend_initialize_result("guest")))
        .respond(ok(2, json!({"opened": "second"})));
    let second_connection: Box<dyn GuestConnection> = Box::new(JsonRpcConnection::new(
        second_channel,
        BasicChecks::new("guest"),
    ));

    let connections = Arc::new(std::sync::Mutex::new(VecDeque::from([
        first_connection,
        second_connection,
    ])));
    let open = move || {
        let connection = connections
            .lock()
            .unwrap()
            .pop_front()
            .expect("only two opens expected");
        std::future::ready(Ok(connection))
    };

    let pool: Pool<String> = Pool::new(config());
    let key = "provider".to_owned();

    let first: Value = pool
        .call(&key, "fp-1", open.clone(), "compile", &())
        .await
        .unwrap();
    assert_eq!(first, json!({"opened": "first"}));

    let second: Value = pool
        .call(&key, "fp-2", open.clone(), "compile", &())
        .await
        .unwrap();
    assert_eq!(second, json!({"opened": "second"}));

    assert_eq!(first_log.closes(), 1);
    assert_eq!(
        first_log.methods().last().map(String::as_str),
        Some(methods::EXIT)
    );
}

// (Minor, item 3c) Two concurrent first calls on the same key must not race
// each other into opening two guests: the slot's own lock serializes them,
// so the second call finds the session the first one opened.
#[tokio::test]
async fn two_concurrent_first_calls_on_the_same_key_open_only_one_guest() {
    let opens = Arc::new(AtomicUsize::new(0));
    let (release_tx, release_rx) = oneshot::channel();
    let gate = Arc::new(std::sync::Mutex::new(Some(release_rx)));
    let open = {
        let opens = Arc::clone(&opens);
        move || {
            opens.fetch_add(1, Ordering::SeqCst);
            let gate = gate.lock().unwrap().take().expect("opened once");
            // Two answers: A's gated call (id 2) and, since B rides the same
            // session once A's gate opens, B's call after it (id 3).
            let channel = GatedChannel::new(
                ok(1, frontend_initialize_result("guest")),
                [
                    ok(2, json!({"call": "shared"})),
                    ok(3, json!({"call": "shared"})),
                ],
                gate,
            );
            let connection: Box<dyn GuestConnection> =
                Box::new(JsonRpcConnection::new(channel, BasicChecks::new("guest")));
            std::future::ready(Ok(connection))
        }
    };
    let pool: Pool<String> = Pool::new(config());
    let key = "provider".to_owned();

    timeout(Duration::from_secs(5), async {
        let call_a = pool.call::<_, Value, _, _>(&key, "fp", open.clone(), "compile", &());
        let call_b = pool.call::<_, Value, _, _>(&key, "fp", open.clone(), "compile", &());
        tokio::pin!(call_a);
        tokio::pin!(call_b);

        // A reaches the gate and parks there, holding the slot's lock.
        assert_pending(call_a.as_mut());
        assert_eq!(opens.load(Ordering::SeqCst), 1);

        // B, the same key, is stuck waiting on the slot's lock: it cannot
        // even attempt to open its own guest while A holds it.
        assert_pending(call_b.as_mut());
        assert_eq!(
            opens.load(Ordering::SeqCst),
            1,
            "B must not open a second guest while A holds the slot"
        );

        release_tx
            .send(())
            .expect("call_a is still awaiting the gate");
        let (a_result, b_result) = tokio::join!(call_a, call_b);
        assert_eq!(a_result.unwrap(), json!({"call": "shared"}));
        assert_eq!(b_result.unwrap(), json!({"call": "shared"}));
        assert_eq!(opens.load(Ordering::SeqCst), 1);
    })
    .await
    .expect("must not hang once released");
}

// A call that waits on a slot's lock while `abandon` removes that slot must
// not use the abandoned slot once it gets the lock: it looks the key up
// again and opens a fresh guest there. The call already in flight finishes
// against the session it holds, and that session is dropped with the
// detached slot once both calls are done.
#[tokio::test]
async fn a_call_waiting_when_the_key_is_abandoned_opens_a_fresh_guest() {
    let opens = Arc::new(AtomicUsize::new(0));
    let (release_tx, release_rx) = oneshot::channel();
    let first_probe = Probe::default();
    let open = {
        let opens = Arc::clone(&opens);
        let gate = Arc::new(std::sync::Mutex::new(Some(release_rx)));
        let first_probe = first_probe.clone();
        move || {
            let attempt = opens.fetch_add(1, Ordering::SeqCst);
            let connection: Box<dyn GuestConnection> = if attempt == 0 {
                let gate = gate.lock().unwrap().take().expect("gated once");
                // Two answers, so that a waiter wrongly riding this session
                // would get "first" instead of failing and retrying.
                let channel = GatedChannel::new(
                    ok(1, frontend_initialize_result("guest")),
                    [
                        ok(2, json!({"guest": "first"})),
                        ok(3, json!({"guest": "first"})),
                    ],
                    gate,
                )
                .with_probe(first_probe.clone());
                Box::new(JsonRpcConnection::new(channel, BasicChecks::new("guest")))
            } else {
                let channel = MemoryChannel::new()
                    .respond(ok(1, frontend_initialize_result("guest")))
                    .respond(ok(2, json!({"guest": "second"})))
                    .respond(ok(3, json!({"guest": "second, reused"})));
                Box::new(JsonRpcConnection::new(channel, BasicChecks::new("guest")))
            };
            std::future::ready(Ok(connection))
        }
    };
    let pool: Pool<String> = Pool::new(config());
    let key = "provider".to_owned();

    timeout(Duration::from_secs(5), async {
        let call_a = pool.call::<_, Value, _, _>(&key, "fp", open.clone(), "compile", &());
        let call_b = pool.call::<_, Value, _, _>(&key, "fp", open.clone(), "compile", &());
        tokio::pin!(call_a);
        tokio::pin!(call_b);

        // A is in flight on the first guest; B waits on the slot's lock.
        assert_pending(call_a.as_mut());
        assert_pending(call_b.as_mut());
        assert_eq!(opens.load(Ordering::SeqCst), 1);

        pool.abandon(&key).await;

        release_tx
            .send(())
            .expect("call_a is still awaiting the gate");
        let (a_result, b_result) = tokio::join!(call_a, call_b);
        assert_eq!(a_result.unwrap(), json!({"guest": "first"}));
        assert_eq!(b_result.unwrap(), json!({"guest": "second"}));
        assert_eq!(opens.load(Ordering::SeqCst), 2);
    })
    .await
    .expect("must not hang once released");

    assert!(
        first_probe.dropped(),
        "the abandoned guest must not stay cached"
    );

    // B's guest is the one cached for the key now.
    let reused: Value = pool
        .call(&key, "fp", open.clone(), "compile", &())
        .await
        .unwrap();
    assert_eq!(reused, json!({"guest": "second, reused"}));
    assert_eq!(opens.load(Ordering::SeqCst), 2);
}

// (Item 6) `Pool::call`'s future must stay `Send` on native targets when its
// generics are, so a caller can `tokio::spawn` it. Compile-time only: the
// future is built and checked, never polled.
#[test]
fn pool_call_future_is_send_on_native_targets() {
    fn assert_send<T: Send>(_: &T) {}

    let pool: Pool<String> = Pool::new(config());
    let key = "provider".to_owned();
    let opens = Arc::new(AtomicUsize::new(0));
    let open = opener("guest", opens, |_attempt| {
        vec![Ok(ok(1, frontend_initialize_result("guest")))]
    });
    let params = json!({});

    let future = pool.call::<_, Value, _, _>(&key, "fp", open, "compile", &params);
    assert_send(&future);
}
