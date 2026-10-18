//! Explicit SDK process fixture capturing the backend's actual typed input.

use morphir_extension_sdk::protocol::{ExtensionRequest, methods};
use morphir_extension_sdk::{
    Artifact, Backend, BackendCapability, Extension, ExtensionCapabilities, ExtensionInfo,
    GenerateRequest, GenerateResult, NativeExtension,
};
use std::io;

#[path = "support/frame.rs"]
mod frame;

struct CapturingBackend;

impl Extension for CapturingBackend {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "mep-literal-json-backend".into(),
            ..Default::default()
        }
    }
    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            backend: Some(BackendCapability {
                targets: vec!["literal-json".into()],
                ir_versions: vec!["4".into()],
                generate: true,
            }),
            ..Default::default()
        }
    }
}

impl Backend for CapturingBackend {
    fn target_languages() -> Vec<String> {
        vec!["literal-json".into()]
    }
    fn generate(&self, request: GenerateRequest) -> morphir_extension_sdk::Result<GenerateResult> {
        Ok(GenerateResult {
            success: true,
            artifacts: vec![
                Artifact {
                    path: "received-ir.json".into(),
                    content: serde_json::to_string(&request.ir)?,
                    binary: false,
                },
                Artifact {
                    path: "received-option.json".into(),
                    content: serde_json::to_string(request.options.get("literal").unwrap())?,
                    binary: false,
                },
            ],
            diagnostics: vec![],
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[derive(serde::Deserialize)]
    struct MethodProbe {
        method: String,
        id: Option<u64>,
    }
    let guest = NativeExtension::backend_only(CapturingBackend)?;
    let mut reader = io::stdin().lock();
    let mut writer = io::stdout().lock();
    let mut awaiting_exit = false;
    while let Some(body) = frame::read_frame(&mut reader)? {
        let probe: MethodProbe = serde_json::from_slice(&body)?;
        if probe.id.is_none() {
            if probe.method == methods::EXIT && awaiting_exit {
                return Ok(());
            }
            if probe.method == methods::INITIALIZED && !awaiting_exit {
                continue;
            }
            return Err("unexpected protocol notification".into());
        }
        // Decode the original literal bytes directly through the SDK type.
        let request: ExtensionRequest = serde_json::from_slice(&body)?;
        let shutdown = request.method == methods::SHUTDOWN;
        frame::write_frame(&mut writer, &guest.protocol().handle(request))?;
        awaiting_exit |= shutdown;
    }
    Err("stdin closed before morphir.exit".into())
}
