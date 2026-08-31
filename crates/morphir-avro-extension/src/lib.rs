//! Apache Avro backend extension for Morphir.
//!
//! Decode supported Morphir IR with [`normalize()`], then create a renderer-neutral
//! Avro model with [`project`]. The projection keeps value bodies out of the
//! backend while retaining public type roots and source identity metadata.

mod avro;
mod diagnostic;
mod internal;
mod model;
mod naming;
mod normalize;
mod options;
mod render;

pub use avro::{
    AvroField, AvroFullName, AvroMessage, AvroPackage, AvroRequest, AvroRoot, AvroType, AvroUnion,
    EnumSchema, FixedSchema, NamedSchema, Properties, Protocol, RecordSchema, UnionError, project,
};
pub use diagnostic::{AvroDiagnostic, ProjectedDiagnostic};
pub use internal::{AvroGenerationError, AvroInternalError};
pub use model::{
    Constructor, DistributionKind, EntryPointKind, EntryPointMetadata, IncompletenessKind,
    NamedType, ProjectionDependency, ProjectionModule, ProjectionPackage, TypeDeclaration,
    TypeExpr, ValueKind, ValueSpecification,
};
pub use naming::escape_idl_identifier;
pub use normalize::{NormalizeError, normalize};
pub use options::{
    Aliases, AvroOptions, Dependencies, Projection, Representation, TypeMapping, Unsupported,
};
pub use render::{render_idl, render_json};

use morphir_extension_sdk::{
    Backend, BackendCapability, Diagnostic, DiagnosticSeverity, Extension, ExtensionCapabilities,
    ExtensionError, ExtensionInfo, ExtensionType, GenerateRequest, GenerateResult,
};

/// Portable Morphir backend that projects specifications into Apache Avro.
#[derive(Default)]
pub struct AvroExtension;

impl Extension for AvroExtension {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "morphir-avro".into(),
            name: "Morphir Avro".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: Some("Projects Morphir specifications into Apache Avro".into()),
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
                targets: vec!["avro".into()],
                ir_versions: vec!["3".into(), "4".into()],
                generate: true,
            }),
            ..ExtensionCapabilities::default()
        }
    }
}

impl Backend for AvroExtension {
    fn generate(&self, request: GenerateRequest) -> morphir_extension_sdk::Result<GenerateResult> {
        backend_generate_with(request, generate_request)
    }

    fn target_languages() -> Vec<String> {
        vec!["avro".into()]
    }
}

morphir_extension_sdk::export_extension!(AvroExtension, backend);

/// Decode one MEP generation request and return backend diagnostics as data.
///
/// Option decoding runs before IR normalization so an invalid option has stable
/// precedence over errors in the supplied Morphir document. Protocol decoding
/// remains the SDK dispatcher's responsibility.
pub fn generate_request(request: GenerateRequest) -> Result<GenerateResult, AvroInternalError> {
    let options = match AvroOptions::from_map(&request.options) {
        Ok(options) => options,
        Err(diagnostic) => {
            return Ok(failed_generation(
                diagnostic.into_diagnostic(DiagnosticSeverity::Error),
            ));
        }
    };
    let package = match normalize(&request.ir) {
        Ok(package) => package,
        Err(error) => {
            return Ok(failed_generation(Diagnostic {
                severity: DiagnosticSeverity::Error,
                code: Some(error.code().into()),
                message: error.to_string(),
                location: None,
                related: Vec::new(),
            }));
        }
    };
    try_generate(&package, &options)
}

fn backend_generate_with(
    request: GenerateRequest,
    generate: impl FnOnce(GenerateRequest) -> Result<GenerateResult, AvroInternalError>,
) -> morphir_extension_sdk::Result<GenerateResult> {
    generate(request).map_err(|error| ExtensionError::ExecutionFailed(error.to_string()))
}

fn failed_generation(diagnostic: Diagnostic) -> GenerateResult {
    GenerateResult {
        success: false,
        artifacts: Vec::new(),
        diagnostics: vec![diagnostic],
    }
}

/// Project a body-free Morphir package and render its Avro artifacts.
///
/// Domain failures are returned as protocol diagnostics with no artifacts.
/// Warn-and-skip projection keeps closed artifacts and warning locations.
pub fn generate(package: &ProjectionPackage, options: &AvroOptions) -> GenerateResult {
    match try_generate(package, options) {
        Ok(result) => result,
        Err(error) => failed_generation(Diagnostic {
            severity: DiagnosticSeverity::Error,
            code: None,
            message: error.to_string(),
            location: None,
            related: Vec::new(),
        }),
    }
}

fn try_generate(
    package: &ProjectionPackage,
    options: &AvroOptions,
) -> Result<GenerateResult, AvroInternalError> {
    let projected = match project(package, options) {
        Ok(projected) => projected,
        Err(error) => {
            let diagnostics = error.into_diagnostics()?;
            return Ok(GenerateResult {
                success: false,
                artifacts: Vec::new(),
                diagnostics: diagnostics
                    .into_iter()
                    .map(|diagnostic| diagnostic.into_diagnostic(DiagnosticSeverity::Error))
                    .collect(),
            });
        }
    };
    let rendered = match options.representation {
        Representation::Json => render_json(&projected, options.dependencies),
        Representation::Idl => render_idl(&projected, options.dependencies),
    };
    match rendered {
        Ok(artifacts) => Ok(GenerateResult {
            success: true,
            artifacts,
            diagnostics: projected
                .diagnostics()
                .iter()
                .cloned()
                .map(ProjectedDiagnostic::into_diagnostic)
                .collect(),
        }),
        Err(error) => {
            let diagnostics = error.into_diagnostics()?;
            Ok(GenerateResult {
                success: false,
                artifacts: Vec::new(),
                diagnostics: diagnostics
                    .into_iter()
                    .map(|diagnostic| diagnostic.into_diagnostic(DiagnosticSeverity::Error))
                    .collect(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use morphir_extension_sdk::{ExtensionError, GenerateRequest};

    use super::{AvroInternalError, backend_generate_with};

    #[test]
    fn backend_maps_an_injected_invariant_failure_to_execution_failed() {
        let result = backend_generate_with(GenerateRequest::default(), |_| {
            Err(AvroInternalError::invariant(
                "injected renderer registry failure",
            ))
        });

        let error = result.expect_err("an internal invariant must fail the MEP operation");
        match error {
            ExtensionError::ExecutionFailed(message) => {
                assert!(message.contains("injected renderer registry failure"));
            }
            other => panic!("expected ExecutionFailed, got {other}"),
        }
    }
}
