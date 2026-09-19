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
use crate::names;
use crate::resolved::{Access, FqName, Interface, InterfaceType, RType, ResolvedModule};
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
        Err(diagnostic) => return rejected(diagnostic, None),
    };

    let (dependency_interfaces, mut diagnostics) =
        dependencies::from_request(&request.dependencies);

    // The request itself is sound, so the context it compiles under is known
    // and is reported whatever happens next: it is the value the host stores
    // and echoes back, and a run that failed still tells it what it was.
    let context_digest = boundary::context_digest(&validated, &dependency_interfaces);

    let Some(emitter) = emit::emitter_for(&validated.ir_version) else {
        return rejected(
            boundary::request_error(format!(
                "no emitter for Morphir IR `{}`",
                validated.ir_version
            )),
            Some(context_digest),
        );
    };

    let read = read_baseline(&request, &validated.ir_version, &context_digest);
    let (baseline, baseline_interfaces) = (read.modules, read.interfaces);
    diagnostics.extend(read.diagnostics);

    let (documents, skipped_documents, headerless) =
        read_documents(&request, &baseline, validated.doc_comments);
    diagnostics.extend(skipped_documents.iter().cloned());

    // A document whose header could not be read still names a module when the
    // baseline recognises its uri, so it is part of the package: its dependents
    // must see a module that failed, not a module that was deleted.
    let package: HashSet<String> = documents
        .iter()
        .map(Document::dotted)
        .chain(headerless.iter().map(|entry| entry.name.clone()))
        .collect();
    // A baseline entry that cannot be reused is, to a dependent, exactly an
    // interface that changed: a module the request no longer contains was
    // deleted, and one whose IR would not decode has no interface to offer. If
    // either were left out of `changed`, a dependent with untouched source
    // would be reused against an interface that is no longer there.
    let mut run = Run {
        interfaces: HashMap::new(),
        changed: baseline
            .keys()
            .filter(|name| !package.contains(*name))
            .cloned()
            .chain(read.dropped)
            .collect(),
        stopped: HashSet::new(),
        results: Vec::new(),
        module_irs: Vec::new(),
        compiled: Vec::new(),
    };

    // Before anything is walked, so that a dependent of a headerless document
    // already sees it as stopped.
    for entry in &headerless {
        stop_headerless(&mut run, entry, &baseline_interfaces);
    }

    // Two Elm module names can print differently and still write the same IR
    // module path once the package path is stripped from each (`Foo` and
    // `My.Foo` under package `My` both become `Foo`: `My.Foo` is the
    // package-qualified spelling of the very `Foo` the bare document already
    // publishes). The first document to claim a path keeps compiling under it;
    // every later one collides, and is reported and failed here — before either
    // is walked — rather than left to fail late in the emitter, where the whole
    // distribution would be lost and both modules would still be reported
    // `Compiled`.
    let duplicate_losers = stop_duplicate_relative_paths(
        &mut run,
        &documents,
        &validated.package,
        &baseline_interfaces,
        &package,
    );

    let (order, cycle) = ordered(&documents, &package);

    for index in order {
        if duplicate_losers.contains(&index) {
            continue;
        }
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

    // Only now, with every module's public interface known, can a module that
    // an exposed module reaches into be found and published too.
    promote_implicitly_exposed(&mut run, &validated.package);

    // A distribution holds its dependencies' *specifications*, and a compile
    // request supplies their definitions, so the specification is derived.
    // `morphir_core` can do that for v4 (`PackageDefinition::to_specification`);
    // the classic package model has no such conversion, so a classic
    // distribution is still written with no dependencies. Once classic grows
    // one, this is the single place that changes.
    let specifications = match validated.ir_version.as_str() {
        "4" => dependencies::v4_specifications(&request.dependencies),
        _ => Vec::new(),
    };
    let input = PackageInput {
        package: &validated.package,
        modules: &run.compiled,
        dependencies: &specifications,
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
    // An Error anywhere — including one about the request rather than about a
    // module, such as a dependency distribution that would not read — means the
    // frontend did not compile what it was asked to, whatever the per-module
    // statuses say.
    let success = ir.is_some()
        && skipped_documents.is_empty()
        && run.results.iter().all(|result| is_written(result.status))
        && !diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error);

    CompileResult {
        success,
        ir_version: Some(validated.ir_version),
        ir,
        diagnostics,
        modules,
        module_results: run.results,
        context_digest: Some(context_digest),
    }
}

fn is_written(status: ModuleStatus) -> bool {
    matches!(status, ModuleStatus::Compiled | ModuleStatus::Unchanged)
}

fn rejected(diagnostic: Diagnostic, context_digest: Option<String>) -> CompileResult {
    CompileResult {
        success: false,
        ir_version: None,
        ir: None,
        diagnostics: vec![diagnostic],
        modules: Vec::new(),
        module_results: Vec::new(),
        context_digest,
    }
}

/// A document whose module header could not be read, but whose uri the baseline
/// recognises, so the module it stands for is known even though its text is not.
struct Headerless {
    /// Dotted module name, from the baseline entry.
    name: String,
    uri: String,
    source_digest: String,
    diagnostics: Vec<Diagnostic>,
}

/// Parses every document.
///
/// A document whose module name cannot be read names no module by itself. When
/// the baseline recognises its uri the name is known anyway, and the document
/// becomes a failed module ([`Headerless`]) so that its dependents take the
/// ordinary failed-dependency path instead of being told the module was
/// deleted. Without a baseline match there is nothing to report a result for,
/// so it is dropped with its diagnostics.
fn read_documents(
    request: &CompileRequest,
    baseline: &HashMap<String, BaselineModule>,
    docs: cst_to_ast::DocComments,
) -> (Vec<Document>, Vec<Diagnostic>, Vec<Headerless>) {
    let mut documents: Vec<Document> = Vec::with_capacity(request.documents.len());
    let mut skipped = Vec::new();
    let mut headerless: Vec<Headerless> = Vec::new();

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

        let module = match cst_to_ast::to_ast(&parsed, text, docs) {
            Ok(module) => module,
            Err(error) => {
                let reported = source::diagnostic(
                    uri,
                    text,
                    error.span,
                    DiagnosticSeverity::Error,
                    SYNTAX,
                    error.message,
                );
                match baseline
                    .values()
                    .find(|entry| entry.uri == uri)
                    .filter(|entry| {
                        !headerless.iter().any(|held| held.name == entry.name)
                            && !documents
                                .iter()
                                .any(|document| document.dotted() == entry.name)
                    }) {
                    Some(entry) => headerless.push(Headerless {
                        name: entry.name.clone(),
                        uri: uri.to_string(),
                        source_digest: sha256_hex(text.as_bytes()),
                        diagnostics: std::iter::once(reported).chain(syntax).collect(),
                    }),
                    None => {
                        skipped.push(reported);
                        skipped.extend(syntax);
                    }
                }
                continue;
            }
        };

        // Nothing is wrong with the document's syntax; the *request* named the
        // same module twice, and only the request can say which one it meant.
        let dotted = module.name.join(".");
        if documents.iter().any(|document| document.dotted() == dotted) {
            skipped.push(source::diagnostic(
                uri,
                text,
                module.span,
                DiagnosticSeverity::Error,
                boundary::REQUEST,
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

    (documents, skipped, headerless)
}

/// The baseline the request supplied, once it has been read.
struct Baseline {
    /// Reusable entries, keyed by dotted module name.
    modules: HashMap<String, BaselineModule>,
    /// The public interface each reusable entry's IR describes.
    interfaces: HashMap<String, Interface>,
    /// Entries that were thrown away, by dotted module name.
    dropped: Vec<String>,
    diagnostics: Vec<Diagnostic>,
}

/// Reads the baseline. An entry whose IR this version cannot read is dropped
/// entirely — reusing IR whose interface is unknown would let a dependent
/// resolve against nothing — reported as a warning, and named in `dropped` so
/// that its dependents are recompiled rather than reused against it.
///
/// The whole baseline is thrown away unless it states the very context this
/// run compiles under, because every entry in it describes resolution against
/// that context and nothing in an entry reveals which one it was.
fn read_baseline(request: &CompileRequest, ir_version: &str, context_digest: &str) -> Baseline {
    let mut baseline = Baseline {
        modules: HashMap::new(),
        interfaces: HashMap::new(),
        dropped: Vec::new(),
        diagnostics: Vec::new(),
    };

    let Some(supplied) = &request.baseline else {
        return baseline;
    };

    // What a module resolved to last time depends on the whole compile context
    // — the IR version, the prelude, and the dependency distributions supplied
    // with the request — so a baseline from a different one, or one that will
    // not say which one it came from, is thrown away whole rather than partly
    // trusted.
    match &supplied.context_digest {
        Some(recorded) if recorded == context_digest => {}
        Some(_) => {
            baseline.diagnostics.push(boundary::request_warning(
                "baseline ignored: it was built under a different compile context",
            ));
            return baseline;
        }
        None => {
            baseline.diagnostics.push(boundary::request_warning(
                "baseline ignored: it carries no contextDigest",
            ));
            return baseline;
        }
    }

    for entry in &supplied.modules {
        let name: Vec<String> = entry.name.split('.').map(str::to_string).collect();
        match dependencies::interface_from_module_ir(ir_version, &name, &entry.ir) {
            Ok(interface) => {
                baseline.interfaces.insert(entry.name.clone(), interface);
                baseline.modules.insert(entry.name.clone(), entry.clone());
            }
            Err(reason) => {
                baseline.dropped.push(entry.name.clone());
                baseline.diagnostics.push(boundary::request_warning(format!(
                    "baseline for module {} ignored: {reason}",
                    entry.name
                )));
            }
        }
    }

    baseline
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
    let access = boundary::module_access(
        validated.exposed.as_deref(),
        &validated.package,
        document.name(),
    );

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
        run.module_irs.push((
            boundary::relative_module(&validated.package, document.name()),
            access,
            entry.ir.clone(),
        ));
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
    run.module_irs.push((
        boundary::relative_module(&validated.package, document.name()),
        access,
        ir.clone(),
    ));
    run.results.push(ModuleResult {
        name: dotted,
        uri: document.uri.clone(),
        status: ModuleStatus::Compiled,
        source_digest: Some(digests.source),
        interface_digest: Some(digests.interface),
        depends_on: depends_on(&resolved, document, package),
        ir: Some(ir),
        diagnostics,
    });
    run.compiled.push(resolved);
}

/// Publishes every module an exposed module reaches into.
///
/// An exposed module whose public surface names a type of an unexposed module
/// would otherwise describe a type nobody outside the package may name.
/// morphir-elm resolves that by publishing the module that owns the type
/// (`Morphir.Elm.IncrementalFrontend`, `collectImplicitlyExposedModules`), and
/// this does the same, from the same starting point: only what a *public* type
/// of an *exposed* module publishes counts — a public alias's body and a public
/// custom type's constructor arguments — which is exactly what a module's
/// [`Interface`] holds.
///
/// It is transitive, as morphir-elm's is: the type that made a module public
/// contributes its own references in turn.
///
/// It diverges from morphir-elm in one place, deliberately. morphir-elm stops
/// at the *module* — `Morphir.Elm.IncrementalFrontend`, lines 1250-1252, drop a
/// reference into an already-published module without following it — so if an
/// exposed module publishes `Hidden.A` and `Hidden.B`, only whichever of them
/// was reached first has its own references followed, and a module the other
/// one names stays private while a public type points into it. That result is
/// internally inconsistent, so this walk follows every *declaration* it
/// reaches, not every module. It only ever publishes more modules than
/// morphir-elm would, never fewer, so nothing that was public becomes private.
///
/// This runs after the walk rather than during it, because a module's access is
/// not knowable until every module that could reach it has been resolved. The
/// access an emitter writes into the distribution is the one in
/// [`Run::module_irs`], not the one the per-module IR was emitted under, so
/// setting it here is what the document ends up saying. A reused module is
/// covered too: its interface came out of the baseline, and the promotion is
/// recomputed from scratch on every run.
fn promote_implicitly_exposed(run: &mut Run, package: &[String]) {
    let spelled = |path: &[String]| -> Vec<String> {
        path.iter()
            .map(|segment| names::type_spelling(segment))
            .collect()
    };
    // Keyed by IR module path, spelled the way an `Interface` spells names, so
    // that the paths in `module_irs` and the ones inside an `FqName` agree.
    let interfaces: HashMap<Vec<String>, &Interface> = run
        .interfaces
        .iter()
        .map(|(name, interface)| {
            (
                spelled(&boundary::relative_module(package, name)),
                interface,
            )
        })
        .collect();
    let exposed: HashSet<Vec<String>> = run
        .module_irs
        .iter()
        .filter(|(_, access, _)| *access == Access::Public)
        .map(|(path, _, _)| spelled(path))
        .collect();

    let package = spelled(package);
    let mut pending: Vec<FqName> = Vec::new();
    for path in &exposed {
        if let Some(interface) = interfaces.get(path) {
            for declared in &interface.types {
                published_references(declared, &mut pending);
            }
        }
    }

    let mut implicit: HashSet<Vec<String>> = HashSet::new();
    // The declarations already followed, by module path and type name. Keying
    // this on the *declaration* and not on its module is what makes the walk
    // complete: a module is published by the first reference that reaches it,
    // but every reference that reaches it still has its own declaration
    // followed, so a second type of the same module opens what *it* names too.
    // It is also what makes the walk terminate, since there are finitely many
    // declarations and none is followed twice.
    let mut followed: HashSet<(Vec<String>, String)> = HashSet::new();
    while let Some(reference) = pending.pop() {
        // A reference out of the package is somebody else's to publish, and an
        // explicitly exposed module's own public declarations are already
        // seeds, so there is nothing to add by following one again.
        if reference.package != package || exposed.contains(&reference.module) {
            continue;
        }
        if !followed.insert((reference.module.clone(), reference.name.clone())) {
            continue;
        }
        implicit.insert(reference.module.clone());
        if let Some(interface) = interfaces.get(&reference.module)
            && let Some(declared) = interface
                .types
                .iter()
                .find(|declared| declared.name == reference.name)
        {
            published_references(declared, &mut pending);
        }
    }

    for (path, access, _) in &mut run.module_irs {
        if implicit.contains(&spelled(path)) {
            *access = Access::Public;
        }
    }
}

/// Every type reference a public declaration publishes: an alias publishes the
/// type it stands for, and a custom type publishes its constructors' argument
/// types — but only when the constructors are public, since an opaque type
/// shows a dependent nothing.
fn published_references(declared: &InterfaceType, out: &mut Vec<FqName>) {
    if let Some(alias) = &declared.alias {
        type_references(alias, out);
    }
    for (_, arguments) in declared.constructors.iter().flatten() {
        for argument in arguments {
            type_references(argument, out);
        }
    }
}

/// Every [`FqName`] a resolved type mentions, however deeply nested.
fn type_references(ty: &RType, out: &mut Vec<FqName>) {
    match ty {
        RType::Var(_) | RType::Unit => {}
        RType::Ref(name, arguments) => {
            out.push(name.clone());
            for argument in arguments {
                type_references(argument, out);
            }
        }
        RType::Record(fields) | RType::ExtensibleRecord(_, fields) => {
            for field in fields {
                type_references(&field.ty, out);
            }
        }
        RType::Tuple(elements) => {
            for element in elements {
                type_references(element, out);
            }
        }
        RType::Function(argument, result) => {
            type_references(argument, out);
            type_references(result, out);
        }
    }
}

/// What a module result reports as its dependencies: every in-package module
/// whose name the resolver actually used, *and* every in-package module the
/// document imports.
///
/// The second half is an over-approximation on purpose. A name that resolves
/// nowhere today can resolve to an imported module tomorrow — adding
/// `type alias T = Int` to a module that is imported `exposing (..)` can make a
/// name that resolved elsewhere ambiguous — and a dependent that recorded only
/// the references it resolved would be reused against a scope that changed
/// underneath it. Widening to the imports is what keeps an incremental run
/// equal to a clean one; the cost is recompiling a module whose unused import
/// changed.
fn depends_on(
    resolved: &ResolvedModule,
    document: &Document,
    package: &HashSet<String>,
) -> Vec<String> {
    let mut names: Vec<String> = resolved
        .depends_on
        .iter()
        .map(|name| name.join("."))
        .chain(
            document
                .imports_within(package)
                .into_iter()
                .map(|(name, _)| name.join(".")),
        )
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Finds every document whose IR module path — the relative path computed by
/// [`boundary::relative_module`] — is already claimed by an earlier document,
/// reports each collision with one `ELM_REQUEST` error naming both Elm module
/// names and both document uris, fails the later document via [`stop`] exactly
/// as any other module that could not be compiled, and returns the indices of
/// every document [`compile`]'s walk must now skip.
///
/// The first document to claim a path is left untouched here — it keeps
/// compiling under that path — so only strictly later documents (in request
/// order) are ever reported or failed.
fn stop_duplicate_relative_paths(
    run: &mut Run,
    documents: &[Document],
    package_path: &[String],
    baseline_interfaces: &HashMap<String, Interface>,
    package: &HashSet<String>,
) -> HashSet<usize> {
    let mut claimed: HashMap<Vec<String>, usize> = HashMap::new();
    let mut losers = HashSet::new();

    for (index, document) in documents.iter().enumerate() {
        let relative = boundary::relative_module(package_path, document.name());
        let Some(&first) = claimed.get(&relative) else {
            claimed.insert(relative, index);
            continue;
        };

        let first_document = &documents[first];
        let diagnostic = source::diagnostic(
            &document.uri,
            &document.text,
            document.module.span,
            DiagnosticSeverity::Error,
            boundary::REQUEST,
            format!(
                "module `{}` ({}) and module `{}` ({}) both write the IR module path `{}`",
                first_document.dotted(),
                first_document.uri,
                document.dotted(),
                document.uri,
                relative.join("."),
            ),
        );
        stop(
            run,
            document,
            baseline_interfaces,
            ModuleStatus::Failed,
            vec![diagnostic],
            package,
        );
        losers.insert(index);
    }

    losers
}

/// Records a document whose module header could not be read but whose module
/// the baseline names: a failed module, so that its dependents resolve against
/// its last good interface when there is one and are blocked when there is not.
fn stop_headerless(
    run: &mut Run,
    entry: &Headerless,
    baseline_interfaces: &HashMap<String, Interface>,
) {
    run.stopped.insert(entry.name.clone());
    match baseline_interfaces.get(&entry.name) {
        Some(interface) => {
            let path: Vec<String> = entry.name.split('.').map(str::to_string).collect();
            run.interfaces.insert(path, interface.clone());
        }
        None => {
            run.changed.insert(entry.name.clone());
        }
    }
    run.results.push(ModuleResult {
        name: entry.name.clone(),
        uri: entry.uri.clone(),
        status: ModuleStatus::Failed,
        source_digest: Some(entry.source_digest.clone()),
        interface_digest: None,
        depends_on: Vec::new(),
        ir: None,
        diagnostics: entry.diagnostics.clone(),
    });
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
        depends_on: {
            let mut names: Vec<String> = document
                .imports_within(package)
                .into_iter()
                .map(|(name, _)| name.join("."))
                .collect();
            names.sort_unstable();
            names
        },
        ir: None,
        diagnostics,
    });
}
