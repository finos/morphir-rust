//! Parse the supported Rust type subset into the shared typed IR.
mod boundary;
mod declarations;
mod recursion;
mod source;
mod types;

use crate::{Outcome, error};
use morphir_core::{
    ir::classic::*,
    migration::{MigrationOptions, migrate_distribution},
};
use morphir_extension_sdk::{CompileRequest, CompileResult};
use source::Source;

pub(crate) fn compile(request: &CompileRequest) -> Outcome<CompileResult> {
    let settings = boundary::validate(request)?;
    let source = Source(&request.documents[0]);
    let file = syn::parse_file(&source.0.text)
        .map_err(|e| source.error(e.span(), "RS_SYNTAX", e.to_string()))?;
    let doc = source.attributes(&file.attrs, false)?;
    let symbols = declarations::symbols(&source, &file.items)?;
    let context = types::Context {
        source: &source,
        symbols: &symbols,
        package: &settings.package,
        module: &settings.module,
    };
    let mut definitions = Vec::new();
    let mut diagnostics = settings.diagnostics;
    for item in &file.items {
        if let syn::Item::Fn(function) = item {
            source.attributes(&function.attrs, false)?;
            let mut diagnostic = source.error(
                syn::spanned::Spanned::span(function),
                "RS_VALUES_UNSUPPORTED",
                "Rust executable values are not supported",
            );
            if !request.options.types_only {
                return Err(diagnostic);
            }
            diagnostic.severity = morphir_extension_sdk::DiagnosticSeverity::Warning;
            diagnostics.push(diagnostic);
        } else {
            definitions.push(declarations::lower(&context, item)?);
        }
    }
    let definitions = recursion::nominalize(&context, &file.items, definitions)?;
    declarations::validate_aliases(&context, &file.items, &definitions)?;
    let classic = Distribution {
        format_version: 3,
        distribution: DistributionBody::Library(
            settings.package,
            vec![],
            PackageDefinition {
                modules: vec![ModuleEntry {
                    path: settings.module,
                    definition: AccessControlled {
                        access: settings.access,
                        value: ModuleDefinition {
                            types: definitions,
                            values: vec![],
                            doc: (!doc.is_empty()).then_some(doc),
                        },
                    },
                }],
            },
        ),
    };
    let ir = if settings.version == 3 {
        serde_json::to_value(classic)
    } else {
        let migrated = migrate_distribution(&classic, MigrationOptions::default())
            .map_err(|e| error("RS_MIGRATION", format!("{e:?}")))?;
        serde_json::to_value(migrated.value)
    }
    .map_err(|e| error("RS_SERIALIZATION", e.to_string()))?;
    Ok(CompileResult {
        success: true,
        ir_version: Some(settings.version.to_string()),
        ir: Some(ir),
        diagnostics,
        modules: vec![settings.module_name],
    })
}
