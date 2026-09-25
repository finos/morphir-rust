//! A native MEP backend used to prove the spawned-process host adapter.

use morphir_extension_sdk::protocol::{ExtensionRequest, methods};
use morphir_extension_sdk::{
    Artifact, Backend, BackendCapability, Diagnostic, DiagnosticSeverity, Extension,
    ExtensionCapabilities, ExtensionInfo, ExtensionType, GenerateRequest, GenerateResult, Result,
};
use std::io;

#[path = "support/describe.rs"]
mod describe;
#[path = "support/frame.rs"]
mod frame;

use frame::{read_frame, write_frame};

#[derive(Default)]
struct NativeBackend;

impl Extension for NativeBackend {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "mep-native-backend".into(),
            name: "MEP native backend fixture".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            types: vec![],
            ..ExtensionInfo::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            backend: Some(BackendCapability {
                targets: vec!["json".into()],
                ir_versions: vec!["3".into(), "4".into()],
                generate: true,
            }),
            ..ExtensionCapabilities::default()
        }
    }
}

impl Backend for NativeBackend {
    fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
        if std::env::var_os("MEP_FIXTURE_HANG_GENERATE").is_some() {
            std::thread::sleep(std::time::Duration::from_secs(30));
        }
        if request.ir.is_string() {
            return Ok(GenerateResult {
                success: false,
                artifacts: vec![],
                diagnostics: vec![Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    code: Some("N001".into()),
                    message: "Expected a Morphir IR object".into(),
                    location: None,
                    related: vec![],
                }],
            });
        }

        Ok(GenerateResult {
            success: true,
            artifacts: vec![Artifact {
                path: "observed-ir.json".into(),
                content: request.ir.to_string(),
                binary: false,
            }],
            diagnostics: vec![],
        })
    }

    fn target_languages() -> Vec<String> {
        vec!["json".into()]
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--stderr-holder") {
        std::thread::sleep(std::time::Duration::from_secs(1));
        return Ok(());
    }

    eprintln!("native MEP fixture started");
    if std::env::var_os("MEP_FIXTURE_HOLD_STDERR_OPEN").is_some() {
        std::process::Command::new(std::env::current_exe()?)
            .arg("--stderr-holder")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()?;
    }
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let mut awaiting_exit = false;
    let mut describe_fixture = describe::DescribeFixture::from_environment();

    loop {
        let Some(body) = read_frame(&mut reader)? else {
            if awaiting_exit {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "stdin closed before morphir.exit",
                )
                .into());
            }
            break;
        };
        let message: serde_json::Value = serde_json::from_slice(&body)?;
        if let Some(fixture) = &mut describe_fixture {
            fixture.accept(&message)?;
        }
        if awaiting_exit {
            let notification: serde_json::Value = serde_json::from_slice(&body)?;
            if notification
                .get("jsonrpc")
                .and_then(serde_json::Value::as_str)
                != Some("2.0")
                || notification
                    .get("method")
                    .and_then(serde_json::Value::as_str)
                    != Some(methods::EXIT)
                || notification.get("id").is_some()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "expected morphir.exit notification",
                )
                .into());
            }
            break;
        }
        if message["jsonrpc"] == "2.0" && message.get("id").is_none() {
            if message["method"] == methods::EXIT {
                break;
            }
            if message["method"] == methods::INITIALIZED {
                continue;
            }
        }
        let request: ExtensionRequest = serde_json::from_slice(&body)?;
        let shutdown = request.method == methods::SHUTDOWN;
        let mut response = morphir_extension_sdk::__dispatch_request::<NativeBackend>(
            &request,
            &[morphir_extension_sdk::__dispatch_backend::<NativeBackend>],
            &[ExtensionType::Backend],
        );
        if std::env::var_os("MEP_FIXTURE_INVALID_ENVELOPE").is_some() {
            response.jsonrpc = "1.0".into();
        }
        if request.method == methods::INITIALIZE
            && std::env::var_os("MEP_FIXTURE_UNSUPPORTED_PROTOCOL").is_some()
            && let Some(result) = response
                .result
                .as_mut()
                .and_then(serde_json::Value::as_object_mut)
        {
            result.insert("protocolVersion".into(), "unsupported".into());
        }
        if let Some(fixture) = &describe_fixture {
            fixture.adjust(&request, &mut response);
        }
        write_frame(&mut writer, &response)?;
        if request.method == methods::INITIALIZE
            && std::env::var_os("MEP_FIXTURE_HANG_AFTER_INITIALIZE").is_some()
        {
            std::thread::sleep(std::time::Duration::from_secs(30));
        }
        if shutdown {
            if std::env::var_os("MEP_FIXTURE_IGNORE_SHUTDOWN").is_some() {
                std::thread::sleep(std::time::Duration::from_secs(30));
            }
            awaiting_exit = true;
        }
    }

    Ok(())
}
