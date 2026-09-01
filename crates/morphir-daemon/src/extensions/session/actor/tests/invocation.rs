//! Invoking a reused session, and what each outcome means.

use super::*;

#[tokio::test]
async fn sequential_invocations_reuse_one_session() {
    let (session, requests) = ready_session_answering(&[
        generate_result("first.avro"),
        generate_result("second.avro"),
    ])
    .await;
    let handle = spawn_session(session);

    let first: serde_json::Value = handle
        .invoke("morphir.backend.generate", serde_json::json!({}))
        .await
        .unwrap();
    let second: serde_json::Value = handle
        .invoke("morphir.backend.generate", serde_json::json!({}))
        .await
        .unwrap();

    assert_eq!(generated_paths(&first), ["first.avro"]);
    assert_eq!(generated_paths(&second), ["second.avro"]);
    assert_eq!(
        requests.methods(),
        [methods::INITIALIZE, methods::GENERATE, methods::GENERATE]
    );
}

#[tokio::test]
async fn an_invocation_forwards_the_callers_method_and_params() {
    let (session, requests) = ready_session_answering(&[generate_result("out.avro")]).await;
    let handle = spawn_session(session);

    let _: serde_json::Value = handle
        .invoke(
            methods::GENERATE,
            serde_json::json!({"target": "avro", "options": {"pretty": true}}),
        )
        .await
        .unwrap();

    assert_eq!(requests.methods(), [methods::INITIALIZE, methods::GENERATE]);
    assert_eq!(
        requests.params(1),
        serde_json::json!({"target": "avro", "options": {"pretty": true}})
    );
}

#[tokio::test]
async fn a_rejected_invocation_keeps_the_session_usable() {
    let (session, _requests) =
        ready_session_rejecting_then_answering(generate_result("recovered.avro")).await;
    let handle = spawn_session(session);

    let rejected = handle
        .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
        .await;
    assert!(
        matches!(rejected, Err(DaemonError::Extension(ref message)) if message.contains("the extension refused this request")),
        "unexpected result: {rejected:?}"
    );

    let recovered: serde_json::Value = handle
        .invoke("morphir.backend.generate", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(generated_paths(&recovered), ["recovered.avro"]);
}

#[tokio::test]
async fn a_caller_can_tell_a_rejection_from_a_dead_session() {
    let (rejecting, _) =
        ready_session_rejecting_then_answering(generate_result("unused.avro")).await;
    let rejecting = spawn_session(rejecting);
    let (failing, _) = ready_session_failing_transport().await;
    let failing = spawn_session(failing);

    let rejected = rejecting
        .invoke::<serde_json::Value>(methods::GENERATE, serde_json::json!({}))
        .await;
    let lost = failing
        .invoke::<serde_json::Value>(methods::GENERATE, serde_json::json!({}))
        .await;

    // A caller caching a handle evicts on one of these and retries on the
    // other, so the two must be different variants, not different strings.
    assert!(
        matches!(rejected, Err(DaemonError::Extension(_))),
        "a refused operation should not look like a lost session: {rejected:?}"
    );
    assert!(
        matches!(lost, Err(DaemonError::SessionLost(_))),
        "a dead session should be its own variant: {lost:?}"
    );
    // The cause keeps its original variant instead of being stringified.
    assert!(
        matches!(lost, Err(DaemonError::SessionLost(ref cause)) if matches!(**cause, DaemonError::Io(_))),
        "the transport failure lost its variant: {lost:?}"
    );
}

#[tokio::test]
async fn a_failed_invocation_stops_the_actor() {
    let (session, _requests) = ready_session_failing_transport().await;
    let handle = spawn_session(session);

    let failed = handle
        .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
        .await;
    assert!(
        matches!(failed, Err(DaemonError::SessionLost(_))),
        "unexpected result: {failed:?}"
    );

    let after = handle
        .invoke::<serde_json::Value>("morphir.backend.generate", serde_json::json!({}))
        .await;
    assert!(
        matches!(after, Err(DaemonError::SessionLost(_))),
        "unexpected result: {after:?}"
    );
    // An actor that merely dropped its session would still accept the
    // message and answer with the released-session cause. Undeliverable
    // means the actor itself is gone.
    assert!(
        !format!("{after:?}").contains("already released"),
        "the actor kept accepting messages: {after:?}"
    );
}

#[tokio::test]
async fn an_undeliverable_request_is_reported_without_kameo_vocabulary() {
    let (session, _requests) =
        ready_backend_session([Ok(
            ExtensionResponse::success(2, serde_json::json!({})).expect("a valid envelope")
        )])
        .await;
    let handle = spawn_session(session);
    handle.shutdown().await.unwrap();

    let error = handle
        .invoke::<serde_json::Value>(methods::GENERATE, serde_json::json!({}))
        .await
        .expect_err("a stopped session cannot serve an invocation");

    // This module's whole reason for erasing the framework is that callers
    // -- and the people reading their output -- never learn an actor
    // library is involved. Reporting kameo's own Display strings would
    // hand a user "Extension error: actor stopped".
    let message = error.to_string();
    assert!(
        message.contains("the session ended before this request was handled"),
        "expected a domain phrase, got: {message}"
    );
    for leak in ["actor", "kameo", "mailbox"] {
        assert!(
            !message.to_lowercase().contains(leak),
            "the actor framework leaked into user-facing text via {leak:?}: {message}"
        );
    }
}
