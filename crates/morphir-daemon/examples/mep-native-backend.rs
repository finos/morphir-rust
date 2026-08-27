//! A native MEP backend used to prove the spawned-process host adapter.

use morphir_extension_sdk::protocol::{ExtensionRequest, ExtensionResponse, methods};
use morphir_extension_sdk::{
    Artifact, Backend, Diagnostic, DiagnosticSeverity, Extension, ExtensionCapabilities,
    ExtensionInfo, ExtensionType, GenerateRequest, GenerateResult, Result,
};
use std::io::{self, BufRead, Write};

const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

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
        ExtensionCapabilities::default()
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

    while let Some(body) = read_frame(&mut reader)? {
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
            break;
        }
    }

    Ok(())
}

fn read_frame(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut content_length = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            return Ok(None);
        }
        if header == "\r\n" || header == "\n" {
            break;
        }
        let (name, value) = header
            .split_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid MEP header"))?;
        if name.eq_ignore_ascii_case("content-length") {
            let length = value
                .trim()
                .parse::<usize>()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            if length > MAX_FRAME_BYTES {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "MEP frame exceeds fixture limit",
                ));
            }
            content_length = Some(length);
        }
    }

    let length = content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

fn write_frame(writer: &mut impl Write, response: &ExtensionResponse) -> io::Result<()> {
    let body = serde_json::to_vec(response).map_err(io::Error::other)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}
