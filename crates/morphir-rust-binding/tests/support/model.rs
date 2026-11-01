//! Object mothers for MEP `0.2.0-draft.1` model operation tests.

#![allow(dead_code)]

use morphir_extension_sdk::{
    NativeExtension,
    model::{
        CompileModelRequest, GenerateModelRequest, MODEL_ENVELOPE_VERSION, MODEL_MEP_VERSION,
        ModelCodec, ModelOperation, ModelTransportLimits, ModelTransportSupport,
        ModelTransportTuple, MorphirModel,
    },
    prelude::*,
    protocol::{ExtensionRequest, ExtensionResponse, InitializeParams, PeerInfo, methods},
};
use morphir_rust_binding::RustExtension;
use serde_json::Value;
use std::path::PathBuf;

/// The source compiled by the golden tests.
pub const SOURCE: &str = include_str!("../fixtures/model/models.rs");

/// The directory of the recorded golden outputs.
pub fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/model")
}

/// Compare `actual` with a recorded golden file. Set `MORPHIR_BLESS=1` to record it again.
pub fn assert_golden(name: &str, actual: &str) {
    let path = fixtures().join(name);
    if std::env::var_os("MORPHIR_BLESS").is_some() {
        std::fs::write(&path, actual).unwrap();
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read golden {}: {error}", path.display()));
    assert_eq!(expected, actual, "golden {name} differs");
}

/// The tuple of `operation` that the extension advertises.
pub fn a_json_tuple(operation: ModelOperation) -> ModelTransportTuple {
    ModelTransportTuple {
        operation,
        profile: "morphir-ir".into(),
        codec: ModelCodec::MorphirJson,
        envelope_version: MODEL_ENVELOPE_VERSION.into(),
        ir_version: "4.0.0".into(),
    }
}

/// The Ion tuple of `operation`, which the extension does not advertise.
pub fn an_ion_tuple(operation: ModelOperation) -> ModelTransportTuple {
    ModelTransportTuple {
        profile: "morphir-model".into(),
        codec: ModelCodec::IonBinary,
        ..a_json_tuple(operation)
    }
}

/// The limits that the MoonBit CLI worker host offers.
pub fn moonbit_host_limits() -> ModelTransportLimits {
    ModelTransportLimits {
        max_encoded_bytes: 2_796_204,
        max_decoded_bytes: 2 * 1024 * 1024,
        max_depth: 128,
        max_nodes: 100_000,
    }
}

/// A host offer like the MoonBit CLI's: Ion first, then Morphir JSON, for both operations.
pub fn a_moonbit_like_offer() -> ModelTransportSupport {
    ModelTransportSupport {
        tuples: vec![
            an_ion_tuple(ModelOperation::CompileModel),
            an_ion_tuple(ModelOperation::GenerateModel),
            a_json_tuple(ModelOperation::CompileModel),
            a_json_tuple(ModelOperation::GenerateModel),
        ],
        limits: moonbit_host_limits(),
        guarantees: vec![],
    }
}

/// A host offer with only Ion tuples.
pub fn an_ion_only_offer() -> ModelTransportSupport {
    ModelTransportSupport {
        tuples: vec![
            an_ion_tuple(ModelOperation::CompileModel),
            an_ion_tuple(ModelOperation::GenerateModel),
        ],
        ..a_moonbit_like_offer()
    }
}

/// A native extension and the result of its `morphir.initialize`.
pub struct Session {
    pub extension: NativeExtension,
    pub initialized: ExtensionResponse,
    next_id: std::cell::Cell<u64>,
}

impl Session {
    /// Initialize a native Rust extension with `versions` and an optional model transport `offer`.
    pub fn start(versions: &[&str], offer: Option<ModelTransportSupport>) -> Self {
        let extension = NativeExtension::frontend_backend(RustExtension).unwrap();
        let initialized = extension.protocol().handle(
            ExtensionRequest::new(
                methods::INITIALIZE,
                InitializeParams {
                    model_transport: offer,
                    protocol_versions: versions.iter().map(|v| (*v).into()).collect(),
                    host: PeerInfo {
                        kind: Default::default(),
                        name: "model-test".into(),
                        version: "1.0.0".into(),
                    },
                },
                0,
            )
            .unwrap(),
        );
        Self {
            extension,
            initialized,
            next_id: std::cell::Cell::new(1),
        }
    }

    /// A session on MEP `0.1`.
    pub fn on_0_1() -> Self {
        let session = Self::start(&["0.1"], None);
        assert!(
            session.initialized.error.is_none(),
            "{:?}",
            session.initialized.error
        );
        session
    }

    /// A session on MEP `0.2.0-draft.1` with the MoonBit-like offer.
    pub fn on_model_line() -> Self {
        let session = Self::start(&[MODEL_MEP_VERSION, "0.1"], Some(a_moonbit_like_offer()));
        assert!(
            session.initialized.error.is_none(),
            "{:?}",
            session.initialized.error
        );
        session
    }

    /// The initialize result as JSON.
    pub fn result(&self) -> &Value {
        self.initialized.result.as_ref().expect("initialize result")
    }

    /// Send one request and return its response.
    pub fn call(&self, method: &str, params: impl serde::Serialize) -> ExtensionResponse {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.extension
            .protocol()
            .handle(ExtensionRequest::new(method, params, id).unwrap())
    }
}

/// A `compileModel` request for the golden source.
pub fn a_compile_model_request() -> CompileModelRequest {
    a_compile_model_request_for("models.rs", SOURCE)
}

/// A `compileModel` request for one document.
pub fn a_compile_model_request_for(uri: &str, text: &str) -> CompileModelRequest {
    CompileModelRequest {
        language_id: "rust".into(),
        sources: SourceSet {
            root: None,
            documents: vec![SourceDocument {
                uri: uri.into(),
                language_id: "rust".into(),
                version: 1,
                text: text.into(),
            }],
        },
        package: CompilePackage {
            name: "acme/example".into(),
            exposed_modules: None,
        },
        dependencies: vec![],
        options: CompileOptions {
            types_only: false,
            ir_version: "4.0.0".into(),
            ..Default::default()
        },
        context_digest: "0".repeat(64),
        output: a_json_tuple(ModelOperation::CompileModel),
    }
}

/// The legacy `compile` request for the same source at IR v4.
pub fn a_legacy_compile_request() -> CompileRequest {
    CompileRequest {
        language_id: "rust".into(),
        sources: a_compile_model_request().sources,
        package: CompilePackage {
            name: "acme/example".into(),
            exposed_modules: None,
        },
        options: CompileOptions {
            types_only: false,
            ir_version: "4".into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// A `generateModel` request for `model`.
pub fn a_generate_model_request(model: MorphirModel) -> GenerateModelRequest {
    GenerateModelRequest {
        model,
        input: a_json_tuple(ModelOperation::GenerateModel),
        target: "rust".into(),
        context_digest: "1".repeat(64),
        required_guarantees: vec![],
        options: Default::default(),
    }
}

/// Admit a compiled model envelope for `operation` and decode its payload as JSON.
pub fn payload_json(model: &MorphirModel, operation: ModelOperation) -> Value {
    let retained = morphir_extension_sdk::model::retain_model(
        model,
        &a_json_tuple(operation),
        &moonbit_host_limits(),
    )
    .unwrap();
    serde_json::from_slice(retained.bytes()).unwrap()
}
