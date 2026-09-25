//! A process guest and a native guest answer the same `Session::compile`.
//!
//! Both guests run the one frontend in `examples/support/parity_frontend.rs`:
//! in the host process through a `NativeChannel`, and as the
//! `mep-native-frontend` example over stdio. Build the example and give its
//! path before running this ignored test:
//!
//! `cargo build -p morphir-host-native --example mep-native-frontend`
//! `MEP_NATIVE_FRONTEND_FIXTURE=target/debug/examples/mep-native-frontend cargo test -p morphir-host-native --test guest_parity -- --ignored`

mod support;

#[path = "../examples/support/parity_frontend.rs"]
mod parity_frontend;

use morphir_extension_sdk::{
    CompileOptions, CompilePackage, CompileRequest, CompileResult, SourceDocument, SourceSet,
};
use morphir_host::{
    ExpectedChecks, GuestConnection, InvocationMode, InvocationPolicy, JsonRpcConnection, Registry,
    Session,
};
use morphir_host_native::process::{ProcessChannel, ProcessLaunch};
use morphir_host_native::{CheckedConnection, NativeSource};
use parity_frontend::{LANGUAGE, ParityFrontend};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use support::mep::{completed, host_config};

fn frontend_fixture_path() -> PathBuf {
    let path = std::env::var_os("MEP_NATIVE_FRONTEND_FIXTURE")
        .map(PathBuf::from)
        .expect("MEP_NATIVE_FRONTEND_FIXTURE should point at the built mep-native-frontend");
    if path.is_absolute() {
        path
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path)
    }
}

fn a_compile_request() -> CompileRequest {
    let document = |uri: &str, text: &str| SourceDocument {
        uri: uri.into(),
        language_id: LANGUAGE.into(),
        version: 1,
        text: text.into(),
    };
    CompileRequest {
        language_id: LANGUAGE.into(),
        sources: SourceSet {
            root: Some("file:///workspace/src".into()),
            documents: vec![
                document("file:///workspace/src/Main.parity", "main = 1"),
                document("file:///workspace/src/Orders.parity", "orders = []"),
            ],
        },
        package: CompilePackage {
            name: "example/parity".into(),
            exposed_modules: None,
        },
        dependencies: vec![],
        options: CompileOptions {
            types_only: false,
            ir_version: "4".into(),
            extra: Default::default(),
        },
        baseline: None,
    }
}

async fn compile_with<G: GuestConnection + 'static>(
    connection: G,
    request: CompileRequest,
) -> CompileResult {
    let mut session = Session::open(connection, &host_config("guest-parity", "1.0.0"))
        .await
        .unwrap_or_else(|error| panic!("MEP negotiation failed: {error}"));
    assert_eq!(session.negotiated().extension().id, "mep-native-frontend");
    let result = completed("compilation", session.compile(request).await);
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("MEP shutdown failed: {error}"));
    result
}

async fn compile_natively(request: CompileRequest) -> CompileResult {
    let mut registry = Registry::new();
    registry
        .register(Arc::new(NativeSource::new(ParityFrontend::native())))
        .unwrap();
    let resolved = registry
        .resolve_frontend(LANGUAGE, "4", InvocationPolicy::ProtocolOnly)
        .unwrap();
    assert_eq!(resolved.invocation_mode(), InvocationMode::NativeMep);
    let connection = resolved
        .connect(Path::new("."))
        .await
        .unwrap_or_else(|error| panic!("the built-in should connect: {error}"));
    compile_with(connection, request).await
}

async fn compile_in_a_process(request: CompileRequest) -> CompileResult {
    let launch = ProcessLaunch::new(
        "mep-native-frontend",
        frontend_fixture_path(),
        std::env::current_dir().expect("the test working directory should exist"),
    );
    let channel = ProcessChannel::spawn(launch)
        .await
        .expect("the host should start the extension process");
    let checks = ExpectedChecks::new(channel.expectation());
    let connection = CheckedConnection::new(JsonRpcConnection::new(channel, checks));
    compile_with(connection, request).await
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the independently built mep-native-frontend executable"]
async fn a_process_guest_and_a_native_guest_answer_the_same_compile() {
    let native = compile_natively(a_compile_request()).await;
    let process = compile_in_a_process(a_compile_request()).await;

    assert!(native.success);
    assert_eq!(native.ir_version.as_deref(), Some("4"));
    assert_eq!(native.modules, ["Main", "Orders"]);
    assert_eq!(native.diagnostics.len(), 2);
    // `CompileResult` has no `PartialEq`; its wire form is what a host sees.
    assert_eq!(
        serde_json::to_value(&process).unwrap(),
        serde_json::to_value(&native).unwrap()
    );
}
