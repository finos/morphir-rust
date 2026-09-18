//! Parse the supported Rust type subset into the shared typed IR.
mod binding_attributes;
mod bindings;
mod boundary;
mod declarations;
mod patterns;
mod recursion;
mod source;
mod types;
mod values;

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
        items: &file.items,
    };
    let definitions = file
        .items
        .iter()
        .filter(|item| !matches!(item, syn::Item::Fn(_)))
        .map(|item| declarations::lower(&context, item))
        .collect::<Outcome<Vec<_>>>()?;
    let definitions = recursion::nominalize(&context, &file.items, definitions)?;
    declarations::validate_aliases(&context, &file.items, &definitions)?;
    let pattern_context = if request.options.types_only {
        crate::patterns::Context::default()
    } else {
        patterns::context(&context, &definitions)?
    };

    let functions = if request.options.types_only {
        Default::default()
    } else {
        values::collect(&context)?
    };
    let shared = values::shared(&functions, pattern_context)?;
    let mut values = Vec::new();
    let mut expressions = Vec::new();
    if !request.options.types_only {
        source.unique(file.items.iter().flat_map(|item| match item {
            syn::Item::Fn(f) => vec![f.sig.ident.clone()],
            syn::Item::Struct(s) => vec![s.ident.clone()],
            syn::Item::Enum(e) => e.variants.iter().map(|v| v.ident.clone()).collect(),
            _ => vec![],
        }))?;
    }
    let mut diagnostics = settings.diagnostics;
    for item in &file.items {
        if let syn::Item::Fn(function) = item {
            let binding = bindings::lower(&context, function)?;
            if !request.options.types_only
                && let Some(binding) = binding
            {
                if settings.version != 4 {
                    return Err(source.error(
                        function.sig.ident.span(),
                        "RS_BINDING_VERSION",
                        "Native and external binding declarations require Morphir IR v4",
                    ));
                }
                values.push(binding);
                continue;
            }
            let mut diagnostic = source.error(
                syn::spanned::Spanned::span(function),
                "RS_VALUES_UNSUPPORTED",
                "Type-only compilation omitted a Rust function",
            );
            if !request.options.types_only {
                expressions.push(values::lower(&context, function, &functions, &shared)?);
                continue;
            }
            diagnostic.severity = morphir_extension_sdk::DiagnosticSeverity::Warning;
            diagnostics.push(diagnostic);
        }
    }
    let mut checked = std::collections::BTreeMap::new();
    for (name, definition) in &expressions {
        let name = FQName::new(
            settings.package.clone(),
            settings.module.clone(),
            name.clone(),
        );
        let mut migration = morphir_core::migration::MigrationContext::default();
        let key = morphir_core::migration::migrate_fqname(&name, &migration.cursor)
            .map_err(|e| error("RS_MIGRATION", format!("{e:?}")))?
            .to_canonical_string();
        let value = morphir_core::migration::migrate_value_definition(
            &definition.value.value,
            &mut migration,
        )
        .map_err(|e| error("RS_MIGRATION", format!("{e:?}")))?;
        checked.insert(key, value);
    }
    crate::functions::check_cycles(&checked)
        .map_err(|e| source.error(proc_macro2::Span::call_site(), "RS_VALUE_RECURSION", e))?;
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
                            values: expressions,
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
        let mut migrated = migrate_distribution(&classic, MigrationOptions::default())
            .map_err(|e| error("RS_MIGRATION", format!("{e:?}")))?;
        let morphir_core::ir::v4::Distribution::Library(library) = &mut migrated.value.distribution
        else {
            unreachable!("Classic library migration produces a v4 library");
        };
        let (_, module) = library
            .def
            .modules
            .get_index_mut(0)
            .expect("one source module was migrated");
        module.value.values.extend(values);
        serde_json::to_value(migrated.value)
    }
    .map_err(|e| error("RS_SERIALIZATION", e.to_string()))?;
    Ok(CompileResult {
        success: true,
        ir_version: Some(settings.version.to_string()),
        ir: Some(ir),
        diagnostics,
        modules: vec![settings.module_name],
        module_results: vec![],
        context_digest: None,
    })
}
