//! Ending a session, however it ends.

use super::*;

#[tokio::test]
async fn shutdown_completes_the_mep_handshake() {
    let (session, requests) =
        ready_backend_session([Ok(
            ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
        )])
        .await;
    let handle = spawn_session(session);

    handle.shutdown().await.unwrap();

    // Dropping the session instead of shutting it down would leave the
    // extension running and this request unsent.
    assert_eq!(requests.methods(), [methods::INITIALIZE, methods::SHUTDOWN]);
}

#[tokio::test]
async fn shutdown_reports_a_failed_handshake_as_a_lost_session() {
    let (session, _requests) = ready_backend_session([Ok(ExtensionResponse::error(
        2,
        RpcError::extension_error("the extension could not stop cleanly"),
    ))])
    .await;
    let handle = spawn_session(session);

    let result = handle.shutdown().await;

    assert!(
        matches!(result, Err(DaemonError::SessionLost(_))),
        "unexpected result: {result:?}"
    );
}

#[tokio::test]
async fn explicit_shutdown_is_not_repeated_when_the_actor_stops() {
    let (session, requests) =
        ready_backend_session([Ok(
            ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
        )])
        .await;
    // Spawned directly (bypassing `spawn_session`) so the test can hold
    // the concrete `ActorRef` and use `wait_for_shutdown_result`, which
    // resolves only once `on_stop` has actually returned. `SessionHandle`
    // erases the actor type on purpose, so it has no equivalent method;
    // polling with `yield_now` in a loop would only approximate this.
    let (activity, _receiver) = tokio::sync::watch::channel(SessionActivity::Idle);
    let actor_ref = SessionActor::spawn(SessionActor {
        session: Some(session),
        activity,
    });

    actor_ref.ask(Shutdown).await.unwrap();
    // The explicit `Shutdown` message already completed the MEP handshake
    // and took `self.session`. `on_stop` runs right after, as part of the
    // same terminal stop; waiting for it here (rather than guessing with
    // a fixed number of yields) is what makes the following assertion
    // deterministic instead of racy.
    actor_ref
        .wait_for_shutdown_result()
        .await
        .expect("on_stop should not error");

    assert_eq!(
        requests.methods(),
        [methods::INITIALIZE, methods::SHUTDOWN],
        "on_stop should not repeat the MEP shutdown handshake"
    );
}

#[tokio::test]
async fn dropping_the_last_handle_completes_the_mep_shutdown_handshake() {
    let (session, requests) =
        ready_backend_session([Ok(
            ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
        )])
        .await;
    // Deliberately spawned through the production entry point: the point of
    // this test is that nothing spawned alongside the actor (the idle
    // watchdog in particular) keeps it alive once callers let go. An idle
    // duration far longer than this test can run makes the watchdog's own
    // deadline an impossible explanation for the stop.
    let handle = spawn_session_with_idle_timeout(session, std::time::Duration::from_secs(600));

    drop(handle);

    // `SessionHandle` erases the actor type, so there is no `ActorRef` left
    // to wait on once the last handle is gone; the shared request log is
    // the only observable. Polled with a sleep rather than `yield_now` so a
    // failing run waits idly instead of burning a worker thread next to the
    // rest of the suite, and bounded so that failure is a fast, clearly
    // attributed timeout rather than a hung suite.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while requests.methods().len() < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .expect(
        "dropping the last handle should stop the actor and complete the \
         MEP shutdown handshake",
    );
    assert_eq!(requests.methods(), [methods::INITIALIZE, methods::SHUTDOWN]);
}

#[tokio::test]
async fn shutdown_stops_the_actor_and_later_calls_report_it() {
    let (session, _requests) =
        ready_backend_session([Ok(
            ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
        )])
        .await;
    let handle = spawn_session(session);

    handle.shutdown().await.unwrap();

    let after = handle
        .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
        .await;
    assert!(
        matches!(after, Err(DaemonError::SessionLost(_))),
        "unexpected result: {after:?}"
    );
    assert!(
        !format!("{after:?}").contains("already released"),
        "the actor kept accepting messages: {after:?}"
    );
}
