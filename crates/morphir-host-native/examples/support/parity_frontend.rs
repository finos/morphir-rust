//! One frontend that runs both in the host process and as a stdio guest, so
//! a test can give the same compile call to each and compare the answers.

use morphir_extension_sdk::{
    CompileRequest, CompileResult, Diagnostic, DiagnosticSeverity, Extension,
    ExtensionCapabilities, ExtensionInfo, Frontend, FrontendCapability, LanguageCapability,
    NativeExtension, Result,
};

pub const LANGUAGE: &str = "parity";

#[derive(Default)]
pub struct ParityFrontend;

impl ParityFrontend {
    /// The frontend as a built-in extension.
    pub fn native() -> NativeExtension {
        NativeExtension::frontend_only(Self).expect("the parity frontend declares a frontend")
    }
}

impl Extension for ParityFrontend {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "mep-native-frontend".into(),
            name: "MEP native frontend fixture".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            ..ExtensionInfo::default()
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                languages: vec![LanguageCapability {
                    id: LANGUAGE.into(),
                    file_extensions: vec![".parity".into()],
                }],
                ir_versions: vec!["4".into()],
                compile: true,
                ..FrontendCapability::default()
            }),
            ..ExtensionCapabilities::default()
        }
    }
}

impl Frontend for ParityFrontend {
    /// Answer with one module and one diagnostic for each document, so the
    /// result depends on the request and not only on the frontend.
    fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
        let documents = &request.sources.documents;
        Ok(CompileResult {
            success: true,
            ir_version: Some(request.options.ir_version.clone()),
            ir: Some(serde_json::json!({
                "Library": {
                    "packageName": request.package.name,
                    "dependencies": {},
                    "def": {"modules": {}}
                }
            })),
            diagnostics: documents
                .iter()
                .map(|document| Diagnostic {
                    severity: DiagnosticSeverity::Info,
                    code: Some("P001".into()),
                    message: format!("compiled {} bytes", document.text.len()),
                    location: None,
                    related: vec![],
                })
                .collect(),
            modules: documents
                .iter()
                .map(|document| module_name(&document.uri))
                .collect(),
            module_results: vec![],
            context_digest: None,
        })
    }

    fn supported_languages() -> Vec<String> {
        vec![LANGUAGE.into()]
    }

    fn file_extensions() -> Vec<String> {
        vec![".parity".into()]
    }
}

/// The last path segment of `uri` without its extension.
fn module_name(uri: &str) -> String {
    let file = uri.rsplit('/').next().unwrap_or(uri);
    file.split_once('.')
        .map_or(file, |(stem, _)| stem)
        .to_owned()
}
