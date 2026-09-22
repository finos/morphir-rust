use crate::{Outcome, error};
use morphir_core::{
    format_version::{NormalizedFormatVersion, ScalarValue, SupportTable},
    ir::classic::{Access, Name, Path},
};
use morphir_extension_sdk::{CompileRequest, Diagnostic, DiagnosticSeverity};

pub(super) struct Settings {
    pub version: u32,
    pub package: Path,
    pub module: Path,
    pub module_name: String,
    pub access: Access,
    pub diagnostics: Vec<Diagnostic>,
}

pub(super) fn validate(request: &CompileRequest) -> Outcome<Settings> {
    if request.language_id != "rust"
        || request
            .sources
            .documents
            .iter()
            .any(|d| d.language_id != "rust")
    {
        return Err(error(
            "RS_LANGUAGE",
            "The Rust frontend requires languageId rust",
        ));
    }
    if request.sources.documents.len() != 1 {
        return Err(error(
            "RS_DOCUMENTS",
            "Exactly one Rust document is supported",
        ));
    }
    if !request.dependencies.is_empty() {
        return Err(error(
            "RS_DEPENDENCIES",
            "External dependencies are not supported",
        ));
    }
    let scalar = match request.options.ir_version.as_str() {
        "3" => ScalarValue::Integer(3),
        "4" => ScalarValue::Integer(4),
        other => ScalarValue::String(other.into()),
    };
    let support = SupportTable::reference();
    let normalized = NormalizedFormatVersion::from_scalar(&scalar, &support)
        .map_err(|e| error(e.code(), e.message()))?;
    if let Some(e) = support.unsupported_diagnostic(&normalized.release, normalized.compatibility) {
        return Err(error(e.code(), e.message()));
    }
    let canonical = morphir_core::naming::Path::from_canonical_string(&request.package.name)
        .map_err(|e| error("RS_PACKAGE", e))?;
    if canonical.is_empty() {
        return Err(error("RS_PACKAGE", "Package name must not be empty"));
    }
    let package = Path::new(
        canonical
            .segments
            .iter()
            .map(|n| Name::new(n.words()))
            .collect(),
    );
    let uri = &request.sources.documents[0].uri;
    let stem = uri
        .rsplit('/')
        .next()
        .and_then(|s| s.strip_suffix(".rs"))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            error(
                "RS_DOCUMENT_URI",
                "The document URI must end in a named .rs file",
            )
        })?;
    let module_ident = syn::parse_str::<syn::Ident>(stem).map_err(|_| {
        error(
            "RS_DOCUMENT_URI",
            "The document file stem must be a Rust identifier",
        )
    })?;
    if !stem.is_ascii() || stem.chars().all(|c| c == '_') {
        return Err(error(
            "RS_DOCUMENT_URI",
            "The document file stem must contain ASCII letters or digits",
        ));
    }
    let module_name = morphir_core::naming::Name::from(&module_ident.to_string()).to_title_case();
    let module = Path::new(vec![stem.parse().expect("infallible classic name")]);
    if request
        .package
        .exposed_modules
        .iter()
        .flatten()
        .any(|name| name != &module_name)
        || request
            .package
            .exposed_modules
            .as_ref()
            .is_some_and(|modules| modules.len() > 1)
    {
        return Err(error(
            "RS_EXPOSED_MODULES",
            format!("The only available module is {module_name}"),
        ));
    }
    for (key, value) in &request.options.extra {
        let valid = match key.as_str() {
            "outputDir" => value.is_string(),
            "emitParseStage" | "emitParseStageFatal" => value.is_boolean(),
            _ => {
                return Err(error(
                    "RS_OPTION",
                    format!("Unknown Rust compile option {key}"),
                ));
            }
        };
        if !valid {
            return Err(error("RS_OPTION", format!("Invalid value type for {key}")));
        }
    }
    let mut diagnostics = vec![];
    if request
        .options
        .extra
        .get("emitParseStage")
        .and_then(|v| v.as_bool())
        == Some(true)
    {
        let mut diagnostic = error(
            "RS_PARSE_STAGE_UNAVAILABLE",
            "The Rust frontend cannot emit a parse-stage artifact through compile",
        );
        if request
            .options
            .extra
            .get("emitParseStageFatal")
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            return Err(diagnostic);
        }
        diagnostic.severity = DiagnosticSeverity::Warning;
        diagnostics.push(diagnostic);
    }
    Ok(Settings {
        version: normalized.release.major(),
        package,
        module,
        module_name,
        access: if request
            .package
            .exposed_modules
            .as_ref()
            .is_some_and(Vec::is_empty)
        {
            Access::Private
        } else {
            Access::Public
        },
        diagnostics,
    })
}
