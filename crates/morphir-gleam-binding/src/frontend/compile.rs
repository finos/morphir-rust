//! Dependency-ordered compilation with caller-owned incremental state.
use super::{ast::ModuleIR, resolver};
use crate::vfs::OsVfs;
use crate::{error_diagnostic, failed_compile, incremental, version::IrVersion};
use indexmap::IndexMap;
use morphir_core::ir::v4::{
    Access, AccessControlled, Distribution, IRFile, LibraryContent, ModuleDefinition,
    PackageDefinition,
};
use morphir_extension_sdk::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

struct Document {
    source: crate::PreparedDocument,
    parsed: Option<ModuleIR>,
    diagnostics: Vec<Diagnostic>,
}

pub(crate) fn compile(mut request: CompileRequest) -> Result<CompileResult> {
    let version = match IrVersion::parse(&request.options.ir_version) {
        Ok(version) => version,
        Err(message) => {
            return Ok(failed_compile(vec![error_diagnostic(
                "UNSUPPORTED_IR_VERSION",
                message,
                None,
            )]));
        }
    };
    request.options.ir_version = version.release().into();
    if let Some(error) = crate::validate_request_semantics(&request) {
        return Ok(failed_compile(vec![error]));
    }
    let package = match crate::validate_package_name(&request.package.name) {
        Ok(package) => package,
        Err(message) => {
            return Ok(failed_compile(vec![error_diagnostic(
                "INVALID_PACKAGE_NAME",
                message,
                None,
            )]));
        }
    };
    let dependencies =
        match super::dependencies::package_specifications(&request.dependencies, version.release())
        {
            Ok(dependencies) => dependencies,
            Err(errors) => {
                return Ok(failed_compile(
                    errors
                        .into_iter()
                        .map(|e| error_diagnostic(e.code, e.message, None))
                        .collect(),
                ));
            }
        };
    let root = request
        .options
        .extra
        .get("sourceRootUri")
        .or_else(|| request.options.extra.get("sourceRoot"))
        .and_then(|v| v.as_str());
    let prepared = match crate::prepare_documents(&request.documents, root) {
        Ok(documents) => documents,
        Err(errors) => return Ok(failed_compile(errors)),
    };
    let exposed = match crate::validate_exposed_modules(
        request.package.exposed_modules.as_deref(),
        prepared.iter().map(|d| d.module_key.as_str()),
    ) {
        Ok(exposed) => exposed,
        Err(errors) => return Ok(failed_compile(errors)),
    };
    let context = incremental::context_digest(&request, &dependencies);
    let baseline = incremental::read_baseline(&request, &context, |value| {
        serde_json::from_value(value.clone()).map_err(|e| e.to_string())
    });
    let mut diagnostics = baseline.diagnostics;
    let documents: Vec<Document> = prepared
        .into_iter()
        .map(|source| {
            match super::parse_gleam(&format!("{}.gleam", source.module_key), &source.text) {
                Ok(mut parsed) => {
                    parsed.name = source.module_key.clone();
                    Document {
                        source,
                        parsed: Some(parsed),
                        diagnostics: vec![],
                    }
                }
                Err(error) => {
                    let diagnostic = error.to_diagnostic(&source.uri, &source.text);
                    Document {
                        source,
                        parsed: None,
                        diagnostics: vec![diagnostic],
                    }
                }
            }
        })
        .collect();
    let names: HashSet<_> = documents
        .iter()
        .map(|d| d.source.module_key.clone())
        .collect();
    let imports: HashMap<String, Vec<String>> = documents
        .iter()
        .map(|d| {
            let depends_on = d
                .parsed
                .as_ref()
                .map(|m| {
                    m.imports
                        .iter()
                        .map(|i| canonical_module(&i.module))
                        .filter(|name| names.contains(name))
                        .collect()
                })
                .unwrap_or_default();
            (d.source.module_key.clone(), depends_on)
        })
        .collect();
    let (order, cyclic) = dependency_order(&documents, &imports);
    let mut changed = baseline.dropped;
    changed.extend(
        baseline
            .modules
            .keys()
            .filter(|name| !names.contains(*name))
            .cloned(),
    );
    let mut available: IndexMap<String, AccessControlled<ModuleDefinition>> = IndexMap::new();
    let mut modules = IndexMap::new();
    let mut results = Vec::new();
    let output_dir = request
        .options
        .extra
        .get("outputDir")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    for index in order {
        let document = &documents[index];
        let source = &document.source;
        let name = &source.module_key;
        let source_digest = incremental::source_digest(&source.text);
        let depends_on = imports[name].clone();
        let mut result = ModuleResult {
            name: name.clone(),
            uri: source.uri.clone(),
            status: ModuleStatus::Failed,
            source_digest: Some(source_digest.clone()),
            interface_digest: None,
            depends_on,
            ir: None,
            diagnostics: document.diagnostics.clone(),
        };
        if cyclic.contains(name) {
            result.diagnostics.push(error_diagnostic(
                "GLEAM_IMPORT_CYCLE",
                format!("Module '{name}' participates in an import cycle"),
                Some(&source.uri),
            ));
        } else if let Some(parsed) = &document.parsed {
            let blocked: Vec<_> = result
                .depends_on
                .iter()
                .filter(|dep| !available.contains_key(*dep))
                .cloned()
                .collect();
            if !blocked.is_empty() {
                result.status = ModuleStatus::Blocked;
                result.diagnostics.push(error_diagnostic(
                    "GLEAM_BLOCKED",
                    format!(
                        "Module '{name}' is blocked by failed dependencies: {}",
                        blocked.join(", ")
                    ),
                    Some(&source.uri),
                ));
            } else {
                let mut stored_imports = baseline
                    .modules
                    .get(name)
                    .map(|entry| entry.depends_on.clone())
                    .unwrap_or_default();
                stored_imports.sort();
                stored_imports.dedup();
                let mut current_imports = result.depends_on.clone();
                current_imports.sort();
                current_imports.dedup();
                let decision = if stored_imports == current_imports {
                    incremental::decide(name, &source_digest, &baseline.modules, &changed)
                } else {
                    incremental::Decision::Compile
                };
                match decision {
                    incremental::Decision::Reuse(entry) => {
                        let definition = baseline.definitions[name].clone();
                        result.status = ModuleStatus::Unchanged;
                        result.interface_digest = Some(entry.interface_digest);
                        modules.insert(name.clone(), definition.clone());
                        available.insert(name.clone(), definition);
                    }
                    incremental::Decision::Compile => {
                        match resolver::resolve_one(&package, parsed, &available, &dependencies) {
                            Err(errors) => result
                                .diagnostics
                                .extend(errors.into_iter().map(|e| located_resolution(e, source))),
                            Ok(mut resolved) => {
                                let cycles = resolver::validate_alias_cycles(std::slice::from_ref(
                                    &resolved,
                                ));
                                result.diagnostics.extend(
                                    cycles.into_iter().map(|e| located_resolution(e, source)),
                                );
                                if request.options.types_only || version == IrVersion::V3 {
                                    for value in &resolved.values {
                                        let mut diagnostic = error_diagnostic(
                                            "GLEAM_VALUE_SKIPPED",
                                            format!(
                                                "Value '{}' omitted from type-only compilation{}",
                                                value.name,
                                                if version == IrVersion::V3 {
                                                    "; use IR v4 to retain Gleam function bodies"
                                                } else {
                                                    ""
                                                }
                                            ),
                                            Some(&source.uri),
                                        );
                                        diagnostic.severity = DiagnosticSeverity::Warning;
                                        result.diagnostics.push(diagnostic);
                                    }
                                    resolved.values.clear();
                                }
                                if !result
                                    .diagnostics
                                    .iter()
                                    .any(|d| d.severity == DiagnosticSeverity::Error)
                                {
                                    let visitor = super::GleamToMorphirVisitor::new(
                                        OsVfs,
                                        output_dir.clone(),
                                        package.clone(),
                                        source.module_name.clone(),
                                    );
                                    match visitor.build_module_definition(
                                        &resolved,
                                        if exposed.contains(name) {
                                            Access::Public
                                        } else {
                                            Access::Private
                                        },
                                    ) {
                                        Ok(definition) => {
                                            let digest = incremental::interface_digest(&definition);
                                            if baseline.modules.get(name).is_none_or(|prior| {
                                                prior.interface_digest != digest
                                            }) {
                                                changed.insert(name.clone());
                                            }
                                            result.status = ModuleStatus::Compiled;
                                            result.interface_digest = Some(digest);
                                            result.ir = Some(serde_json::to_value(&definition)?);
                                            available.insert(name.clone(), definition.clone());
                                            modules.insert(name.clone(), definition);
                                        }
                                        Err(error) => {
                                            let diagnostic = match error.get_ref().and_then(|error| error.downcast_ref::<super::visitor::UnsupportedValue>()) {
                                                Some(unsupported) => located_resolution(resolver::ResolutionError {
                                                    code: "GLEAM_UNSUPPORTED_VALUE",
                                                    module: name.clone(),
                                                    span: unsupported.span,
                                                    message: error.to_string(),
                                                }, source),
                                                None => error_diagnostic("IR_CONVERSION_ERROR", error.to_string(), Some(&source.uri)),
                                            };
                                            result.diagnostics.push(diagnostic);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if matches!(result.status, ModuleStatus::Failed | ModuleStatus::Blocked) {
            if let Some(definition) = baseline.definitions.get(name) {
                available.insert(name.clone(), definition.clone());
            } else {
                changed.insert(name.clone());
            }
        }
        diagnostics.extend(result.diagnostics.iter().cloned());
        results.push(result);
    }
    // Stable ordering is independent of the topological walk and baseline reuse.
    modules.sort_keys();
    results.sort_by(|a, b| a.name.cmp(&b.name));
    let success = !diagnostics
        .iter()
        .any(|d| d.severity == DiagnosticSeverity::Error);
    if success
        && request
            .options
            .extra
            .get("emitParseStage")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    {
        let parsed = documents
            .iter()
            .filter_map(|d| {
                d.parsed
                    .as_ref()
                    .map(|m| super::parse_stage::ParseStageModule {
                        module_name: &d.source.module_key,
                        uri: &d.source.uri,
                        module: m,
                    })
            })
            .collect::<Vec<_>>();
        let outcome = super::parse_stage::emit_parse_stage(&output_dir, &parsed);
        if let Some((diagnostic, fatal)) = crate::parse_stage_diagnostic(
            outcome,
            request
                .options
                .extra
                .get("emitParseStageFatal")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        ) {
            if fatal {
                return Ok(failed_compile(vec![diagnostic]));
            }
            diagnostics.push(diagnostic);
        }
    }
    let module_names = modules.keys().cloned().collect();
    let ir = IRFile {
        format_version: Default::default(),
        distribution: Distribution::Library(LibraryContent {
            package_name: package,
            dependencies,
            def: PackageDefinition { modules },
        }),
    };
    let encoded = match version.encode(&ir) {
        Ok(encoded) => encoded,
        Err(error) => {
            return Ok(failed_compile(vec![error_diagnostic(
                "GLEAM_IR", error, None,
            )]));
        }
    };
    Ok(CompileResult {
        success,
        ir_version: Some(version.release().into()),
        ir: success.then_some(encoded),
        diagnostics,
        modules: module_names,
        module_results: results,
        context_digest: Some(context),
    })
}

fn canonical_module(value: &str) -> String {
    value
        .split('/')
        .map(|part| morphir_core::naming::Name::from(part).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

fn located_resolution(
    error: resolver::ResolutionError,
    source: &crate::PreparedDocument,
) -> Diagnostic {
    let mut diagnostic = error_diagnostic(error.code, error.message, Some(&source.uri));
    let position = |offset: usize| {
        let offset = offset.min(source.text.len());
        let prefix = &source.text[..source.text.floor_char_boundary(offset)];
        SourcePosition {
            line: prefix.bytes().filter(|b| *b == b'\n').count() as u32,
            character: prefix
                .rsplit('\n')
                .next()
                .unwrap_or("")
                .encode_utf16()
                .count() as u32,
        }
    };
    if let Some(location) = &mut diagnostic.location {
        location.range = SourceRange {
            start: position(error.span.start),
            end: position(error.span.end),
        };
    }
    diagnostic
}

fn dependency_order(
    documents: &[Document],
    imports: &HashMap<String, Vec<String>>,
) -> (Vec<usize>, HashSet<String>) {
    #[derive(Default)]
    struct Traversal {
        stack: Vec<usize>,
        done: HashSet<usize>,
        cyclic: HashSet<String>,
        order: Vec<usize>,
    }
    fn visit(
        index: usize,
        documents: &[Document],
        imports: &HashMap<String, Vec<String>>,
        positions: &HashMap<&str, usize>,
        traversal: &mut Traversal,
    ) {
        if traversal.done.contains(&index) {
            return;
        }
        if let Some(start) = traversal.stack.iter().position(|i| *i == index) {
            traversal.cyclic.extend(
                traversal.stack[start..]
                    .iter()
                    .map(|i| documents[*i].source.module_key.clone()),
            );
            return;
        }
        traversal.stack.push(index);
        for dependency in &imports[&documents[index].source.module_key] {
            if let Some(next) = positions.get(dependency.as_str()) {
                visit(*next, documents, imports, positions, traversal);
            }
        }
        traversal.stack.pop();
        if traversal.done.insert(index) {
            traversal.order.push(index);
        }
    }
    let positions = documents
        .iter()
        .enumerate()
        .map(|(i, d)| (d.source.module_key.as_str(), i))
        .collect();
    let mut traversal = Traversal::default();
    for index in 0..documents.len() {
        visit(index, documents, imports, &positions, &mut traversal);
    }
    (traversal.order, traversal.cyclic)
}
