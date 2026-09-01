//! The idle watchdog: what counts as idle, and what does not.

use super::*;

#[tokio::test(start_paused = true)]
async fn an_idle_session_stops_itself() {
    let (session, _requests) = ready_session_answering(&[generate_result("out.avro")]).await;
    let handle = spawn_session_with_idle_timeout(session, std::time::Duration::from_secs(60));

    tokio::time::advance(std::time::Duration::from_secs(61)).await;
    tokio::task::yield_now().await;

    let after = handle
        .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
        .await;
    assert!(after.is_err(), "an idle session should have stopped");
    assert!(
        matches!(after, Err(DaemonError::SessionLost(_))),
        "an idle stop should surface the same way as any other dead session: {after:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn an_idle_stop_completes_the_mep_shutdown_handshake() {
    let (session, requests) =
        ready_backend_session([Ok(
            ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
        )])
        .await;
    // Spawned directly (bypassing `spawn_session_with_idle_timeout`) so
    // this test can hold the concrete `ActorRef` and wait deterministically
    // for `on_stop` via `wait_for_shutdown_result`, rather than inferring
    // completion from a subsequent `invoke` failing (which only proves the
    // mailbox stopped accepting new messages, not that `on_stop` itself
    // has finished running).
    let (activity, receiver) = tokio::sync::watch::channel(SessionActivity::Idle);
    let actor_ref = SessionActor::spawn(SessionActor {
        session: Some(session),
        activity,
    });
    let idle = std::time::Duration::from_secs(60);
    let _watchdog = spawn_idle_watchdog(actor_ref.downgrade(), receiver, idle);

    tokio::time::advance(idle + std::time::Duration::from_secs(1)).await;
    actor_ref
        .wait_for_shutdown_result()
        .await
        .expect("on_stop should not error");

    // Dropping the session instead of shutting it down would leave the
    // extension running and this request unsent. Proving this here (and
    // not just for the explicit `Shutdown` message) is the point of
    // giving an idle-stopped actor its own `on_stop` hook.
    assert_eq!(requests.methods(), [methods::INITIALIZE, methods::SHUTDOWN]);
}

#[tokio::test(start_paused = true)]
async fn activity_resets_the_idle_timer() {
    let (session, _requests) = ready_session_answering(&[
        generate_result("first.avro"),
        generate_result("second.avro"),
    ])
    .await;
    let idle = std::time::Duration::from_secs(10);
    let handle = spawn_session_with_idle_timeout(session, idle);

    // Advance less than the idle duration, then invoke: this should reset
    // the timer rather than letting the elapsed time accumulate toward it.
    tokio::time::advance(std::time::Duration::from_secs(7)).await;
    tokio::task::yield_now().await;
    let first: serde_json::Value = handle
        .invoke("morphir.backend.generate", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(generated_paths(&first), ["first.avro"]);

    // Advance less than the idle duration again. Without a reset, the two
    // advances together (7s + 7s = 14s) would exceed the 10s idle window
    // and this session would already be dead.
    tokio::time::advance(std::time::Duration::from_secs(7)).await;
    tokio::task::yield_now().await;
    let second: serde_json::Value = handle
        .invoke("morphir.backend.generate", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(generated_paths(&second), ["second.avro"]);
}

#[tokio::test(start_paused = true)]
async fn an_invocation_outliving_the_idle_window_keeps_the_session() {
    let idle = std::time::Duration::from_secs(10);
    // One invocation on its own takes three idle windows to answer, so the
    // watchdog's deadline passes while the session is at its busiest.
    let (session, _requests) = ready_slow_session_answering(
        &[generate_result("slow.avro"), generate_result("next.avro")],
        idle * 3,
    )
    .await;
    let handle = spawn_session_with_idle_timeout(session, idle);

    let slow: serde_json::Value = handle
        .invoke(methods::GENERATE, serde_json::json!({}))
        .await
        .expect("an invocation in flight is not idle");
    assert_eq!(generated_paths(&slow), ["slow.avro"]);

    // The point of the test: a session that was working the whole time is
    // still usable afterwards. An idle deadline that only restarts when an
    // invocation *begins* has already expired by now.
    let next = handle
        .invoke::<serde_json::Value>(methods::GENERATE, serde_json::json!({}))
        .await;
    assert!(
        next.is_ok(),
        "a session busy for the whole idle window was stopped anyway: {next:?}"
    );
    assert_eq!(generated_paths(&next.unwrap()), ["next.avro"]);
}

#[tokio::test(start_paused = true)]
async fn a_session_that_outlived_its_deadline_still_stops_once_it_goes_idle() {
    let idle = std::time::Duration::from_secs(10);
    let (session, _requests) = ready_slow_session_answering(
        &[generate_result("slow.avro"), generate_result("unused.avro")],
        idle * 3,
    )
    .await;
    let handle = spawn_session_with_idle_timeout(session, idle);

    // Runs straight through the first deadline, so the watchdog is left
    // holding a deadline it declined to act on.
    let _: serde_json::Value = handle
        .invoke(methods::GENERATE, serde_json::json!({}))
        .await
        .unwrap();
    tokio::time::advance(idle + std::time::Duration::from_secs(1)).await;
    tokio::task::yield_now().await;

    let after = handle
        .invoke::<serde_json::Value>(methods::GENERATE, serde_json::json!({}))
        .await;
    assert!(
        matches!(after, Err(DaemonError::SessionLost(_))),
        "declining to stop a busy session must not disarm the idle stop: {after:?}"
    );
}

#[tokio::test]
async fn the_idle_watchdog_exits_when_the_actor_stops_for_another_reason() {
    let (session, _requests) =
        ready_backend_session([Ok(
            ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
        )])
        .await;
    let (activity, receiver) = tokio::sync::watch::channel(SessionActivity::Idle);
    let actor_ref = SessionActor::spawn(SessionActor {
        session: Some(session),
        activity,
    });
    // An idle duration far longer than this test will take: if the
    // watchdog only exits by reaching its deadline (rather than noticing
    // the actor's `activity` sender dropped), the `timeout` below fails
    // fast instead of the test hanging for real minutes.
    let idle = std::time::Duration::from_secs(600);
    let watchdog = spawn_idle_watchdog(actor_ref.downgrade(), receiver, idle);

    actor_ref.ask(Shutdown).await.unwrap();
    actor_ref
        .wait_for_shutdown_result()
        .await
        .expect("on_stop should not error");

    tokio::time::timeout(std::time::Duration::from_secs(1), watchdog)
        .await
        .expect(
            "the watchdog should exit as soon as the actor stops, \
             not linger until its idle deadline",
        )
        .expect("the watchdog task should not panic");
}
