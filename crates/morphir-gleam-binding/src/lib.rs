//! Morphir Gleam Binding Extension
//!
//! This extension provides Gleam language support for Morphir:
//! - Frontend: Parse Gleam source files to Morphir IR
//! - Backend: Generate Gleam code from Morphir IR

use indexmap::IndexMap;
use morphir_common::vfs::OsVfs;
use morphir_core::ir::v4::{
    Access as MorphirAccess, Distribution, FormatVersion, IRFile, LibraryContent, PackageDefinition,
};
use morphir_core::naming::{ModuleName, Name, PackageName};
use morphir_extension_sdk::prelude::*;
use percent_encoding::percent_decode_str;
use std::collections::HashSet;
use std::path::PathBuf;
use url::Url;

const GLEAM_IR_VERSION: &str = "4.0.0";

pub mod backend;
pub mod frontend;
pub mod roundtrip;

/// Gleam extension implementing both Frontend and Backend
#[derive(Default)]
pub struct GleamExtension;

impl Extension for GleamExtension {
    fn info() -> ExtensionInfo {
        ExtensionInfo {
            id: "morphir-gleam-binding".into(),
            name: "Morphir Gleam Binding".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            description: Some("Gleam language support for Morphir".into()),
            types: vec![ExtensionType::Frontend, ExtensionType::Backend],
            author: Some("FINOS".into()),
            license: Some("Apache-2.0".into()),
            homepage: Some("https://github.com/finos/morphir-rust".into()),
            min_sdk_version: Some("0.1.0".into()),
        }
    }

    fn capabilities() -> ExtensionCapabilities {
        ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                languages: vec![LanguageCapability {
                    id: "gleam".into(),
                    file_extensions: vec![".gleam".into()],
                }],
                ir_versions: vec![GLEAM_IR_VERSION.into()],
                compile: true,
                incremental: false,
                fragments: false,
            }),
            backend: Some(BackendCapability {
                targets: vec!["gleam".into()],
                ir_versions: vec![GLEAM_IR_VERSION.into()],
                generate: true,
            }),
            ..Default::default()
        }
    }
}

impl Frontend for GleamExtension {
    fn compile(&self, request: CompileRequest) -> Result<CompileResult> {
        if request.options.ir_version != GLEAM_IR_VERSION {
            return Ok(unsupported_ir_version(&request.options.ir_version));
        }

        if let Some(diagnostic) = validate_request_semantics(&request) {
            return Ok(failed_compile(vec![diagnostic]));
        }

        let package_name = match validate_package_name(&request.package.name) {
            Ok(package_name) => package_name,
            Err(message) => {
                return Ok(failed_compile(vec![error_diagnostic(
                    "INVALID_PACKAGE_NAME",
                    message,
                    None,
                )]));
            }
        };
        let dependencies = match frontend::dependencies::package_specifications(
            &request.dependencies,
            GLEAM_IR_VERSION,
        ) {
            Ok(dependencies) => dependencies,
            Err(errors) => {
                return Ok(failed_compile(
                    errors
                        .into_iter()
                        .map(|error| error_diagnostic(error.code, error.message, None))
                        .collect(),
                ));
            }
        };

        host_info!("Compiling {} Gleam source file(s)", request.documents.len());

        let output_dir = request
            .options
            .extra
            .get("outputDir")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let emit_parse_stage = request
            .options
            .extra
            .get("emitParseStage")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let emit_parse_stage_fatal = request
            .options
            .extra
            .get("emitParseStageFatal")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let source_root = request
            .options
            .extra
            .get("sourceRootUri")
            .or_else(|| request.options.extra.get("sourceRoot"))
            .and_then(|value| value.as_str());

        let prepared = match prepare_documents(&request.documents, source_root) {
            Ok(prepared) => prepared,
            Err(diagnostics) => return Ok(failed_compile(diagnostics)),
        };
        let exposed_modules = match validate_exposed_modules(
            &request.package.exposed_modules,
            prepared.iter().map(|document| document.module_key.as_str()),
        ) {
            Ok(exposed) => exposed,
            Err(diagnostics) => return Ok(failed_compile(diagnostics)),
        };

        let mut parsed_modules = Vec::with_capacity(prepared.len());
        let mut diagnostics = Vec::new();
        for document in prepared {
            match frontend::parse_gleam(&format!("{}.gleam", document.module_key), &document.text) {
                Ok(module) => parsed_modules.push((document, module)),
                Err(error) => diagnostics.push(error.to_diagnostic(&document.uri, &document.text)),
            }
        }
        if !diagnostics.is_empty() {
            return Ok(failed_compile(diagnostics));
        }

        let mut modules = IndexMap::new();
        for (document, module_ir) in &parsed_modules {
            let visitor = frontend::GleamToMorphirVisitor::new(
                OsVfs,
                output_dir.clone(),
                package_name.clone(),
                document.module_name.clone(),
            );
            let access = if exposed_modules.contains(&document.module_key) {
                MorphirAccess::Public
            } else {
                MorphirAccess::Private
            };
            match visitor.build_module_definition(module_ir, access) {
                Ok(module) => {
                    modules.insert(document.module_key.clone(), module);
                }
                Err(error) => diagnostics.push(error_diagnostic(
                    "IR_CONVERSION_ERROR",
                    format!("Failed to convert to Morphir IR: {error}"),
                    Some(&document.uri),
                )),
            }
        }
        if !diagnostics.is_empty() {
            return Ok(failed_compile(diagnostics));
        }

        let distribution = Distribution::Library(LibraryContent {
            package_name,
            dependencies,
            def: PackageDefinition { modules },
        });

        if emit_parse_stage {
            let parse_modules = parsed_modules
                .iter()
                .map(
                    |(document, module)| frontend::parse_stage::ParseStageModule {
                        module_name: document.module_key.as_str(),
                        uri: document.uri.as_str(),
                        module,
                    },
                )
                .collect::<Vec<_>>();
            let outcome = frontend::parse_stage::emit_parse_stage(&output_dir, &parse_modules);
            if let Some((diagnostic, fatal)) =
                parse_stage_diagnostic(outcome, emit_parse_stage_fatal)
            {
                if fatal {
                    return Ok(failed_compile(vec![diagnostic]));
                }
                diagnostics.push(diagnostic);
            }
        }

        let module_names = match &distribution {
            Distribution::Library(content) => content.def.modules.keys().cloned().collect(),
            _ => unreachable!("Gleam produces Library distributions"),
        };
        let ir_file = IRFile {
            format_version: FormatVersion::default(),
            distribution,
        };
        Ok(CompileResult {
            success: true,
            ir_version: Some(GLEAM_IR_VERSION.into()),
            ir: Some(serde_json::to_value(ir_file)?),
            diagnostics,
            modules: module_names,
        })
    }

    fn supported_languages() -> Vec<String> {
        vec!["gleam".into()]
    }

    fn file_extensions() -> Vec<String> {
        vec![".gleam".into()]
    }
}

fn unsupported_ir_version(requested_version: &str) -> CompileResult {
    failed_compile(vec![Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: Some("UNSUPPORTED_IR_VERSION".into()),
        message: format!(
            "Unsupported Morphir IR version '{requested_version}'; Gleam supports '{GLEAM_IR_VERSION}'"
        ),
        location: None,
        related: vec![],
    }])
}

fn document_location(uri: &str) -> SourceLocation {
    SourceLocation {
        uri: uri.into(),
        range: SourceRange::default(),
    }
}

fn failed_compile(diagnostics: Vec<Diagnostic>) -> CompileResult {
    CompileResult {
        success: false,
        ir_version: None,
        ir: None,
        diagnostics,
        modules: vec![],
    }
}

fn error_diagnostic(code: &str, message: impl Into<String>, uri: Option<&str>) -> Diagnostic {
    Diagnostic {
        severity: DiagnosticSeverity::Error,
        code: Some(code.into()),
        message: message.into(),
        location: uri.map(document_location),
        related: vec![],
    }
}

fn parse_stage_diagnostic(
    outcome: frontend::parse_stage::EmitParseStageOutcome,
    fatal: bool,
) -> Option<(Diagnostic, bool)> {
    use frontend::parse_stage::EmitParseStageOutcome;

    let (severity, code, message, uri, is_fatal) = match outcome {
        EmitParseStageOutcome::Committed {
            cleanup_warning: None,
        } => return None,
        EmitParseStageOutcome::Committed {
            cleanup_warning: Some(warning),
        } => (
            DiagnosticSeverity::Warning,
            "PARSE_STAGE_CLEANUP_WARNING",
            warning,
            None,
            false,
        ),
        EmitParseStageOutcome::RolledBack { failure } => (
            if fatal {
                DiagnosticSeverity::Error
            } else {
                DiagnosticSeverity::Warning
            },
            "PARSE_STAGE_EMIT_ERROR",
            format!("Failed to emit parse stage output: {}", failure.message),
            failure.uri,
            fatal,
        ),
        EmitParseStageOutcome::RecoveryRequired {
            failure,
            recovery_path,
        } => (
            if fatal {
                DiagnosticSeverity::Error
            } else {
                DiagnosticSeverity::Warning
            },
            "PARSE_STAGE_RECOVERY_REQUIRED",
            format!(
                "Failed to emit parse stage output: {}; recovery artifacts retained at '{}'",
                failure.message,
                recovery_path.display()
            ),
            None,
            fatal,
        ),
    };
    Some((
        Diagnostic {
            severity,
            code: Some(code.into()),
            message,
            location: uri.as_deref().map(document_location),
            related: vec![],
        },
        is_fatal,
    ))
}

fn validate_request_semantics(request: &CompileRequest) -> Option<Diagnostic> {
    if request.language_id != "gleam" {
        return Some(error_diagnostic(
            "UNSUPPORTED_LANGUAGE",
            format!(
                "Expected request language 'gleam', got '{}'",
                request.language_id
            ),
            None,
        ));
    }
    if let Some(document) = request
        .documents
        .iter()
        .find(|document| document.language_id != "gleam")
    {
        return Some(error_diagnostic(
            "UNSUPPORTED_DOCUMENT_LANGUAGE",
            format!(
                "Expected document language 'gleam', got '{}'",
                document.language_id
            ),
            Some(&document.uri),
        ));
    }
    request.options.types_only.then(|| {
        error_diagnostic(
            "UNSUPPORTED_TYPES_ONLY",
            "The Gleam frontend does not support types-only compilation",
            None,
        )
    })
}

fn validate_package_name(value: &str) -> std::result::Result<PackageName, String> {
    frontend::dependencies::canonical_package_name(value)
}

#[derive(Debug)]
struct PreparedDocument {
    uri: String,
    text: String,
    module_name: ModuleName,
    module_key: String,
}

fn prepare_documents(
    documents: &[SourceDocument],
    source_root: Option<&str>,
) -> std::result::Result<Vec<PreparedDocument>, Vec<Diagnostic>> {
    let source_root = source_root.map(parsed_path).transpose();
    let source_root = match source_root {
        Ok(root) => root,
        Err(message) => {
            return Err(vec![error_diagnostic("INVALID_SOURCE_ROOT", message, None)]);
        }
    };
    let mut seen = HashSet::new();
    let mut prepared = Vec::with_capacity(documents.len());
    let mut diagnostics = Vec::new();
    for document in documents {
        match module_name_from_document_uri(&document.uri, source_root.as_ref()) {
            Ok(module_name) => {
                let module_key = module_name.to_string();
                if !seen.insert(module_key.clone()) {
                    diagnostics.push(error_diagnostic(
                        "DUPLICATE_MODULE",
                        format!("Duplicate Gleam module '{module_key}'"),
                        Some(&document.uri),
                    ));
                } else {
                    prepared.push(PreparedDocument {
                        uri: document.uri.clone(),
                        text: document.text.clone(),
                        module_name,
                        module_key,
                    });
                }
            }
            Err(message) => diagnostics.push(error_diagnostic(
                "INVALID_MODULE_URI",
                message,
                Some(&document.uri),
            )),
        }
    }
    if diagnostics.is_empty() {
        Ok(prepared)
    } else {
        Err(diagnostics)
    }
}

fn module_name_from_document_uri(
    uri: &str,
    source_root: Option<&ParsedPath>,
) -> std::result::Result<ModuleName, String> {
    let path = parsed_path(uri)?;
    // An explicit source root is authoritative. Without one, preserve the path
    // following the conventional `src` directory, or fall back to the basename.
    let relative = if let Some(root) = source_root {
        if !path.has_same_root_identity(root) {
            return Err(format!(
                "Document URI '{uri}' is outside the configured source root"
            ));
        }
        path.segments
            .strip_prefix(root.segments.as_slice())
            .ok_or_else(|| format!("Document URI '{uri}' is outside the configured source root"))?
            .to_vec()
    } else if let Some(src_index) = path.segments.iter().rposition(|segment| segment == "src") {
        path.segments[src_index + 1..].to_vec()
    } else {
        path.segments.last().cloned().into_iter().collect()
    };
    let mut module_segments = relative;
    let file_name = module_segments
        .last_mut()
        .ok_or_else(|| format!("Document URI '{uri}' has no module path"))?;
    *file_name = file_name
        .strip_suffix(".gleam")
        .ok_or_else(|| format!("Document URI '{uri}' does not identify a .gleam file"))?
        .to_owned();
    let refs = module_segments
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    canonicalize_gleam_module_segments(&refs)
}

#[derive(Debug, PartialEq, Eq)]
enum PathOrigin {
    Url { scheme: String, authority: String },
    Windows,
    Path,
}

#[derive(Debug)]
struct ParsedPath {
    origin: PathOrigin,
    drive: Option<String>,
    segments: Vec<String>,
}

impl ParsedPath {
    fn has_same_root_identity(&self, other: &Self) -> bool {
        self.drive == other.drive
            && (self.origin == other.origin
                || matches_local_file_path(&self.origin, &other.origin)
                || matches_local_file_path(&other.origin, &self.origin))
    }
}

fn matches_local_file_path(url_origin: &PathOrigin, path_origin: &PathOrigin) -> bool {
    matches!(
        (url_origin, path_origin),
        (
            PathOrigin::Url { scheme, authority },
            PathOrigin::Windows | PathOrigin::Path
        ) if scheme == "file" && authority.is_empty()
    )
}

fn parsed_path(value: &str) -> std::result::Result<ParsedPath, String> {
    let is_windows_path = value.as_bytes().get(1) == Some(&b':')
        && value
            .as_bytes()
            .get(2)
            .is_some_and(|byte| *byte == b'/' || *byte == b'\\');
    validate_raw_path_structure(value, is_windows_path)?;
    let (path, origin) = if is_windows_path {
        (
            value
                .split(['?', '#'])
                .next()
                .unwrap_or(value)
                .replace('\\', "/"),
            PathOrigin::Windows,
        )
    } else if let Ok(url) = Url::parse(value) {
        (
            url.path().replace('\\', "/"),
            PathOrigin::Url {
                scheme: url.scheme().to_owned(),
                authority: url.authority().to_owned(),
            },
        )
    } else {
        (
            value
                .split(['?', '#'])
                .next()
                .unwrap_or(value)
                .replace('\\', "/"),
            PathOrigin::Path,
        )
    };
    let mut raw_segments = path.split('/').collect::<Vec<_>>();
    while raw_segments.first() == Some(&"") {
        raw_segments.remove(0);
    }
    while raw_segments.last() == Some(&"") {
        raw_segments.pop();
    }
    if raw_segments.iter().any(|segment| segment.is_empty()) {
        return Err(format!("Path '{value}' contains an empty segment"));
    }
    let drive = raw_segments
        .first()
        .is_some_and(|segment| segment.len() == 2 && segment.ends_with(':'))
        .then(|| raw_segments[0][..1].to_ascii_uppercase());
    if drive.is_some() {
        raw_segments.remove(0);
    }
    let segments = raw_segments
        .into_iter()
        .map(|segment| {
            percent_decode_str(segment)
                .decode_utf8()
                .map(|decoded| decoded.into_owned())
                .map_err(|_| format!("Path '{value}' contains invalid UTF-8 percent encoding"))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ParsedPath {
        origin,
        drive,
        segments,
    })
}

fn validate_raw_path_structure(
    value: &str,
    is_windows_path: bool,
) -> std::result::Result<(), String> {
    let normalized = value
        .split(['?', '#'])
        .next()
        .unwrap_or(value)
        .replace('\\', "/");
    let raw_path = if is_windows_path {
        normalized.as_str()
    } else if let Some((_, remainder)) = normalized.split_once("://") {
        if remainder.starts_with('/') {
            remainder
        } else {
            remainder.find('/').map_or("", |index| &remainder[index..])
        }
    } else if let Some((scheme, remainder)) = normalized.split_once(':') {
        if scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
        {
            remainder
        } else {
            normalized.as_str()
        }
    } else {
        normalized.as_str()
    };
    let segments = raw_path.split('/').collect::<Vec<_>>();
    for (index, segment) in segments.iter().enumerate() {
        if segment.is_empty() && index != 0 && index + 1 != segments.len() {
            return Err(format!("Path '{value}' contains an empty segment"));
        }
        let decoded = percent_decode_str(segment)
            .decode_utf8()
            .map_err(|_| format!("Path '{value}' contains invalid UTF-8 percent encoding"))?;
        if decoded == "." || decoded == ".." {
            return Err(format!("Path '{value}' contains a dot segment"));
        }
    }
    Ok(())
}

fn validate_module_source_segments(segments: &[&str]) -> std::result::Result<(), String> {
    if segments.is_empty() {
        return Err("module name cannot be empty".into());
    }
    for segment in segments {
        if segment.is_empty()
            || !segment
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase())
            || !segment
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(format!("Invalid module name segment '{segment}'"));
        }
    }
    Ok(())
}

fn canonicalize_gleam_module_segments(
    segments: &[&str],
) -> std::result::Result<ModuleName, String> {
    validate_module_source_segments(segments)?;
    let canonical = segments
        .iter()
        .map(|segment| Name::from(segment).to_string())
        .collect::<Vec<_>>()
        .join("/");
    Ok(ModuleName::parse(&canonical))
}

fn validate_exposed_modules<'a>(
    exposed: &[String],
    compiled: impl Iterator<Item = &'a str>,
) -> std::result::Result<HashSet<String>, Vec<Diagnostic>> {
    let compiled = compiled.collect::<HashSet<_>>();
    let mut validated = HashSet::new();
    let mut diagnostics = Vec::new();
    for module in exposed {
        let segments = module.split('/').collect::<Vec<_>>();
        match canonicalize_gleam_module_segments(&segments) {
            Err(message) => {
                diagnostics.push(error_diagnostic("INVALID_EXPOSED_MODULE", message, None));
            }
            Ok(module_name) => {
                let canonical = module_name.to_string();
                if !compiled.contains(canonical.as_str()) {
                    diagnostics.push(error_diagnostic(
                        "MISSING_EXPOSED_MODULE",
                        format!("Exposed module '{module}' was not compiled"),
                        None,
                    ));
                } else {
                    validated.insert(canonical);
                }
            }
        }
    }
    if diagnostics.is_empty() {
        Ok(validated)
    } else {
        Err(diagnostics)
    }
}

impl Backend for GleamExtension {
    fn generate(&self, request: GenerateRequest) -> Result<GenerateResult> {
        host_info!("Generating Gleam code from IR");

        match backend::generate_gleam(&request.ir, &request.options) {
            Ok(artifacts) => Ok(GenerateResult {
                success: true,
                artifacts,
                diagnostics: vec![],
            }),
            Err(e) => Ok(GenerateResult {
                success: false,
                artifacts: vec![],
                diagnostics: vec![Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    code: Some("G001".into()),
                    message: e.to_string() as String,
                    location: None,
                    related: vec![],
                }],
            }),
        }
    }

    fn target_languages() -> Vec<String> {
        vec!["gleam".into()]
    }
}

// Export the extension
morphir_extension_sdk::export_extension!(GleamExtension, frontend, backend);

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_core::ir::v4::{
        Access as MorphirAccess, AccessControlled, Distribution, Documented, FormatVersion, IRFile,
        Incompleteness, InputTypeEntry, LibraryContent, ModuleDefinition, PackageDefinition,
        PackageSpecification, SpecsContent, Type, TypeAttributes, TypeDefinition,
        TypeSpecification, ValueBody, ValueDefinition, ValueSpecification,
    };
    use std::collections::HashMap;

    const IR_VERSION: &str = "4.0.0";

    fn compile_request(
        uri: &str,
        text: &str,
        ir_version: &str,
    ) -> (CompileRequest, tempfile::TempDir) {
        let output_dir = tempfile::tempdir().expect("create temporary output directory");
        let mut extra = HashMap::new();
        extra.insert(
            "outputDir".into(),
            serde_json::json!(output_dir.path().to_string_lossy()),
        );
        extra.insert("emitParseStage".into(), serde_json::json!(false));

        (
            CompileRequest {
                language_id: "gleam".into(),
                documents: vec![SourceDocument {
                    uri: uri.into(),
                    language_id: "gleam".into(),
                    version: 1,
                    text: text.into(),
                }],
                package: CompilePackage {
                    name: "example/package".into(),
                    exposed_modules: vec![],
                },
                dependencies: vec![],
                options: CompileOptions {
                    types_only: false,
                    ir_version: ir_version.into(),
                    extra,
                },
            },
            output_dir,
        )
    }

    fn document(uri: &str, text: &str) -> SourceDocument {
        SourceDocument {
            uri: uri.into(),
            language_id: "gleam".into(),
            version: 1,
            text: text.into(),
        }
    }

    fn specs_dependency(package_name: &str) -> CompileDependency {
        CompileDependency {
            package_name: package_name.into(),
            ir_version: IR_VERSION.into(),
            distribution: serde_json::to_value(Distribution::Specs(SpecsContent {
                package_name: PackageName::parse(package_name),
                dependencies: IndexMap::new(),
                spec: PackageSpecification {
                    modules: IndexMap::new(),
                },
            }))
            .expect("serialize dependency distribution"),
        }
    }

    fn library(result: &CompileResult) -> LibraryContent {
        let ir_file: IRFile =
            serde_json::from_value(result.ir.clone().expect("successful result contains IR"))
                .expect("successful result is typed V4 IR");
        assert_eq!(ir_file.format_version, FormatVersion::default());
        match ir_file.distribution {
            Distribution::Library(content) => content,
            other => panic!("expected Library distribution, got {other:?}"),
        }
    }

    fn assert_typed_failure(result: &CompileResult) {
        assert!(!result.success);
        assert!(result.ir_version.is_none());
        assert!(result.ir.is_none());
        assert!(result.modules.is_empty());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        );
    }

    fn assert_directory_empty(directory: &tempfile::TempDir) {
        assert_eq!(
            std::fs::read_dir(directory.path())
                .expect("read temporary output directory")
                .count(),
            0
        );
    }

    #[test]
    fn capabilities_advertise_the_gleam_v4_frontend() {
        let capabilities = serde_json::to_value(GleamExtension::capabilities())
            .expect("serialize Gleam capabilities");

        assert_eq!(
            capabilities["frontend"],
            serde_json::json!({
                "languages": [{"id": "gleam", "fileExtensions": [".gleam"]}],
                "irVersions": [IR_VERSION],
                "compile": true,
                "incremental": false,
                "fragments": false
            })
        );
        assert_eq!(
            capabilities["backend"],
            serde_json::json!({
                "targets": ["gleam"],
                "irVersions": [IR_VERSION],
                "generate": true
            })
        );
    }

    #[test]
    fn compile_accepts_mep_documents_and_returns_v4_ir_and_modules() {
        let (request, _output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );

        let result = GleamExtension
            .compile(request)
            .expect("compile valid Gleam document");

        assert!(result.success);
        assert_eq!(result.ir_version.as_deref(), Some(IR_VERSION));
        let library = library(&result);
        assert_eq!(library.package_name.to_string(), "example/package");
        assert_eq!(library.def.modules.keys().collect::<Vec<_>>(), ["main"]);
        assert_eq!(result.modules, ["main"]);
        assert!(result.diagnostics.is_empty());
    }

    #[test]
    fn compile_preserves_a_typed_v4_specs_dependency() {
        let (mut request, _output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );
        let specification = PackageSpecification {
            modules: IndexMap::new(),
        };
        request.dependencies.push(CompileDependency {
            package_name: "example/dependency".into(),
            ir_version: IR_VERSION.into(),
            distribution: serde_json::to_value(Distribution::Specs(SpecsContent {
                package_name: PackageName::parse("example/dependency"),
                dependencies: IndexMap::new(),
                spec: specification.clone(),
            }))
            .expect("serialize dependency distribution"),
        });

        let result = GleamExtension
            .compile(request)
            .expect("compile with a V4 Specs dependency");

        assert!(result.success);
        assert_eq!(
            library(&result).dependencies.get("example/dependency"),
            Some(&specification)
        );
    }

    #[test]
    fn compile_derives_a_dependency_specification_from_a_v4_library_ir_file() {
        let (mut request, _output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );
        let module_definition = || ModuleDefinition {
            types: IndexMap::from([(
                "opaque-type".into(),
                AccessControlled {
                    access: MorphirAccess::Public,
                    value: Documented::new(
                        None,
                        TypeDefinition::CustomTypeDefinition {
                            type_params: vec![Name::from("parameter")],
                            constructors: AccessControlled {
                                access: MorphirAccess::Private,
                                value: vec![],
                            },
                        },
                    ),
                },
            )]),
            values: IndexMap::from([(
                "public-value".into(),
                AccessControlled {
                    access: MorphirAccess::Public,
                    value: Documented::new(
                        None,
                        ValueDefinition {
                            input_types: IndexMap::from([(
                                "argument".into(),
                                InputTypeEntry {
                                    type_attributes: None,
                                    input_type: Type::unit(TypeAttributes::default()),
                                },
                            )]),
                            output_type: Some(Type::unit(TypeAttributes::default())),
                            body: ValueBody::External {
                                external_name: "unused".into(),
                                target_platform: "test".into(),
                            },
                        },
                    ),
                },
            )]),
            doc: None,
        };
        let dependency = IRFile {
            format_version: FormatVersion::String(IR_VERSION.into()),
            distribution: Distribution::Library(LibraryContent {
                package_name: PackageName::parse("example/dependency"),
                dependencies: IndexMap::new(),
                def: PackageDefinition {
                    modules: IndexMap::from([
                        (
                            "public-module".into(),
                            AccessControlled {
                                access: MorphirAccess::Public,
                                value: module_definition(),
                            },
                        ),
                        (
                            "private-module".into(),
                            AccessControlled {
                                access: MorphirAccess::Private,
                                value: module_definition(),
                            },
                        ),
                    ]),
                },
            }),
        };
        request.dependencies.push(CompileDependency {
            package_name: "example/dependency".into(),
            ir_version: IR_VERSION.into(),
            distribution: serde_json::to_value(dependency).expect("serialize dependency IR file"),
        });

        let result = GleamExtension
            .compile(request)
            .expect("compile with a V4 Library dependency");
        let dependency = &library(&result).dependencies["example/dependency"];

        assert!(result.success);
        assert_eq!(
            dependency.modules.keys().collect::<Vec<_>>(),
            ["public-module"]
        );
        assert_eq!(
            dependency.modules["public-module"].types["opaque-type"],
            Documented::new(
                None,
                TypeSpecification::OpaqueTypeSpecification {
                    type_params: vec![Name::from("parameter")],
                }
            )
        );
        assert_eq!(
            dependency.modules["public-module"].values["public-value"],
            Documented::new(
                None,
                ValueSpecification {
                    inputs: IndexMap::from([(
                        "argument".into(),
                        Type::unit(TypeAttributes::default())
                    )]),
                    output: Type::unit(TypeAttributes::default()),
                }
            )
        );
    }

    #[test]
    fn compile_rejects_a_dependency_with_an_unsupported_ir_version_atomically() {
        let (mut request, output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );
        let mut dependency = specs_dependency("example/dependency");
        dependency.ir_version = "3".into();
        request.dependencies.push(dependency);

        let result = GleamExtension
            .compile(request)
            .expect("return a typed dependency failure");

        assert_typed_failure(&result);
        assert_eq!(
            result.diagnostics[0].code.as_deref(),
            Some("UNSUPPORTED_DEPENDENCY_IR_VERSION")
        );
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn compile_rejects_a_dependency_ir_file_with_a_mismatched_format_version() {
        let (mut request, output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );
        let mut dependency = specs_dependency("example/dependency");
        let distribution = serde_json::from_value(dependency.distribution)
            .expect("decode typed dependency distribution");
        dependency.distribution = serde_json::to_value(IRFile {
            format_version: FormatVersion::Integer(3),
            distribution,
        })
        .expect("serialize dependency IR file");
        request.dependencies.push(dependency);

        let result = GleamExtension
            .compile(request)
            .expect("return a typed dependency failure");

        assert_typed_failure(&result);
        assert_eq!(
            result.diagnostics[0].code.as_deref(),
            Some("DEPENDENCY_IR_VERSION_MISMATCH")
        );
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn compile_rejects_a_dependency_whose_distribution_has_another_package() {
        let (mut request, output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );
        let mut dependency = specs_dependency("actual/package");
        dependency.package_name = "declared/package".into();
        request.dependencies.push(dependency);

        let result = GleamExtension
            .compile(request)
            .expect("return a typed dependency failure");

        assert_typed_failure(&result);
        assert_eq!(
            result.diagnostics[0].code.as_deref(),
            Some("DEPENDENCY_PACKAGE_MISMATCH")
        );
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn compile_rejects_a_noncanonical_dependency_package_name() {
        let (mut request, output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );
        let mut dependency = specs_dependency("example/dependency");
        dependency.package_name = "Example/Dependency".into();
        request.dependencies.push(dependency);

        let result = GleamExtension
            .compile(request)
            .expect("return a typed dependency failure");

        assert_typed_failure(&result);
        assert_eq!(
            result.diagnostics[0].code.as_deref(),
            Some("INVALID_DEPENDENCY_PACKAGE_NAME")
        );
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn compile_rejects_duplicate_dependency_package_keys_atomically() {
        let (mut request, output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );
        request.dependencies = vec![
            specs_dependency("example/dependency"),
            specs_dependency("example/dependency"),
        ];

        let result = GleamExtension
            .compile(request)
            .expect("return a typed dependency failure");

        assert_typed_failure(&result);
        assert_eq!(
            result.diagnostics[0].code.as_deref(),
            Some("DUPLICATE_DEPENDENCY")
        );
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn compile_rejects_an_incomplete_dependency_type_that_has_no_specification() {
        let (mut request, output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );
        request.dependencies.push(CompileDependency {
            package_name: "example/dependency".into(),
            ir_version: IR_VERSION.into(),
            distribution: serde_json::to_value(Distribution::Library(LibraryContent {
                package_name: PackageName::parse("example/dependency"),
                dependencies: IndexMap::new(),
                def: PackageDefinition {
                    modules: IndexMap::from([(
                        "public-module".into(),
                        AccessControlled {
                            access: MorphirAccess::Public,
                            value: ModuleDefinition {
                                types: IndexMap::from([(
                                    "incomplete".into(),
                                    AccessControlled {
                                        access: MorphirAccess::Public,
                                        value: Documented::new(
                                            None,
                                            TypeDefinition::IncompleteTypeDefinition {
                                                type_params: vec![],
                                                incompleteness: Incompleteness::Draft,
                                                partial_type_expr: None,
                                            },
                                        ),
                                    },
                                )]),
                                values: IndexMap::new(),
                                doc: None,
                            },
                        },
                    )]),
                },
            }))
            .expect("serialize incomplete dependency"),
        });

        let result = GleamExtension
            .compile(request)
            .expect("return a typed dependency failure");

        assert_typed_failure(&result);
        assert_eq!(
            result.diagnostics[0].code.as_deref(),
            Some("INCOMPATIBLE_DEPENDENCY_DISTRIBUTION")
        );
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn compile_rejects_an_arbitrary_object_around_a_typed_dependency_distribution() {
        let (mut request, output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );
        let mut dependency = specs_dependency("example/dependency");
        dependency.distribution["unexpected"] = serde_json::json!({});
        request.dependencies.push(dependency);

        let result = GleamExtension
            .compile(request)
            .expect("return a typed dependency failure");

        assert_typed_failure(&result);
        assert_eq!(
            result.diagnostics[0].code.as_deref(),
            Some("INVALID_DEPENDENCY_DISTRIBUTION")
        );
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn compile_rejects_a_noncanonical_embedded_dependency_package_identity() {
        let (mut request, output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );
        let mut dependency = specs_dependency("example/dependency");
        dependency.distribution["Specs"]["packageName"] = serde_json::json!("Example/Dependency");
        request.dependencies.push(dependency);

        let result = GleamExtension
            .compile(request)
            .expect("return a typed dependency failure");

        assert_typed_failure(&result);
        // "Example" is a mixed-case segment, which the v4 name grammar rejects, so
        // the distribution is invalid rather than merely naming another package.
        // A mismatch between two well-formed names is covered by
        // compile_rejects_a_dependency_whose_distribution_has_another_package.
        assert_eq!(
            result.diagnostics[0].code.as_deref(),
            Some("INVALID_DEPENDENCY_DISTRIBUTION")
        );
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn successful_compile_serializes_the_v4_ir_file_root() {
        let (request, _output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );

        let result = GleamExtension
            .compile(request)
            .expect("compile valid Gleam document");
        let ir = result.ir.expect("successful IR");
        assert_eq!(ir["formatVersion"], 4);
        assert!(ir.get("distribution").is_some());
        let ir_file: IRFile = serde_json::from_value(ir).expect("deserialize V4 IR file root");

        assert_eq!(ir_file.format_version, FormatVersion::default());
        assert!(matches!(ir_file.distribution, Distribution::Library(_)));
    }

    #[test]
    fn backend_accepts_the_v4_ir_file_root_emitted_by_the_frontend() {
        let (request, _compile_output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );
        let compiled = GleamExtension
            .compile(request)
            .expect("compile valid Gleam document");
        let output_dir = tempfile::tempdir().expect("create backend output directory");

        let generated = GleamExtension
            .generate(GenerateRequest {
                ir: compiled.ir.expect("successful compile contains IR"),
                options: [("outputDir".to_owned(), serde_json::json!(output_dir.path()))]
                    .into_iter()
                    .collect(),
            })
            .expect("return a typed generation result");

        assert!(generated.success, "{:?}", generated.diagnostics);
        assert_eq!(generated.artifacts.len(), 1);
        assert!(generated.artifacts[0].content.contains("pub fn hello"));
    }

    #[test]
    fn backend_rejects_decodable_but_unsupported_ir_releases() {
        let (request, _compile_output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn hello() { \"world\" }",
            IR_VERSION,
        );
        let compiled = GleamExtension
            .compile(request)
            .expect("compile valid Gleam document");
        let supported_ir = compiled.ir.expect("successful compile contains IR");

        for unsupported in [serde_json::json!("4.0.1"), serde_json::json!(5)] {
            let mut ir = supported_ir.clone();
            ir["formatVersion"] = unsupported.clone();
            let output_dir = tempfile::tempdir().expect("create backend output directory");

            let generated = GleamExtension
                .generate(GenerateRequest {
                    ir,
                    options: [("outputDir".to_owned(), serde_json::json!(output_dir.path()))]
                        .into_iter()
                        .collect(),
                })
                .expect("return a typed generation result");

            assert!(!generated.success, "unsupported release {unsupported}");
            assert!(generated.artifacts.is_empty());
            assert!(
                generated.diagnostics[0]
                    .message
                    .contains("unsupported Morphir IR formatVersion"),
                "{}",
                generated.diagnostics[0].message
            );
        }
    }

    #[test]
    fn malformed_document_returns_a_failed_result_at_its_uri() {
        let uri = "untitled:broken%20module.gleam";
        let (request, _output_dir) = compile_request(uri, "pub fn broken(", IR_VERSION);

        let result = GleamExtension
            .compile(request)
            .expect("return typed compile failure");

        assert!(!result.success);
        assert!(result.ir_version.is_none());
        assert!(result.ir.is_none());
        assert!(result.modules.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics[0]
                .location
                .as_ref()
                .map(|location| location.uri.as_str()),
            Some(uri)
        );
    }

    #[test]
    fn empty_document_list_returns_an_empty_v4_ir_collection() {
        let (mut request, _output_dir) = compile_request("file:///unused.gleam", "", IR_VERSION);
        request.documents.clear();

        let result = GleamExtension
            .compile(request)
            .expect("compile empty document list");

        assert!(result.success);
        assert_eq!(result.ir_version.as_deref(), Some(IR_VERSION));
        assert!(library(&result).def.modules.is_empty());
        assert!(result.modules.is_empty());
    }

    #[test]
    fn empty_compile_skips_fatal_parse_stage_emission_and_filesystem_work() {
        let (mut request, root) = compile_request("file:///unused.gleam", "", IR_VERSION);
        let output_dir = root.path().join("not-created");
        request.documents.clear();
        request.options.extra.insert(
            "outputDir".into(),
            serde_json::json!(output_dir.to_string_lossy()),
        );
        request
            .options
            .extra
            .insert("emitParseStage".into(), serde_json::json!(true));
        request
            .options
            .extra
            .insert("emitParseStageFatal".into(), serde_json::json!(true));

        let result = GleamExtension
            .compile(request)
            .expect("compile empty request without parse-stage I/O");

        assert!(result.success);
        assert!(result.diagnostics.is_empty());
        assert!(!output_dir.exists());
    }

    #[test]
    fn multi_document_compile_returns_one_typed_distribution_with_distinct_nested_modules() {
        let (mut request, _output_dir) = compile_request("file:///unused.gleam", "", IR_VERSION);
        request.documents = vec![
            document(
                "file:///workspace/src/alpha/main.gleam",
                "pub fn alpha() { 1 }",
            ),
            document(
                "file:///workspace/src/beta/main.gleam",
                "pub fn beta() { 2 }",
            ),
        ];

        let result = GleamExtension.compile(request).expect("compile modules");
        let distribution = library(&result);

        assert!(result.success);
        assert_eq!(result.modules, ["alpha/main", "beta/main"]);
        assert_eq!(
            distribution.def.modules.keys().cloned().collect::<Vec<_>>(),
            result.modules
        );
    }

    #[test]
    fn source_root_uri_preserves_nested_module_paths() {
        let (mut request, _output_dir) = compile_request("file:///unused.gleam", "", IR_VERSION);
        request.documents = vec![document(
            "file:///workspace/lib/domain/customer.gleam?rev=1#selection",
            "",
        )];
        request.options.extra.insert(
            "sourceRootUri".into(),
            serde_json::json!("file:///workspace/lib"),
        );

        let result = GleamExtension
            .compile(request)
            .expect("compile nested module");

        assert!(result.success);
        assert_eq!(result.modules, ["domain/customer"]);
    }

    #[test]
    fn source_root_rejects_documents_from_a_different_url_origin() {
        for (source_root, document_uri) in [
            (
                "file://host-a/workspace/src",
                "file://host-b/workspace/src/main.gleam",
            ),
            (
                "https://host-a.test/workspace/src",
                "custom://host-a.test/workspace/src/main.gleam",
            ),
            (
                "https://host-a.test/workspace/src",
                "https://host-b.test/workspace/src/main.gleam",
            ),
        ] {
            let (mut request, output_dir) = compile_request(document_uri, "", IR_VERSION);
            request
                .options
                .extra
                .insert("sourceRootUri".into(), serde_json::json!(source_root));

            let result = GleamExtension
                .compile(request)
                .expect("reject different URL origin");

            assert_typed_failure(&result);
            assert_directory_empty(&output_dir);
        }
    }

    #[test]
    fn source_root_rejects_documents_from_a_different_windows_drive() {
        for (source_root, document_uri) in [
            (r"C:\workspace\src", r"D:\workspace\src\main.gleam"),
            (
                "file:///C:/workspace/src",
                "file:///D:/workspace/src/main.gleam",
            ),
            (r"C:\workspace\src", "file:///D:/workspace/src/main.gleam"),
            ("file:///C:/workspace/src", r"D:\workspace\src\main.gleam"),
        ] {
            let (mut request, output_dir) = compile_request(document_uri, "", IR_VERSION);
            request
                .options
                .extra
                .insert("sourceRootUri".into(), serde_json::json!(source_root));

            let result = GleamExtension
                .compile(request)
                .expect("reject different Windows drive");

            assert_typed_failure(&result);
            assert_directory_empty(&output_dir);
        }
    }

    #[test]
    fn source_root_accepts_documents_from_the_same_origin_and_drive() {
        for (source_root, document_uri) in [
            (
                "file://host-a/workspace/src",
                "file://host-a/workspace/src/domain/main.gleam",
            ),
            (
                "https://host-a.test/workspace/src",
                "https://host-a.test/workspace/src/domain/main.gleam",
            ),
            (r"C:\workspace\src", r"c:\workspace\src\domain\main.gleam"),
            (
                "file:///C:/workspace/src",
                "file:///c:/workspace/src/domain/main.gleam",
            ),
            (
                r"C:\workspace\src",
                "file:///c:/workspace/src/domain/main.gleam",
            ),
            (
                "file:///C:/workspace/src",
                r"c:\workspace\src\domain\main.gleam",
            ),
        ] {
            let (mut request, _output_dir) = compile_request(document_uri, "", IR_VERSION);
            request
                .options
                .extra
                .insert("sourceRootUri".into(), serde_json::json!(source_root));

            let result = GleamExtension
                .compile(request)
                .expect("compile document within source root");

            assert!(
                result.success,
                "{source_root} should contain {document_uri}"
            );
            assert_eq!(result.modules, ["domain/main"]);
        }
    }

    #[test]
    fn uri_and_path_forms_derive_safe_module_names() {
        let (mut request, _output_dir) = compile_request("file:///unused.gleam", "", IR_VERSION);
        request.documents = vec![
            document("file:///workspace/src/file.gleam?rev=1#part", ""),
            document("https://example.test/project/src/remote.gleam#part", ""),
            document(r"C:\workspace\src\windows.gleam", ""),
        ];

        let result = GleamExtension.compile(request).expect("compile URI forms");

        assert!(result.success);
        assert_eq!(result.modules, ["file", "remote", "windows"]);
    }

    #[test]
    fn snake_case_module_paths_are_canonicalized_to_morphir_names() {
        let (request, _output_dir) = compile_request(
            "file:///workspace/src/order_processing/customer_records.gleam",
            "",
            IR_VERSION,
        );

        let result = GleamExtension
            .compile(request)
            .expect("compile snake_case Gleam module");

        assert!(result.success);
        assert_eq!(result.modules, ["order-processing/customer-records"]);
        assert_eq!(
            library(&result).def.modules.keys().collect::<Vec<_>>(),
            ["order-processing/customer-records"]
        );
    }

    #[test]
    fn hyphenated_gleam_source_module_segments_are_rejected() {
        let (request, output_dir) = compile_request(
            "file:///workspace/src/order-processing.gleam",
            "",
            IR_VERSION,
        );

        let result = GleamExtension
            .compile(request)
            .expect("reject hyphenated Gleam module");

        assert_typed_failure(&result);
        assert_eq!(
            result.diagnostics[0].code.as_deref(),
            Some("INVALID_MODULE_URI")
        );
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn module_collisions_after_name_canonicalization_are_rejected() {
        let (mut request, output_dir) = compile_request("file:///unused.gleam", "", IR_VERSION);
        request.documents = vec![
            document("file:///workspace/src/order_processing.gleam", ""),
            document("file:///workspace/src/order__processing.gleam", ""),
        ];

        let result = GleamExtension
            .compile(request)
            .expect("reject canonical module collision");

        assert_typed_failure(&result);
        assert_eq!(
            result.diagnostics[0].code.as_deref(),
            Some("DUPLICATE_MODULE")
        );
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn percent_decoded_invalid_module_name_is_diagnostic() {
        let (request, output_dir) =
            compile_request("file:///workspace/src/bad%20module.gleam", "", IR_VERSION);

        let result = GleamExtension.compile(request).expect("reject module name");

        assert_typed_failure(&result);
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn dot_segments_in_document_uris_are_rejected() {
        let (request, output_dir) = compile_request(
            "file:///workspace/src/domain/%2e%2e/escape.gleam",
            "",
            IR_VERSION,
        );

        let result = GleamExtension.compile(request).expect("reject dot segment");

        assert_typed_failure(&result);
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn duplicate_derived_module_names_fail_atomically() {
        let (mut request, output_dir) = compile_request("file:///unused.gleam", "", IR_VERSION);
        request.documents = vec![
            document("file:///one/src/main.gleam", "pub fn one() { 1 }"),
            document("file:///two/src/main.gleam", "pub fn two() { 2 }"),
        ];

        let result = GleamExtension
            .compile(request)
            .expect("reject duplicate module");

        assert_typed_failure(&result);
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn mixed_valid_and_invalid_documents_fail_without_partial_output() {
        let (mut request, output_dir) = compile_request("file:///unused.gleam", "", IR_VERSION);
        request.documents = vec![
            document("file:///workspace/src/valid.gleam", "pub fn valid() { 1 }"),
            document("file:///workspace/src/invalid.gleam", "pub fn invalid("),
        ];

        let result = GleamExtension
            .compile(request)
            .expect("return compile failure");

        assert_typed_failure(&result);
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn invalid_packages_fail_before_filesystem_work() {
        for package in ["", "/absolute", r"C:\absolute", "a\\b", "a/../b", "A/pkg"] {
            let (mut request, output_dir) =
                compile_request("file:///workspace/src/main.gleam", "", IR_VERSION);
            request.package.name = package.into();

            let result = GleamExtension.compile(request).expect("reject package");

            assert_typed_failure(&result);
            assert_directory_empty(&output_dir);
        }
    }

    #[test]
    fn unsupported_request_semantics_fail_before_filesystem_work() {
        let (mut wrong_request_language, output_dir) =
            compile_request("file:///workspace/src/main.gleam", "", IR_VERSION);
        wrong_request_language.language_id = "elm".into();
        assert_typed_failure(&GleamExtension.compile(wrong_request_language).unwrap());
        assert_directory_empty(&output_dir);

        let (mut wrong_document_language, output_dir) =
            compile_request("file:///workspace/src/main.gleam", "", IR_VERSION);
        wrong_document_language.documents[0].language_id = "elm".into();
        assert_typed_failure(&GleamExtension.compile(wrong_document_language).unwrap());
        assert_directory_empty(&output_dir);

        let (mut types_only, output_dir) =
            compile_request("file:///workspace/src/main.gleam", "", IR_VERSION);
        types_only.options.types_only = true;
        assert_typed_failure(&GleamExtension.compile(types_only).unwrap());
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn exposed_modules_control_access_and_missing_exposures_are_diagnostic() {
        let (mut request, _output_dir) =
            compile_request("file:///workspace/src/public.gleam", "", IR_VERSION);
        request
            .documents
            .push(document("file:///workspace/src/private.gleam", ""));
        request.package.exposed_modules = vec!["public".into()];

        let result = GleamExtension.compile(request).expect("compile exposures");
        let distribution = library(&result);

        assert_eq!(
            distribution.def.modules["public"].access,
            MorphirAccess::Public
        );
        assert_eq!(
            distribution.def.modules["private"].access,
            MorphirAccess::Private
        );

        let (mut missing, output_dir) =
            compile_request("file:///workspace/src/main.gleam", "", IR_VERSION);
        missing.package.exposed_modules = vec!["missing".into()];
        let result = GleamExtension
            .compile(missing)
            .expect("reject missing exposure");
        assert_typed_failure(&result);
        assert_directory_empty(&output_dir);
    }

    #[test]
    fn exposed_modules_use_gleam_source_name_canonicalization() {
        let (mut request, _output_dir) = compile_request(
            "file:///workspace/src/order_processing/customer_records.gleam",
            "",
            IR_VERSION,
        );
        request.package.exposed_modules = vec!["order_processing/customer_records".into()];

        let result = GleamExtension
            .compile(request)
            .expect("compile snake_case exposure");

        assert!(result.success);
        assert_eq!(
            library(&result).def.modules["order-processing/customer-records"].access,
            MorphirAccess::Public
        );
    }

    #[test]
    fn explicit_parse_stage_output_stays_beneath_output_directory() {
        let (mut request, root) = compile_request(
            "file:///workspace/src/domain/customer.gleam",
            "",
            IR_VERSION,
        );
        let output_dir = root.path().join("out/compile/gleam");
        request.options.extra.insert(
            "outputDir".into(),
            serde_json::json!(output_dir.to_string_lossy()),
        );
        request
            .options
            .extra
            .insert("emitParseStage".into(), serde_json::json!(true));

        let result = GleamExtension.compile(request).expect("emit parse stage");

        assert!(result.success);
        assert!(output_dir.join("parse/domain/customer.json").is_file());
        assert!(!root.path().join("out/parse").exists());
    }

    #[test]
    fn parse_stage_output_is_emitted_by_default() {
        let (mut request, output_dir) =
            compile_request("file:///workspace/src/main.gleam", "", IR_VERSION);
        request.options.extra.remove("emitParseStage");

        let result = GleamExtension
            .compile(request)
            .expect("compile with default parse-stage behavior");

        assert!(result.success);
        assert!(output_dir.path().join("parse/main.json").is_file());
    }

    #[test]
    fn committed_parse_stage_cleanup_failure_is_nonfatal_even_in_fatal_mode() {
        let outcome = frontend::parse_stage::EmitParseStageOutcome::Committed {
            cleanup_warning: Some("cleanup failed after commit".into()),
        };

        let (diagnostic, fatal) =
            parse_stage_diagnostic(outcome, true).expect("surface cleanup warning");

        assert!(!fatal);
        assert_eq!(diagnostic.severity, DiagnosticSeverity::Warning);
        assert_eq!(
            diagnostic.code.as_deref(),
            Some("PARSE_STAGE_CLEANUP_WARNING")
        );
        assert!(diagnostic.location.is_none());
    }

    #[test]
    fn unsupported_parse_stage_emission_honors_fatal_option() {
        let outcome = || frontend::parse_stage::EmitParseStageOutcome::RolledBack {
            failure: frontend::parse_stage::EmitFailure {
                message: "Parse-stage filesystem emission is unsupported on wasm32".into(),
                uri: None,
            },
        };

        let (warning, warning_is_fatal) =
            parse_stage_diagnostic(outcome(), false).expect("surface nonfatal warning");
        let (error, error_is_fatal) =
            parse_stage_diagnostic(outcome(), true).expect("surface fatal error");

        assert_eq!(warning.severity, DiagnosticSeverity::Warning);
        assert!(!warning_is_fatal);
        assert_eq!(error.severity, DiagnosticSeverity::Error);
        assert!(error_is_fatal);
    }

    #[test]
    fn recovery_required_diagnostic_has_path_context_without_source_location() {
        let outcome = frontend::parse_stage::EmitParseStageOutcome::RecoveryRequired {
            failure: frontend::parse_stage::EmitFailure {
                message: "rollback failed".into(),
                uri: None,
            },
            recovery_path: PathBuf::from("/output/.morphir-parse-stage-recovery"),
        };

        let (diagnostic, fatal) =
            parse_stage_diagnostic(outcome, true).expect("surface recovery diagnostic");

        assert!(fatal);
        assert!(diagnostic.location.is_none());
        assert!(diagnostic.message.contains(".morphir-parse-stage-recovery"));
    }

    #[test]
    fn repeated_compile_replaces_an_existing_regular_parse_stage_file() {
        let (mut first_request, output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn original() { 1 }",
            IR_VERSION,
        );
        first_request
            .options
            .extra
            .insert("emitParseStage".into(), serde_json::json!(true));
        first_request
            .options
            .extra
            .insert("emitParseStageFatal".into(), serde_json::json!(true));
        let mut second_request = first_request.clone();
        second_request.documents[0].text = "pub fn updated() { 2 }".into();
        std::fs::create_dir(output_dir.path().join("parse")).unwrap();
        std::fs::write(output_dir.path().join("parse/unrelated.json"), "unrelated").unwrap();
        let output_file = output_dir.path().join("parse/main.json");

        let first_result = GleamExtension
            .compile(first_request)
            .expect("emit initial parse stage");
        assert!(first_result.success);
        let first_module: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&output_file).expect("read initial parse stage"))
                .expect("decode initial parse stage");
        assert_eq!(first_module["values"][0]["name"], "original");

        let second_result = GleamExtension
            .compile(second_request)
            .expect("replace parse stage");
        assert!(second_result.success);
        let second_module: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&output_file).expect("read replaced parse stage"),
        )
        .expect("decode replaced parse stage");
        assert_eq!(second_module["values"][0]["name"], "updated");
        assert_eq!(
            std::fs::read_to_string(output_dir.path().join("parse/unrelated.json")).unwrap(),
            "unrelated"
        );
    }

    #[cfg(unix)]
    #[test]
    fn successful_parse_stage_replacement_preserves_restrictive_mode() {
        use std::os::unix::fs::PermissionsExt;

        let (mut request, output_dir) = compile_request(
            "file:///workspace/src/main.gleam",
            "pub fn updated() { 2 }",
            IR_VERSION,
        );
        request
            .options
            .extra
            .insert("emitParseStage".into(), serde_json::json!(true));
        request
            .options
            .extra
            .insert("emitParseStageFatal".into(), serde_json::json!(true));
        let parse_dir = output_dir.path().join("parse");
        let output_file = parse_dir.join("main.json");
        std::fs::create_dir(&parse_dir).expect("create parse directory");
        std::fs::write(&output_file, "original").expect("write original parse stage");
        std::fs::set_permissions(&output_file, std::fs::Permissions::from_mode(0o600))
            .expect("set restrictive mode");

        let result = GleamExtension
            .compile(request)
            .expect("replace parse stage atomically");

        assert!(result.success);
        assert_eq!(
            std::fs::metadata(&output_file)
                .expect("read replacement metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let module: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&output_file).expect("read replacement parse stage"),
        )
        .expect("decode replacement parse stage");
        assert_eq!(module["values"][0]["name"], "updated");
    }

    #[test]
    fn fatal_multi_document_parse_stage_failure_leaves_prior_outputs_unchanged() {
        let (mut request, output_dir) = compile_request("file:///unused.gleam", "", IR_VERSION);
        request.documents = vec![
            document(
                "file:///workspace/src/first.gleam",
                "pub fn changed() { 1 }",
            ),
            document("file:///workspace/src/second.gleam", ""),
        ];
        request
            .options
            .extra
            .insert("emitParseStage".into(), serde_json::json!(true));
        request
            .options
            .extra
            .insert("emitParseStageFatal".into(), serde_json::json!(true));
        let parse_dir = output_dir.path().join("parse");
        std::fs::create_dir_all(&parse_dir).unwrap();
        std::fs::write(parse_dir.join("first.json"), "original-first").unwrap();
        std::fs::create_dir(parse_dir.join("second.json")).unwrap();

        let result = GleamExtension
            .compile(request)
            .expect("return fatal parse-stage failure");

        assert_typed_failure(&result);
        assert_eq!(
            result.diagnostics[0]
                .location
                .as_ref()
                .map(|location| location.uri.as_str()),
            Some("file:///workspace/src/second.gleam")
        );
        assert_eq!(
            std::fs::read_to_string(parse_dir.join("first.json")).unwrap(),
            "original-first"
        );
        assert!(parse_dir.join("second.json").is_dir());
        assert_eq!(
            std::fs::read_dir(output_dir.path())
                .unwrap()
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name())
                .collect::<Vec<_>>(),
            [std::ffi::OsString::from("parse")]
        );
    }

    #[cfg(unix)]
    #[test]
    fn fatal_second_commit_failure_rolls_back_the_first_parse_stage_output() {
        use std::os::unix::fs::PermissionsExt;

        let (mut request, output_dir) = compile_request("file:///unused.gleam", "", IR_VERSION);
        request.documents = vec![
            document(
                "file:///workspace/src/first.gleam",
                "pub fn changed() { 1 }",
            ),
            document("file:///workspace/src/locked/second.gleam", ""),
        ];
        request
            .options
            .extra
            .insert("emitParseStage".into(), serde_json::json!(true));
        request
            .options
            .extra
            .insert("emitParseStageFatal".into(), serde_json::json!(true));
        let parse_dir = output_dir.path().join("parse");
        let locked_dir = parse_dir.join("locked");
        std::fs::create_dir_all(&locked_dir).unwrap();
        let first_output = parse_dir.join("first.json");
        std::fs::write(&first_output, "original-first").unwrap();
        std::fs::set_permissions(&first_output, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o555)).unwrap();

        let result = GleamExtension
            .compile(request)
            .expect("return fatal parse-stage failure");
        std::fs::set_permissions(&locked_dir, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_typed_failure(&result);
        assert_eq!(
            std::fs::read_to_string(&first_output).unwrap(),
            "original-first"
        );
        assert_eq!(
            std::fs::metadata(&first_output)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(!locked_dir.join("second.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn parse_stage_output_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let (mut request, root) =
            compile_request("file:///workspace/src/main.gleam", "", IR_VERSION);
        let output_dir = root.path().join("output");
        let outside_dir = root.path().join("outside");
        std::fs::create_dir_all(&output_dir).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();
        symlink(&outside_dir, output_dir.join("parse")).unwrap();
        request.options.extra.insert(
            "outputDir".into(),
            serde_json::json!(output_dir.to_string_lossy()),
        );
        request
            .options
            .extra
            .insert("emitParseStage".into(), serde_json::json!(true));
        request
            .options
            .extra
            .insert("emitParseStageFatal".into(), serde_json::json!(true));

        let result = GleamExtension
            .compile(request)
            .expect("reject symlink escape");

        assert_typed_failure(&result);
        assert!(!outside_dir.join("main.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn parse_stage_output_rejects_a_nested_directory_symlink() {
        use std::os::unix::fs::symlink;

        let (mut request, root) = compile_request(
            "file:///workspace/src/domain/customer.gleam",
            "",
            IR_VERSION,
        );
        let output_dir = root.path().join("output");
        let parse_dir = output_dir.join("parse");
        let outside_dir = root.path().join("outside");
        std::fs::create_dir_all(&parse_dir).unwrap();
        std::fs::create_dir_all(&outside_dir).unwrap();
        symlink(&outside_dir, parse_dir.join("domain")).unwrap();
        request.options.extra.insert(
            "outputDir".into(),
            serde_json::json!(output_dir.to_string_lossy()),
        );
        request
            .options
            .extra
            .insert("emitParseStage".into(), serde_json::json!(true));
        request
            .options
            .extra
            .insert("emitParseStageFatal".into(), serde_json::json!(true));

        let result = GleamExtension
            .compile(request)
            .expect("reject nested symlink escape");

        assert_typed_failure(&result);
        assert!(!outside_dir.join("customer.json").exists());
        assert!(
            std::fs::read_dir(&output_dir)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .all(|entry| !entry.file_name().to_string_lossy().starts_with(".morphir"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn parse_stage_output_rejects_a_leaf_symlink_without_overwriting_its_target() {
        use std::os::unix::fs::symlink;

        let (mut request, root) =
            compile_request("file:///workspace/src/main.gleam", "", IR_VERSION);
        let output_dir = root.path().join("output");
        let parse_dir = output_dir.join("parse");
        let outside_file = root.path().join("outside.json");
        std::fs::create_dir_all(&parse_dir).unwrap();
        std::fs::write(&outside_file, "attacker-controlled").unwrap();
        symlink(&outside_file, parse_dir.join("main.json")).unwrap();
        request.options.extra.insert(
            "outputDir".into(),
            serde_json::json!(output_dir.to_string_lossy()),
        );
        request
            .options
            .extra
            .insert("emitParseStage".into(), serde_json::json!(true));
        request
            .options
            .extra
            .insert("emitParseStageFatal".into(), serde_json::json!(true));

        let result = GleamExtension
            .compile(request)
            .expect("reject leaf symlink");

        assert_typed_failure(&result);
        assert_eq!(
            std::fs::read_to_string(&outside_file).unwrap(),
            "attacker-controlled"
        );
    }

    #[test]
    fn unsupported_ir_version_returns_a_typed_failure() {
        let (request, _output_dir) = compile_request("file:///main.gleam", "", "3");

        let result = GleamExtension
            .compile(request)
            .expect("return unsupported-version compile failure");

        assert!(!result.success);
        assert!(result.ir_version.is_none());
        assert!(result.ir.is_none());
        assert!(result.modules.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].severity, DiagnosticSeverity::Error);
        assert_eq!(
            result.diagnostics[0].code.as_deref(),
            Some("UNSUPPORTED_IR_VERSION")
        );
    }
}
