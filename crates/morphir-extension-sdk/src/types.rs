//! Core types for the Morphir extension system
//!
//! These types are shared between the SDK (guest) and daemon (host).

use serde::ser::{Error as _, SerializeMap};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Extension type/capability
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionType {
    /// Frontend - parses source into IR
    Frontend,
    /// Backend - generates code from IR
    Backend,
    /// Transform - transforms IR to IR
    Transform,
    /// Validator - analyzes IR and produces diagnostics
    Validator,
}

/// Information about an extension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionInfo {
    /// Extension identifier (e.g., "morphir-gleam-binding")
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Version (semver)
    pub version: String,
    /// Description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Capabilities this extension provides
    pub types: Vec<ExtensionType>,
    /// Author
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Homepage URL
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// License (SPDX identifier)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Minimum SDK version required
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_sdk_version: Option<String>,
}

impl Default for ExtensionInfo {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            version: "0.1.0".to_string(),
            description: None,
            types: Vec::new(),
            author: None,
            homepage: None,
            license: None,
            min_sdk_version: None,
        }
    }
}

/// Source language supported by a frontend extension.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageCapability {
    /// Stable language identifier used in compile requests.
    pub id: String,
    /// File extensions recognized for this language.
    pub file_extensions: Vec<String>,
}

/// Compilation features advertised by a frontend extension.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendCapability {
    /// Source languages accepted by the frontend.
    pub languages: Vec<LanguageCapability>,
    /// Morphir IR versions the frontend can produce.
    pub ir_versions: Vec<String>,
    /// Whether the frontend accepts compile requests.
    pub compile: bool,
    /// Whether the frontend supports incremental compilation.
    pub incremental: bool,
    /// Whether the frontend can compile source fragments.
    pub fragments: bool,
}

/// Code-generation features advertised by a backend extension.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendCapability {
    /// Target formats the backend can generate.
    pub targets: Vec<String>,
    /// Morphir IR versions the backend can consume.
    pub ir_versions: Vec<String>,
    /// Whether the backend accepts generate requests.
    pub generate: bool,
}

/// Extension capabilities for runtime negotiation
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExtensionCapabilities {
    /// Frontend compilation features, when provided by the extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontend: Option<FrontendCapability>,
    /// Backend code-generation features, when provided by the extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<BackendCapability>,
    /// Supports streaming/incremental processing
    #[serde(default)]
    pub streaming: bool,
    /// Supports incremental compilation
    #[serde(default)]
    pub incremental: bool,
    /// Supports cancellation
    #[serde(default)]
    pub cancellation: bool,
    /// Supports progress reporting
    #[serde(default)]
    pub progress: bool,
    /// Additional capability values reserved for protocol extensions.
    ///
    /// Keys that duplicate a known capability field are rejected during serialization.
    #[serde(default, flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Serialize for ExtensionCapabilities {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        const RESERVED_KEYS: [&str; 6] = [
            "frontend",
            "backend",
            "streaming",
            "incremental",
            "cancellation",
            "progress",
        ];
        if let Some(key) = self
            .extra
            .keys()
            .find(|key| RESERVED_KEYS.contains(&key.as_str()))
        {
            return Err(S::Error::custom(format!(
                "extra contains reserved capability key '{key}'"
            )));
        }

        let mut map = serializer.serialize_map(Some(
            4 + usize::from(self.frontend.is_some())
                + usize::from(self.backend.is_some())
                + self.extra.len(),
        ))?;
        if let Some(frontend) = &self.frontend {
            map.serialize_entry("frontend", frontend)?;
        }
        if let Some(backend) = &self.backend {
            map.serialize_entry("backend", backend)?;
        }
        map.serialize_entry("streaming", &self.streaming)?;
        map.serialize_entry("incremental", &self.incremental)?;
        map.serialize_entry("cancellation", &self.cancellation)?;
        map.serialize_entry("progress", &self.progress)?;
        for (key, value) in &self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// Resource limits for extension execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum memory in bytes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_bytes: Option<u64>,
    /// Maximum execution time in milliseconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_time_ms: Option<u64>,
    /// Maximum fuel (instruction count)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fuel: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_bytes: Some(256 * 1024 * 1024), // 256 MB
            max_time_ms: Some(30_000),                 // 30 seconds
            max_fuel: Some(100_000_000),               // 100M instructions
        }
    }
}

/// Source document supplied to a frontend compiler.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDocument {
    /// URI that identifies the document.
    pub uri: String,
    /// Language identifier understood by the frontend.
    pub language_id: String,
    /// Monotonically increasing document version.
    pub version: u64,
    /// Complete source text for this document version.
    pub text: String,
}

/// Package metadata for a frontend compilation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompilePackage {
    /// Morphir package name.
    pub name: String,
    /// Modules exposed by the package.
    pub exposed_modules: Vec<String>,
}

/// A package distribution available to a frontend compilation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileDependency {
    /// Name of the dependency package.
    pub package_name: String,
    /// Morphir IR version used by the distribution.
    pub ir_version: String,
    /// Serialized Morphir distribution.
    pub distribution: serde_json::Value,
}

/// Options that control frontend compilation.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileOptions {
    /// Emit type information without value bodies when supported.
    pub types_only: bool,
    /// Morphir IR version the frontend should produce.
    pub ir_version: String,
    /// Frontend-specific compilation options.
    ///
    /// Keys that duplicate `typesOnly` or `irVersion` are rejected during serialization.
    #[serde(default, flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Serialize for CompileOptions {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        const RESERVED_KEYS: [&str; 2] = ["typesOnly", "irVersion"];
        if let Some(key) = self
            .extra
            .keys()
            .find(|key| RESERVED_KEYS.contains(&key.as_str()))
        {
            return Err(S::Error::custom(format!(
                "extra contains reserved compile option key '{key}'"
            )));
        }

        let mut map = serializer.serialize_map(Some(2 + self.extra.len()))?;
        map.serialize_entry("typesOnly", &self.types_only)?;
        map.serialize_entry("irVersion", &self.ir_version)?;
        for (key, value) in &self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// Request to compile source documents into Morphir IR.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileRequest {
    /// Language identifier shared by the submitted documents.
    pub language_id: String,
    /// Source documents to compile.
    pub documents: Vec<SourceDocument>,
    /// Package metadata for the compilation unit.
    pub package: CompilePackage,
    /// Package distributions available to the compilation.
    #[serde(default)]
    pub dependencies: Vec<CompileDependency>,
    /// Options that control the produced Morphir IR.
    pub options: CompileOptions,
}

/// Result of compilation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileResult {
    /// Whether compilation succeeded
    pub success: bool,
    /// Morphir IR version of the compiled output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir_version: Option<String>,
    /// Compiled IR (JSON)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir: Option<serde_json::Value>,
    /// Diagnostics
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    /// Module names produced by the compilation.
    #[serde(default)]
    pub modules: Vec<String>,
}

/// Request to generate code
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GenerateRequest {
    /// Input IR (JSON)
    pub ir: serde_json::Value,
    /// Generation options
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Result of code generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateResult {
    /// Whether generation succeeded
    pub success: bool,
    /// Generated artifacts
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    /// Diagnostics
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// Request to validate IR
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidateRequest {
    /// Input IR (JSON)
    pub ir: serde_json::Value,
    /// Validation options
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Result of validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateResult {
    /// Whether validation passed
    pub valid: bool,
    /// Diagnostics
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// Request to transform IR
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransformRequest {
    /// Input IR (JSON)
    pub ir: serde_json::Value,
    /// Transformation options
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Result of transformation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformResult {
    /// Whether transformation succeeded
    pub success: bool,
    /// Transformed IR (JSON)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir: Option<serde_json::Value>,
    /// Diagnostics
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// A diagnostic message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Severity level
    pub severity: DiagnosticSeverity,
    /// Error/warning code
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Human-readable message
    pub message: String,
    /// Source location
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<SourceLocation>,
    /// Related information
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedInformation>,
}

/// Diagnostic severity level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    /// Error - compilation fails
    Error,
    /// Warning - may indicate problems
    Warning,
    /// Information - neutral message
    Info,
    /// Hint - suggestion for improvement
    Hint,
}

/// Source code location identified by URI and zero-based range.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLocation {
    /// URI of the source document.
    pub uri: String,
    /// Zero-based range within the source document.
    pub range: SourceRange,
}

/// Half-open range between two zero-based source positions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRange {
    /// Inclusive start position.
    pub start: SourcePosition,
    /// Exclusive end position.
    pub end: SourcePosition,
}

/// Zero-based line and character position in a source document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePosition {
    /// Zero-based line number.
    pub line: u32,
    /// Zero-based UTF-16 code-unit offset within the line, following LSP conventions.
    pub character: u32,
}

impl SourcePosition {
    /// Construct a position from a zero-based line number and the source text before the position.
    ///
    /// `source_line_prefix` must contain only text from the specified line before the position.
    ///
    /// # Panics
    ///
    /// Panics if the prefix contains more UTF-16 code units than fit in a [`u32`].
    pub fn from_line_prefix(line: u32, source_line_prefix: &str) -> Self {
        Self {
            line,
            character: u32::try_from(source_line_prefix.encode_utf16().count())
                .expect("source line prefix exceeds the supported UTF-16 offset"),
        }
    }
}

/// Related diagnostic information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedInformation {
    /// Location of related information
    pub location: SourceLocation,
    /// Message
    pub message: String,
}

/// A generated artifact
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Output path (relative)
    pub path: String,
    /// Content (text or base64 for binary)
    pub content: String,
    /// Whether content is base64-encoded binary
    #[serde(default)]
    pub binary: bool,
}

/// Workspace information provided by host
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// Workspace root path
    pub root: String,
    /// Output directory path
    pub output_dir: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_request_matches_mep_0_1() {
        let expected = serde_json::json!({
            "languageId": "elm",
            "documents": [{
                "uri": "file:///work/Example.elm",
                "languageId": "elm",
                "version": 1,
                "text": "module Example exposing (add)\n"
            }],
            "package": {
                "name": "local/example",
                "exposedModules": ["Example"]
            },
            "dependencies": [],
            "options": {
                "typesOnly": false,
                "irVersion": "3"
            }
        });
        let request = CompileRequest {
            language_id: "elm".into(),
            documents: vec![SourceDocument {
                uri: "file:///work/Example.elm".into(),
                language_id: "elm".into(),
                version: 1,
                text: "module Example exposing (add)\n".into(),
            }],
            package: CompilePackage {
                name: "local/example".into(),
                exposed_modules: vec!["Example".into()],
            },
            dependencies: vec![],
            options: CompileOptions {
                types_only: false,
                ir_version: "3".into(),
                extra: HashMap::new(),
            },
        };

        assert_eq!(serde_json::to_value(request).unwrap(), expected);
        let decoded: CompileRequest = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
    }

    #[test]
    fn compile_request_serializes_dependencies_and_vendor_options() {
        let expected = serde_json::json!({
            "languageId": "elm",
            "documents": [],
            "package": {
                "name": "local/example",
                "exposedModules": []
            },
            "dependencies": [{
                "packageName": "morphir/sdk",
                "irVersion": "3",
                "distribution": {"modules": {}}
            }],
            "options": {
                "typesOnly": true,
                "irVersion": "3",
                "vendorOptimization": {"level": 2}
            }
        });
        let request = CompileRequest {
            language_id: "elm".into(),
            documents: vec![],
            package: CompilePackage {
                name: "local/example".into(),
                exposed_modules: vec![],
            },
            dependencies: vec![CompileDependency {
                package_name: "morphir/sdk".into(),
                ir_version: "3".into(),
                distribution: serde_json::json!({"modules": {}}),
            }],
            options: CompileOptions {
                types_only: true,
                ir_version: "3".into(),
                extra: HashMap::from([(
                    "vendorOptimization".into(),
                    serde_json::json!({"level": 2}),
                )]),
            },
        };

        assert_eq!(serde_json::to_value(request).unwrap(), expected);
        let decoded: CompileRequest = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
    }

    #[test]
    fn compile_options_reject_ir_version_extra_collision() {
        let options = CompileOptions {
            types_only: false,
            ir_version: "3".into(),
            extra: HashMap::from([("irVersion".into(), serde_json::json!("4"))]),
        };

        let error = serde_json::to_value(options).unwrap_err();
        assert!(error.to_string().contains("reserved compile option key"));
    }

    #[test]
    fn compile_options_reject_types_only_extra_collision() {
        let options = CompileOptions {
            types_only: false,
            ir_version: "3".into(),
            extra: HashMap::from([("typesOnly".into(), serde_json::json!(true))]),
        };

        let error = serde_json::to_value(options).unwrap_err();
        assert!(error.to_string().contains("reserved compile option key"));
    }

    #[test]
    fn compile_result_matches_mep_0_1() {
        let expected = serde_json::json!({
            "success": true,
            "irVersion": "3",
            "ir": {"formatVersion": 3},
            "diagnostics": [{
                "severity": "warning",
                "code": "unused-value",
                "message": "Value is not exposed",
                "location": {
                    "uri": "file:///work/Example.elm",
                    "range": {
                        "start": {"line": 2, "character": 4},
                        "end": {"line": 2, "character": 7}
                    }
                }
            }],
            "modules": ["Example"]
        });

        let result = CompileResult {
            success: true,
            ir_version: Some("3".into()),
            ir: Some(serde_json::json!({"formatVersion": 3})),
            diagnostics: vec![Diagnostic {
                severity: DiagnosticSeverity::Warning,
                code: Some("unused-value".into()),
                message: "Value is not exposed".into(),
                location: Some(SourceLocation {
                    uri: "file:///work/Example.elm".into(),
                    range: SourceRange {
                        start: SourcePosition {
                            line: 2,
                            character: 4,
                        },
                        end: SourcePosition {
                            line: 2,
                            character: 7,
                        },
                    },
                }),
                related: vec![],
            }],
            modules: vec!["Example".into()],
        };

        assert_eq!(serde_json::to_value(&result).unwrap(), expected);
        let decoded: CompileResult = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);

        let minimal: CompileResult = serde_json::from_value(serde_json::json!({
            "success": false
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(minimal).unwrap(),
            serde_json::json!({
                "success": false,
                "diagnostics": [],
                "modules": []
            })
        );
    }

    #[test]
    fn extension_capabilities_support_optional_frontend_contract() {
        let capabilities = ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                languages: vec![LanguageCapability {
                    id: "elm".into(),
                    file_extensions: vec![".elm".into()],
                }],
                ir_versions: vec!["3".into()],
                compile: true,
                incremental: false,
                fragments: false,
            }),
            ..ExtensionCapabilities::default()
        };
        let expected = serde_json::json!({
            "streaming": false,
            "incremental": false,
            "cancellation": false,
            "progress": false,
            "frontend": {
                "languages": [{"id": "elm", "fileExtensions": [".elm"]}],
                "irVersions": ["3"],
                "compile": true,
                "incremental": false,
                "fragments": false
            }
        });

        assert_eq!(serde_json::to_value(&capabilities).unwrap(), expected);
        let decoded: ExtensionCapabilities = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);

        let legacy = serde_json::json!({
            "streaming": true,
            "incremental": false,
            "cancellation": true,
            "progress": false,
            "vendorFeature": true
        });
        let decoded: ExtensionCapabilities = serde_json::from_value(legacy.clone()).unwrap();
        assert!(decoded.frontend.is_none());
        assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
    }

    #[test]
    fn backend_capability_round_trips() {
        let capabilities = ExtensionCapabilities {
            backend: Some(BackendCapability {
                targets: vec!["avro".into()],
                ir_versions: vec!["3".into(), "4".into()],
                generate: true,
            }),
            ..ExtensionCapabilities::default()
        };
        let json = serde_json::to_value(&capabilities).unwrap();

        assert_eq!(json["backend"]["targets"], serde_json::json!(["avro"]));
        assert_eq!(json["backend"]["irVersions"], serde_json::json!(["3", "4"]));
        assert_eq!(json["backend"]["generate"], true);
        assert_eq!(
            serde_json::from_value::<ExtensionCapabilities>(json)
                .unwrap()
                .backend
                .unwrap()
                .targets,
            ["avro"]
        );
    }

    #[test]
    fn extra_cannot_replace_the_typed_backend_capability() {
        let capabilities = ExtensionCapabilities {
            extra: HashMap::from([("backend".into(), serde_json::json!({}))]),
            ..ExtensionCapabilities::default()
        };

        assert!(serde_json::to_value(capabilities).is_err());
    }

    #[test]
    fn extension_capabilities_preserve_unknown_structured_values() {
        let expected = serde_json::json!({
            "streaming": false,
            "incremental": false,
            "cancellation": false,
            "progress": false,
            "vendorFrontend": {
                "modes": ["batch", "watch"],
                "limits": {"documents": 100}
            }
        });

        let decoded: ExtensionCapabilities = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
    }

    #[test]
    fn extension_capabilities_reject_reserved_extra_keys() {
        for key in [
            "frontend",
            "streaming",
            "incremental",
            "cancellation",
            "progress",
        ] {
            let capabilities = ExtensionCapabilities {
                extra: HashMap::from([(key.into(), serde_json::json!({"override": true}))]),
                ..ExtensionCapabilities::default()
            };

            let error = serde_json::to_value(capabilities).unwrap_err();
            assert!(error.to_string().contains("reserved capability key"));
        }
    }

    #[test]
    fn source_position_counts_utf16_code_units() {
        let position = SourcePosition::from_line_prefix(4, "a😀");

        assert_eq!(position.line, 4);
        assert_eq!(position.character, 3);
    }
}
