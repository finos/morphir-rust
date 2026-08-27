//! A standalone MEP backend daemon used to prove JSON-RPC HTTP hosting.

use jsonrpsee::core::RpcResult;
use jsonrpsee::server::{RpcModule, ServerBuilder};
use jsonrpsee::types::ErrorObjectOwned;
use morphir_extension_sdk::protocol::{ExtensionRequest, methods};
use morphir_extension_sdk::{
    Artifact, Backend, Diagnostic, DiagnosticSeverity, Extension, ExtensionCapabilities,
    ExtensionInfo, ExtensionType, GenerateRequest, GenerateResult, Result,
};
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

#[derive(Default)]
struct HttpBackend;

impl Extension for HttpBackend {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "mep-http-backend".into(),
            name: "MEP HTTP backend fixture".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            types: vec![],
            ..ExtensionInfo::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities::default()
    }
}

impl Backend for HttpBackend {
    fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
        if request.ir.is_string() {
            return Ok(GenerateResult {
                success: false,
                artifacts: vec![],
                diagnostics: vec![Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    code: Some("H001".into()),
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

struct DaemonContext {
    shutdown: watch::Sender<bool>,
    hang_generate: bool,
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let hang_generate = std::env::args().any(|arg| arg == "--hang-generate");
    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let context = Arc::new(DaemonContext {
        shutdown: shutdown_tx,
        hang_generate,
    });
    let mut module = RpcModule::from_arc(context);

    register_method(&mut module, methods::INITIALIZE)?;
    register_method(&mut module, methods::COMPILE)?;
    register_method(&mut module, methods::VALIDATE)?;
    register_method(&mut module, methods::TRANSFORM)?;
    module.register_async_method(methods::GENERATE, |params, context, _| async move {
        if context.hang_generate {
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
        dispatch(methods::GENERATE, params.parse()?)
    })?;
    module.register_async_method(methods::SHUTDOWN, |params, context, _| async move {
        let result = dispatch(methods::SHUTDOWN, params.parse()?)?;
        let shutdown = context.shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let _ = shutdown.send(true);
        });
        RpcResult::Ok(result)
    })?;

    let server = ServerBuilder::default().build("127.0.0.1:0").await?;
    let address = server.local_addr()?;
    let handle = server.start(module);
    println!("http://{address}");
    std::io::stdout().flush()?;
    eprintln!("MEP HTTP fixture listening on {address}");

    shutdown_rx.changed().await?;
    handle.stop()?;
    handle.stopped().await;
    Ok(())
}

fn register_method(
    module: &mut RpcModule<DaemonContext>,
    method: &'static str,
) -> std::result::Result<(), jsonrpsee::core::RegisterMethodError> {
    module.register_method(method, move |params, _, _| {
        dispatch(method, params.parse()?)
    })?;
    Ok(())
}

fn dispatch(method: &str, params: serde_json::Value) -> RpcResult<serde_json::Value> {
    let request = ExtensionRequest::new(method, params, 0)
        .map_err(|error| ErrorObjectOwned::owned(-32602, error.to_string(), None::<()>))?;
    let response = morphir_extension_sdk::__dispatch_request::<HttpBackend>(
        &request,
        &[morphir_extension_sdk::__dispatch_backend::<HttpBackend>],
        &[ExtensionType::Backend],
    );
    if let Some(error) = response.error {
        return Err(ErrorObjectOwned::owned(
            error.code,
            error.message,
            error.data,
        ));
    }
    response
        .result
        .ok_or_else(|| ErrorObjectOwned::owned(-32603, "Empty extension result", None::<()>))
}
