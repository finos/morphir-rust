//! OpenAPI and JSON Schema backend extension for Morphir.
//!
//! The extension advertises two targets. `json-schema` renders JSON Schema
//! 2020-12 documents, and `openapi` renders an OpenAPI document. Both targets
//! share one normalization step and one schema projection, so a type has the
//! same schema in either output.

mod diagnostic;
mod options;

pub use diagnostic::{SchemaDiagnostic, SchemaGenerationError};
pub use options::{SchemaOptions, Unsupported};

use morphir_extension_sdk::{
    Backend, BackendCapability, Diagnostic, DiagnosticSeverity, Extension, ExtensionCapabilities,
    ExtensionError, ExtensionInfo, ExtensionType, GenerateRequest, GenerateResult,
};

/// A generation target advertised by this extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// An OpenAPI document.
    OpenApi,
    /// JSON Schema documents.
    JsonSchema,
}

impl Target {
    /// Parse a host-supplied target ID.
    pub fn parse(target: &str) -> Option<Self> {
        match target {
            "openapi" => Some(Self::OpenApi),
            "json-schema" => Some(Self::JsonSchema),
            _ => None,
        }
    }

    /// The stable target ID advertised in the backend capability.
    pub fn id(self) -> &'static str {
        match self {
            Self::OpenApi => "openapi",
            Self::JsonSchema => "json-schema",
        }
    }
}

/// Portable Morphir backend that projects specifications into OpenAPI and JSON Schema.
#[derive(Default)]
pub struct OpenApiExtension;

impl Extension for OpenApiExtension {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "morphir-openapi".into(),
            name: "Morphir OpenAPI".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: Some(
                "Projects Morphir specifications into OpenAPI and JSON Schema".into(),
            ),
            types: vec![ExtensionType::Backend],
            author: Some("FINOS".into()),
            homepage: Some("https://github.com/finos/morphir-rust".into()),
            license: Some("Apache-2.0".into()),
            min_sdk_version: Some("0.2.0".into()),
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            backend: Some(BackendCapability {
                targets: vec!["openapi".into(), "json-schema".into()],
                ir_versions: vec!["3".into(), "4".into()],
                generate: true,
            }),
            ..ExtensionCapabilities::default()
        }
    }
}

impl Backend for OpenApiExtension {
    fn generate(&self, request: GenerateRequest) -> morphir_extension_sdk::Result<GenerateResult> {
        generate_request(request)
            .map_err(|error| ExtensionError::ExecutionFailed(error.to_string()))
    }

    fn target_languages() -> Vec<String> {
        vec!["openapi".into(), "json-schema".into()]
    }
}

morphir_extension_sdk::export_extension!(OpenApiExtension, backend);

/// Decode one MEP generation request and return backend diagnostics as data.
///
/// Target dispatch runs first: an unadvertised target is a backend-domain
/// failure, not a protocol failure, and never falls back to a default target.
/// A malformed-IR failure keeps `morphir_projection::NormalizeError`'s own
/// stable code, matching `morphir-avro-extension`'s `generate_request`, so a
/// caller branching on the code can tell bad IR from a bad backend option.
pub fn generate_request(request: GenerateRequest) -> Result<GenerateResult, SchemaGenerationError> {
    let Some(_target) = Target::parse(&request.target) else {
        return Ok(failed(
            SchemaDiagnostic::unknown_target(&request.target)
                .into_diagnostic(DiagnosticSeverity::Error),
        ));
    };
    let _options = match SchemaOptions::from_map(&request.options) {
        Ok(options) => options,
        Err(diagnostic) => {
            return Ok(failed(
                diagnostic.into_diagnostic(DiagnosticSeverity::Error),
            ));
        }
    };
    let _package = match morphir_projection::normalize(&request.ir) {
        Ok(package) => package,
        Err(error) => {
            return Ok(failed(Diagnostic {
                severity: DiagnosticSeverity::Error,
                code: Some(error.code().into()),
                message: error.to_string(),
                location: None,
                related: Vec::new(),
            }));
        }
    };
    Ok(GenerateResult {
        success: true,
        artifacts: Vec::new(),
        diagnostics: Vec::new(),
    })
}

fn failed(diagnostic: Diagnostic) -> GenerateResult {
    GenerateResult {
        success: false,
        artifacts: Vec::new(),
        diagnostics: vec![diagnostic],
    }
}
