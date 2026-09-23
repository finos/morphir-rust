//! Native protocol fixture with a completed initialization handshake.

use morphir_extension_sdk::{
    NativeExtension,
    protocol::{ExtensionRequest, InitializeParams, MEP_VERSION, PeerInfo, methods},
};
use morphir_rust_binding::RustExtension;

pub fn an_initialized_extension() -> NativeExtension {
    let extension = NativeExtension::frontend_backend(RustExtension).unwrap();
    let response = extension.protocol().handle(
        ExtensionRequest::new(
            methods::INITIALIZE,
            InitializeParams {
                protocol_versions: vec![MEP_VERSION.into()],
                host: PeerInfo {
                    kind: Default::default(),
                    name: "binding-test".into(),
                    version: "1.0.0".into(),
                },
            },
            0,
        )
        .unwrap(),
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    extension
}
