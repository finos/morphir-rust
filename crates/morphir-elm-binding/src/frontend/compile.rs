//! The frontend, assembled: a compile request in, a Morphir IR distribution
//! and one result per module out.
//!
//! The compiler is stateless. Everything it remembers between runs arrives in
//! [`morphir_extension_sdk::CompileBaseline`] and leaves in
//! [`morphir_extension_sdk::CompileResult::module_results`], so two processes
//! compiling the same request with the same baseline produce the same answer.
//!
//! Modules are walked in dependency order, which is what makes a single pass
//! enough: by the time a module is resolved, every in-package module it can
//! name has already been compiled, reused, or found to be beyond reach.
//!
//! A module that cannot be compiled does not stop the ones that do not depend
//! on it. A dependent of a failed module resolves against that module's last
//! good interface when the baseline has one, and is reported as `blocked` when
//! it does not.

use std::collections::{HashMap, HashSet};

use crate::ast;
use crate::digest::sha256_hex;
use crate::frontend::boundary::{self, Validated};
use crate::frontend::dependencies;
use crate::frontend::emit::{self, ModuleIr, PackageInput};
use crate::frontend::resolve::{DependencyInterface, Scope, dependency_order, resolve};
use crate::frontend::source;
use crate::frontend::{cst_to_ast, parse};
use crate::incremental::{Decision, Digests, decide};
use crate::resolved::{Interface, ResolvedModule};
use crate::span::Span;
use morphir_extension_sdk::{
    BaselineModule, CompileRequest, CompileResult, Diagnostic, DiagnosticSeverity, ModuleResult,
    ModuleStatus,
};

const SYNTAX: &str = "ELM_SYNTAX";
const VALUE_SKIPPED: &str = "ELM_VALUE_SKIPPED";
const IMPORT_CYCLE: &str = "ELM_IMPORT_CYCLE";
const BLOCKED: &str = "ELM_BLOCKED";
const IR: &str = "ELM_IR";

/// One source document that yielded a module name.
struct Document {
    uri: String,
    text: String,
    source_digest: String,
    module: ast::Module,
    /// Syntax errors found in this document, already located.
    syntax: Vec<Diagnostic>,
}

impl Document {
    fn name(&self) -> &[String] {
        &self.module.name
    }

    fn dotted(&self) -> String {
        self.module.name.join(".")
    }

    /// The in-package modules this document imports, with the span of the
    /// import that named them, first mention first.
    fn imports_within(&self, package: &HashSet<String>) -> Vec<(Vec<String>, Span)> {
        let mut seen: HashSet<Vec<String>> = HashSet::new();
        self.module
            .imports
            .iter()
            .filter(|import| package.contains(&import.module.join(".")))
            .filter(|import| import.module != self.module.name)
            .filter(|import| seen.insert(import.module.clone()))
            .map(|import| (import.module.clone(), import.span))
            .collect()
    }
}

/// What the walk has learned so far. Everything a later module needs to see is
/// in here, and nothing outlives the call.
struct Run {
    /// Interfaces a module may resolve in-package names against: compiled and
    /// reused modules, plus the last good interface of a failed one.
    interfaces: HashMap<Vec<String>, Interface>,
    /// Dotted names whose public interface differs from the baseline this run.
    changed: HashSet<String>,
    /// Dotted names of modules that could not be compiled this run.
    stopped: HashSet<String>,
    results: Vec<ModuleResult>,
    /// The per-module IR the distribution is assembled from, in module order.
    module_irs: Vec<ModuleIr>,
    /// The modules actually resolved this run, for the emitter's package input.
    compiled: Vec<ResolvedModule>,
}

/// Compiles one request. This never panics: every `expect` below is on a value
/// this function produced itself.
pub fn compile(request: CompileRequest) -> CompileResult {
    let validated = match boundary::validate(&request) {
        Ok(validated) => validated,
        Err(diagnostic) => return rejected(diagnostic),
    };
    let Some(emitter) = emit::emitter_for(&validated.ir_version) else {
        return rejected(boundary::request_error(format!(
            "no emitter for Morphir IR `{}`",
            validated.ir_version
        )));
    };

    let (dependency_interfaces, mut diagnostics) =
        dependencies::from_request(&request.dependencies);
    let (documents, skipped_documents) = read_documents(&request);
    diagnostics.extend(skipped_documents.iter().cloned());

    let (baseline, baseline_interfaces, baseline_diagnostics) = read_baseline(&request, &validated);
    diagnostics.extend(baseline_diagnostics);

    let package: HashSet<String> = documents.iter().map(Document::dotted).collect();
    // A module the baseline knew and the request no longer contains was
    // deleted: its dependents can no longer resolve against it, so they are
    // treated exactly as if its interface had changed.
    let mut run = Run {
        interfaces: HashMap::new(),
        changed: baseline
            .keys()
            .filter(|name| !package.contains(*name))
            .cloned()
            .collect(),
        stopped: HashSet::new(),
        results: Vec::new(),
        module_irs: Vec::new(),
        compiled: Vec::new(),
    };

    let (order, cycle) = ordered(&documents, &package);

    for index in order {
        let document = &documents[index];
        compile_one(
            &mut run,
            document,
            &validated,
            emitter.as_ref(),
            &package,
            &baseline,
            &baseline_interfaces,
            &dependency_interfaces,
        );
    }

    for index in cycle {
        let document = &documents[index];
        let diagnostic = source::diagnostic(
            &document.uri,
            &document.text,
            document.module.span,
            DiagnosticSeverity::Error,
            IMPORT_CYCLE,
            format!(
                "module `{}` is part of an import cycle, or imports a module that is",
                document.dotted()
            ),
        );
        stop(
            &mut run,
            document,
            &baseline_interfaces,
            ModuleStatus::Failed,
            vec![diagnostic],
            &package,
        );
    }

    let input = PackageInput {
        package: &validated.package,
        modules: &run.compiled,
        // A classic distribution carries its dependencies' specifications
        // inline, and a compile request supplies definitions rather than
        // specifications, so none are written here.
        dependencies: &[],
    };
    let ir = match emitter.emit_distribution(&input, &run.module_irs) {
        Ok(ir) => Some(ir),
        Err(reason) => {
            diagnostics.push(boundary::request_error(format!(
                "the Morphir IR {} distribution could not be assembled: {reason}",
                validated.ir_version
            )));
            None
        }
    };

    for result in &run.results {
        diagnostics.extend(result.diagnostics.iter().cloned());
    }

    let modules = run
        .results
        .iter()
        .filter(|result| is_written(result.status))
        .map(|result| result.name.clone())
        .collect();
    let success = ir.is_some()
        && skipped_documents.is_empty()
        && run.results.iter().all(|result| is_written(result.status));

    CompileResult {
        success,
        ir_version: Some(validated.ir_version),
        ir,
        diagnostics,
        modules,
        module_results: run.results,
    }
}

fn is_written(status: ModuleStatus) -> bool {
    matches!(status, ModuleStatus::Compiled | ModuleStatus::Unchanged)
}

fn rejected(diagnostic: Diagnostic) -> CompileResult {
    CompileResult {
        success: false,
        ir_version: None,
        ir: None,
        diagnostics: vec![diagnostic],
        modules: Vec::new(),
        module_results: Vec::new(),
    }
}

/// Parses every document. A document whose module name cannot be read has no
/// module to report a result for, so it is dropped with its diagnostics.
fn read_documents(request: &CompileRequest) -> (Vec<Document>, Vec<Diagnostic>) {
    let mut documents: Vec<Document> = Vec::with_capacity(request.documents.len());
    let mut skipped = Vec::new();

    for source_document in &request.documents {
        let uri = source_document.uri.as_str();
        let text = source_document.text.as_str();
        let parsed = parse::parse(text);
        let syntax: Vec<Diagnostic> = parse::syntax_errors(&parsed, text)
            .into_iter()
            .map(|error| {
                source::diagnostic(
                    uri,
                    text,
                    error.span,
                    DiagnosticSeverity::Error,
                    SYNTAX,
                    error.message,
                )
            })
            .collect();

        let module = match cst_to_ast::to_ast(&parsed, text) {
            Ok(module) => module,
            Err(error) => {
                skipped.push(source::diagnostic(
                    uri,
                    text,
                    error.span,
                    DiagnosticSeverity::Error,
                    SYNTAX,
                    error.message,
                ));
                skipped.extend(syntax);
                continue;
            }
        };

        let dotted = module.name.join(".");
        if documents.iter().any(|document| document.dotted() == dotted) {
            skipped.push(source::diagnostic(
                uri,
                text,
                module.span,
                DiagnosticSeverity::Error,
                SYNTAX,
                format!("module `{dotted}` is declared by more than one document"),
            ));
            continue;
        }

        documents.push(Document {
            uri: uri.to_string(),
            text: text.to_string(),
            source_digest: sha256_hex(text.as_bytes()),
            module,
            syntax,
        });
    }

    (documents, skipped)
}

/// The baseline, keyed by dotted module name, with the public interface each
/// entry's IR describes. An entry whose IR this version cannot read is dropped
/// entirely — reusing IR whose interface is unknown would let a dependent
/// resolve against nothing — and reported as a warning.
#[allow(clippy::type_complexity)]
fn read_baseline(
    request: &CompileRequest,
    validated: &Validated,
) -> (
    HashMap<String, BaselineModule>,
    HashMap<String, Interface>,
    Vec<Diagnostic>,
) {
    let mut baseline = HashMap::new();
    let mut interfaces = HashMap::new();
    let mut diagnostics = Vec::new();

    let Some(supplied) = &request.baseline else {
        return (baseline, interfaces, diagnostics);
    };

    for entry in &supplied.modules {
        let name: Vec<String> = entry.name.split('.').map(str::to_string).collect();
        match dependencies::interface_from_module_ir(&validated.ir_version, &name, &entry.ir) {
            Ok(interface) => {
                interfaces.insert(entry.name.clone(), interface);
                baseline.insert(entry.name.clone(), entry.clone());
            }
            Err(reason) => diagnostics.push(boundary::request_warning(format!(
                "baseline for module {} ignored: {reason}",
                entry.name
            ))),
        }
    }

    (baseline, interfaces, diagnostics)
}

/// Document indices in dependency order, and the indices of the modules that
/// could not be ordered because they are in an import cycle or downstream of
/// one. Ties keep document order, because [`dependency_order`] is stable.
fn ordered(documents: &[Document], package: &HashSet<String>) -> (Vec<usize>, Vec<usize>) {
    let mut nodes: Vec<usize> = (0..documents.len()).collect();
    let mut cycle: Vec<usize> = Vec::new();

    loop {
        let entries: Vec<(Vec<String>, Vec<Vec<String>>)> = nodes
            .iter()
            .map(|&index| {
                (
                    documents[index].name().to_vec(),
                    documents[index]
                        .imports_within(package)
                        .into_iter()
                        .map(|(name, _)| name)
                        .collect(),
                )
            })
            .collect();

        match dependency_order(&entries) {
            Ok(order) => {
                return (order.into_iter().map(|slot| nodes[slot]).collect(), cycle);
            }
            Err(unordered) => {
                let unordered: HashSet<Vec<String>> = unordered.into_iter().collect();
                let before = nodes.len();
                nodes.retain(|&index| {
                    let tangled = unordered.contains(documents[index].name());
                    if tangled {
                        cycle.push(index);
                    }
                    !tangled
                });
                if nodes.len() == before {
                    // Nothing was removed, so nothing would change on a second
                    // pass; the rest is reported as tangled rather than looped on.
                    cycle.extend(nodes);
                    cycle.sort_unstable();
                    return (Vec::new(), cycle);
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn compile_one(
    run: &mut Run,
    document: &Document,
    validated: &Validated,
    emitter: &dyn emit::Emitter,
    package: &HashSet<String>,
    baseline: &HashMap<String, BaselineModule>,
    baseline_interfaces: &HashMap<String, Interface>,
    dependency_interfaces: &[DependencyInterface],
) {
    let dotted = document.dotted();
    let access = boundary::module_access(validated.exposed.as_deref(), &dotted);

    if !document.syntax.is_empty() {
        stop(
            run,
            document,
            baseline_interfaces,
            ModuleStatus::Failed,
            document.syntax.clone(),
            package,
        );
        return;
    }

    // A dependency that failed with no interface to fall back on cannot be
    // resolved against, so this module is reported as blocked rather than as
    // a pile of name errors it did not cause.
    let imports = document.imports_within(package);
    if let Some((blocker, span)) = imports.iter().find(|(name, _)| {
        run.stopped.contains(&name.join(".")) && !run.interfaces.contains_key(name)
    }) {
        let diagnostic = source::diagnostic(
            &document.uri,
            &document.text,
            *span,
            DiagnosticSeverity::Error,
            BLOCKED,
            format!(
                "module `{dotted}` cannot be compiled: module `{}` failed to compile and has no \
                 previous interface to resolve against",
                blocker.join(".")
            ),
        );
        stop(
            run,
            document,
            baseline_interfaces,
            ModuleStatus::Blocked,
            vec![diagnostic],
            package,
        );
        return;
    }

    if let Decision::Reuse(entry) = decide(
        document.name(),
        &document.source_digest,
        baseline,
        &run.changed,
    ) {
        if let Some(interface) = baseline_interfaces.get(&dotted) {
            run.interfaces
                .insert(document.name().to_vec(), interface.clone());
        }
        run.module_irs
            .push((document.name().to_vec(), access, entry.ir.clone()));
        run.results.push(ModuleResult {
            name: dotted,
            uri: document.uri.clone(),
            status: ModuleStatus::Unchanged,
            source_digest: Some(document.source_digest.clone()),
            interface_digest: Some(entry.interface_digest.clone()),
            depends_on: entry.depends_on.clone(),
            ir: None,
            diagnostics: Vec::new(),
        });
        return;
    }

    let resolved = {
        let known = &run.interfaces;
        let package_modules = |path: &[String]| known.get(path).cloned();
        let scope = Scope {
            package: &validated.package,
            prelude: &validated.prelude,
            package_modules: &package_modules,
            dependencies: dependency_interfaces,
        };
        resolve(&document.module, access, &scope)
    };

    let resolved = match resolved {
        Ok(resolved) => resolved,
        Err(errors) => {
            let diagnostics = errors
                .into_iter()
                .map(|error| {
                    source::diagnostic(
                        &document.uri,
                        &document.text,
                        error.span,
                        DiagnosticSeverity::Error,
                        error.code,
                        error.message,
                    )
                })
                .collect();
            stop(
                run,
                document,
                baseline_interfaces,
                ModuleStatus::Failed,
                diagnostics,
                package,
            );
            return;
        }
    };

    let ir = match emitter.emit_module(&resolved) {
        Ok(ir) => ir,
        Err(reason) => {
            let diagnostic = source::diagnostic(
                &document.uri,
                &document.text,
                document.module.span,
                DiagnosticSeverity::Error,
                IR,
                reason,
            );
            stop(
                run,
                document,
                baseline_interfaces,
                ModuleStatus::Failed,
                vec![diagnostic],
                package,
            );
            return;
        }
    };

    let digests = Digests {
        source: document.source_digest.clone(),
        interface: resolved.interface_digest(),
    };
    let interface_changed = baseline
        .get(&dotted)
        .is_none_or(|entry| entry.interface_digest != digests.interface);
    if interface_changed {
        run.changed.insert(dotted.clone());
    }

    let diagnostics = resolved
        .skipped_values
        .iter()
        .map(|(name, span)| {
            source::diagnostic(
                &document.uri,
                &document.text,
                *span,
                DiagnosticSeverity::Warning,
                VALUE_SKIPPED,
                format!("value `{name}` is skipped: this frontend compiles types only"),
            )
        })
        .collect();

    run.interfaces
        .insert(document.name().to_vec(), resolved.interface());
    run.module_irs
        .push((document.name().to_vec(), access, ir.clone()));
    run.results.push(ModuleResult {
        name: dotted,
        uri: document.uri.clone(),
        status: ModuleStatus::Compiled,
        source_digest: Some(digests.source),
        interface_digest: Some(digests.interface),
        depends_on: resolved
            .depends_on
            .iter()
            .map(|name| name.join("."))
            .collect(),
        ir: Some(ir),
        diagnostics,
    });
    run.compiled.push(resolved);
}

/// Records a module that produced no IR this run.
///
/// Its last good interface, when the baseline has one, stays available so that
/// its dependents still resolve; without one, its dependents are told the
/// interface is gone, which is what turns them into blocked modules.
fn stop(
    run: &mut Run,
    document: &Document,
    baseline_interfaces: &HashMap<String, Interface>,
    status: ModuleStatus,
    diagnostics: Vec<Diagnostic>,
    package: &HashSet<String>,
) {
    let dotted = document.dotted();
    run.stopped.insert(dotted.clone());
    match baseline_interfaces.get(&dotted) {
        Some(interface) => {
            run.interfaces
                .insert(document.name().to_vec(), interface.clone());
        }
        None => {
            run.changed.insert(dotted.clone());
        }
    }
    run.results.push(ModuleResult {
        name: dotted,
        uri: document.uri.clone(),
        status,
        source_digest: Some(document.source_digest.clone()),
        interface_digest: None,
        depends_on: document
            .imports_within(package)
            .into_iter()
            .map(|(name, _)| name.join("."))
            .collect(),
        ir: None,
        diagnostics,
    });
}
