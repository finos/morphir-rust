//! Name resolution: turns the syntactic AST into the version-neutral
//! [`crate::resolved`] model by resolving every type reference to an
//! [`FqName`].
//!
//! A reference is resolved against, in order: the types declared in this
//! module, the visible-name table built from the prelude's implicit imports
//! plus the module's own imports, and then the module the name came from —
//! an in-package module, a dependency module, or a prelude platform module.
//!
//! This module knows nothing about tree-sitter or about Morphir IR types.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::ast;
use crate::frontend::boundary;
use crate::names;
use crate::prelude::Prelude;
use crate::resolved::{
    Access, FqName, Interface, RConstructor, RField, RType, ResolvedBody, ResolvedModule,
    ResolvedType,
};
use crate::span::Span;

/// Everything the resolver needs besides the module itself.
pub struct Scope<'a> {
    /// This package's path, e.g. `["local", "example"]`.
    pub package: &'a [String],
    /// The prelude names are resolved against.
    pub prelude: &'a Prelude,
    /// Looks up a module of *this* package by its module path (the request's
    /// own modules plus the incremental baseline).
    pub package_modules: &'a dyn Fn(&[String]) -> Option<Interface>,
    /// The public interfaces of the packages this one depends on.
    pub dependencies: &'a [DependencyInterface],
}

/// The public interfaces of one dependency package.
pub struct DependencyInterface {
    /// The dependency's package path.
    pub package: Vec<String>,
    /// Module interfaces, named relative to `package`.
    pub modules: Vec<Interface>,
}

/// A resolution failure. `span` is the span of the offending reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveError {
    /// The diagnostic code the failure is reported under.
    pub code: &'static str,
    /// The span of the offending reference.
    pub span: Span,
    /// What the reader has to change.
    pub message: String,
}

const NOT_FOUND: &str = "ELM_RESOLVE_NOT_FOUND";
const AMBIGUOUS: &str = "ELM_RESOLVE_AMBIGUOUS";
const DUPLICATE_TYPE: &str = "ELM_DUPLICATE_TYPE";
const TYPE_CYCLE: &str = "ELM_TYPE_CYCLE";

/// Where a module path was found, and what it declares.
struct ModuleTarget {
    package: Vec<String>,
    module: Vec<String>,
    types: Vec<String>,
    in_package: bool,
}

/// Resolves a module. Every unresolvable reference in the module is reported,
/// not just the first.
pub fn resolve(
    module: &ast::Module,
    access: Access,
    scope: &Scope,
) -> Result<ResolvedModule, Vec<ResolveError>> {
    let mut resolver = Resolver {
        module,
        scope,
        local: module
            .types
            .iter()
            .map(|t| names::type_spelling(t.name()))
            .collect(),
        visible: BTreeMap::new(),
        qualified: BTreeMap::new(),
        depends_on: BTreeSet::new(),
        errors: Vec::new(),
    };
    resolver.report_duplicate_declarations();
    resolver.report_alias_cycles();
    resolver.build_tables();

    let types = module
        .types
        .iter()
        .map(|decl| resolver.resolve_decl(decl))
        .collect::<Vec<_>>();

    if !resolver.errors.is_empty() {
        return Err(resolver.errors);
    }

    Ok(ResolvedModule {
        name: module.name.clone(),
        access,
        doc: module.doc.clone(),
        types,
        depends_on: resolver.depends_on.into_iter().collect(),
        skipped_values: module
            .skipped_values
            .iter()
            .map(|v| (v.name.clone(), v.span))
            .collect(),
    })
}

struct Resolver<'a> {
    module: &'a ast::Module,
    scope: &'a Scope<'a>,
    /// Types declared in this module; they win over every import.
    local: BTreeSet<String>,
    /// Unqualified type name -> the (raw, un-aliased) module paths that expose
    /// it. More than one distinct path means the name is ambiguous.
    visible: BTreeMap<String, Vec<Vec<String>>>,
    /// Import alias or dotted module path -> the raw module path it names.
    qualified: BTreeMap<String, Vec<String>>,
    depends_on: BTreeSet<Vec<String>>,
    errors: Vec<ResolveError>,
}

impl Resolver<'_> {
    /// Reports two type declarations that a Morphir document could not tell
    /// apart.
    ///
    /// A Morphir name keeps only a declaration's words, so `Foo_Bar` and
    /// `FooBar` are one name once written. Writing both would leave one of them
    /// silently replacing the other in the document — and in this module's
    /// interface — so the pair is refused instead, naming both spellings.
    fn report_duplicate_declarations(&mut self) {
        let mut seen: BTreeMap<String, &str> = BTreeMap::new();
        let mut duplicates = Vec::new();
        for declaration in &self.module.types {
            let written = declaration.name();
            if let Some(earlier) = seen.insert(names::type_spelling(written), written) {
                duplicates.push((declaration.span(), earlier, written));
            }
        }
        for (span, earlier, written) in duplicates {
            self.error(
                DUPLICATE_TYPE,
                span,
                format!(
                    "types `{earlier}` and `{written}` are the same name in a Morphir document \
                     (`{}`), so only one of them could be written",
                    names::type_spelling(written)
                ),
            );
        }
    }

    /// Reports a module's own type aliases that stand for each other in a
    /// circle: `type alias T = T`, or `A = B` with `B = A`.
    ///
    /// An alias is not a type of its own — it is the type it stands for,
    /// written out — so a circle of them describes nothing that can ever be
    /// written down, and expanding one would not terminate. A custom type is a
    /// type of its own and may name itself as often as it likes, so it is not
    /// an edge here and a cycle that passes through one is no cycle at all.
    ///
    /// Every declaration on a circle is reported, at its own span, so the
    /// reader is told the whole of what has to be broken rather than one
    /// arbitrary member of it.
    fn report_alias_cycles(&mut self) {
        let aliases: BTreeMap<String, (&str, Span)> = self
            .module
            .types
            .iter()
            .filter_map(|declaration| match declaration {
                ast::TypeDecl::Alias { name, span, .. } => {
                    Some((names::type_spelling(name), (name.as_str(), *span)))
                }
                ast::TypeDecl::Custom { .. } => None,
            })
            .collect();

        let edges: BTreeMap<String, BTreeSet<String>> = self
            .module
            .types
            .iter()
            .filter_map(|declaration| match declaration {
                ast::TypeDecl::Alias { name, body, .. } => {
                    let mut referenced = BTreeSet::new();
                    self.collect_local_aliases(body, &aliases, &mut referenced);
                    Some((names::type_spelling(name), referenced))
                }
                ast::TypeDecl::Custom { .. } => None,
            })
            .collect();

        let mut reported: Vec<(Span, String)> = Vec::new();
        for (spelled, (written, span)) in &aliases {
            if !reaches(spelled, spelled, &edges, &mut BTreeSet::new()) {
                continue;
            }
            reported.push((
                *span,
                format!(
                    "type alias `{written}` stands for itself, directly or through other aliases \
                     in this module; an alias cannot be circular"
                ),
            ));
        }
        for (span, message) in reported {
            self.error(TYPE_CYCLE, span, message);
        }
    }

    /// Every alias of this module a type expression names, however deeply.
    ///
    /// A reference qualified by this module's own path counts, because it names
    /// the same declaration; one qualified by anything else cannot.
    fn collect_local_aliases(
        &self,
        ty: &ast::TypeExpr,
        aliases: &BTreeMap<String, (&str, Span)>,
        found: &mut BTreeSet<String>,
    ) {
        match ty {
            ast::TypeExpr::Var { .. } | ast::TypeExpr::Unit { .. } => {}
            ast::TypeExpr::Ref {
                module, name, args, ..
            } => {
                if module.is_empty() || module == &self.module.name {
                    let spelled = names::type_spelling(name);
                    if aliases.contains_key(&spelled) {
                        found.insert(spelled);
                    }
                }
                for arg in args {
                    self.collect_local_aliases(arg, aliases, found);
                }
            }
            ast::TypeExpr::Record { fields, .. } => {
                for field in fields {
                    self.collect_local_aliases(&field.ty, aliases, found);
                }
            }
            ast::TypeExpr::ExtensibleRecord { fields, .. } => {
                for field in fields {
                    self.collect_local_aliases(&field.ty, aliases, found);
                }
            }
            ast::TypeExpr::Tuple { items, .. } => {
                for item in items {
                    self.collect_local_aliases(item, aliases, found);
                }
            }
            ast::TypeExpr::Function { arg, result, .. } => {
                self.collect_local_aliases(arg, aliases, found);
                self.collect_local_aliases(result, aliases, found);
            }
        }
    }

    /// Builds the visible-name and qualified-name tables from the prelude's
    /// implicit imports followed by the module's own imports.
    fn build_tables(&mut self) {
        let implicit: Vec<(Vec<String>, Vec<String>)> = self
            .scope
            .prelude
            .implicit_import
            .iter()
            .map(|imp| {
                (
                    split_dotted(&imp.module),
                    imp.exposing.iter().map(|e| exposed_name(e)).collect(),
                )
            })
            .collect();

        for (module, exposing) in implicit {
            self.expose_module(&module, None);
            if exposing.iter().any(|name| name == "..") {
                self.expose_all(&module);
            }
            for name in exposing.iter().filter(|name| *name != "..") {
                self.expose_name(name, &module);
            }
        }

        let imports: Vec<ast::Import> = self.module.imports.clone();
        for import in &imports {
            self.expose_module(&import.module, import.alias.as_deref());
            match &import.exposing {
                None => {}
                Some(ast::Exposing::All) => self.expose_all(&import.module),
                Some(ast::Exposing::Explicit(items)) => {
                    for item in items {
                        if let ast::Exposed::Type { name, .. } = item {
                            let name = name.clone();
                            self.expose_name(&name, &import.module);
                        }
                    }
                }
            }
        }
    }

    /// Makes `M.T` (and `A.T` for an alias `A`) refer to `module`.
    fn expose_module(&mut self, module: &[String], alias: Option<&str>) {
        self.qualified.insert(module.join("."), module.to_vec());
        if let Some(alias) = alias {
            self.qualified.insert(alias.to_string(), module.to_vec());
        }
    }

    /// `exposing (..)`: every type the module path declares becomes visible.
    ///
    /// A written path can name more than one module (an in-package module and
    /// a platform module of the same name), so the names of all of them are
    /// exposed; the reference site decides whether that is ambiguous. An
    /// unresolvable import contributes no names; the references that needed
    /// them report themselves.
    fn expose_all(&mut self, module: &[String]) {
        let names: Vec<String> = self
            .lookup_modules(module)
            .into_iter()
            .flat_map(|target| target.types)
            .collect();
        for name in names {
            self.expose_name(&name, module);
        }
    }

    /// Names are filed under their document spelling, because that is the only
    /// spelling a module interface can state (see [`crate::names`]).
    fn expose_name(&mut self, name: &str, module: &[String]) {
        let candidates = self.visible.entry(names::type_spelling(name)).or_default();
        if !candidates.iter().any(|c| c == module) {
            candidates.push(module.to_vec());
        }
    }

    fn resolve_decl(&mut self, decl: &ast::TypeDecl) -> ResolvedType {
        match decl {
            ast::TypeDecl::Alias {
                name,
                params,
                body,
                doc,
                ..
            } => ResolvedType {
                name: name.clone(),
                access: self.type_access(name),
                doc: doc.clone(),
                params: params.clone(),
                body: ResolvedBody::Alias(self.resolve_type(body)),
            },
            ast::TypeDecl::Custom {
                name,
                params,
                constructors,
                doc,
                ..
            } => ResolvedType {
                name: name.clone(),
                access: self.type_access(name),
                doc: doc.clone(),
                params: params.clone(),
                body: ResolvedBody::Custom {
                    constructor_access: self.constructor_access(name),
                    constructors: constructors
                        .iter()
                        .map(|ctor| RConstructor {
                            name: ctor.name.clone(),
                            args: ctor.args.iter().map(|a| self.resolve_type(a)).collect(),
                        })
                        .collect(),
                },
            },
        }
    }

    fn type_access(&self, name: &str) -> Access {
        match &self.module.exposing {
            ast::Exposing::All => Access::Public,
            ast::Exposing::Explicit(items) => {
                if items
                    .iter()
                    .any(|item| matches!(item, ast::Exposed::Type { name: n, .. } if n == name))
                {
                    Access::Public
                } else {
                    Access::Private
                }
            }
        }
    }

    fn constructor_access(&self, name: &str) -> Access {
        match &self.module.exposing {
            ast::Exposing::All => Access::Public,
            ast::Exposing::Explicit(items) => {
                if items.iter().any(|item| {
                    matches!(item, ast::Exposed::Type { name: n, constructors: true } if n == name)
                }) {
                    Access::Public
                } else {
                    Access::Private
                }
            }
        }
    }

    fn resolve_type(&mut self, ty: &ast::TypeExpr) -> RType {
        match ty {
            ast::TypeExpr::Var { name, .. } => RType::Var(name.clone()),
            ast::TypeExpr::Ref {
                module,
                name,
                args,
                span,
            } => {
                let args = args.iter().map(|a| self.resolve_type(a)).collect();
                let fq = self.resolve_ref(module, name, *span);
                RType::Ref(fq, args)
            }
            ast::TypeExpr::Record { fields, .. } => RType::Record(self.resolve_fields(fields)),
            ast::TypeExpr::ExtensibleRecord { base, fields, .. } => {
                RType::ExtensibleRecord(base.clone(), self.resolve_fields(fields))
            }
            ast::TypeExpr::Tuple { items, .. } => {
                RType::Tuple(items.iter().map(|i| self.resolve_type(i)).collect())
            }
            ast::TypeExpr::Function { arg, result, .. } => RType::Function(
                Box::new(self.resolve_type(arg)),
                Box::new(self.resolve_type(result)),
            ),
            ast::TypeExpr::Unit { .. } => RType::Unit,
        }
    }

    fn resolve_fields(&mut self, fields: &[ast::Field]) -> Vec<RField> {
        fields
            .iter()
            .map(|f| RField {
                name: f.name.clone(),
                ty: self.resolve_type(&f.ty),
            })
            .collect()
    }

    /// Resolves one type reference, recording an error (and returning a
    /// placeholder name) when it cannot be resolved, so that the rest of the
    /// module still reports its own errors.
    fn resolve_ref(&mut self, module: &[String], name: &str, span: Span) -> FqName {
        // Every table and every module interface states a name in its document
        // spelling, because that is the only spelling a Morphir document keeps.
        // The reference is spelled the same way before it is looked up, so a
        // declaration written `Foo_Bar` is found by a reference to `Foo_Bar` —
        // and, unavoidably, by one to `FooBar`. Diagnostics keep the spelling
        // the reader wrote.
        let spelled = names::type_spelling(name);
        if module.is_empty() {
            // Rule 1: a locally declared type wins over every import and is
            // never ambiguous.
            if self.local.contains(&spelled) {
                return self.own_name(name);
            }
            let candidates = self.visible.get(&spelled).cloned().unwrap_or_default();
            if candidates.is_empty() {
                self.error(
                    NOT_FOUND,
                    span,
                    format!(
                        "`{name}` is not in scope (prelude: {})",
                        self.scope.prelude.id
                    ),
                );
                return self.unresolved(name);
            }

            // Ambiguity is decided on resolved targets, not on written paths:
            // one written path can name two different modules (an in-package
            // `String` and the platform `Morphir.SDK.String`), and two written
            // paths can name the same module (`List` and `Morphir.SDK.List`).
            let mut targets: Vec<ModuleTarget> = Vec::new();
            for candidate in &candidates {
                for target in self.lookup_modules(candidate) {
                    if !target.types.contains(&spelled) {
                        continue;
                    }
                    if !targets
                        .iter()
                        .any(|seen| seen.package == target.package && seen.module == target.module)
                    {
                        targets.push(target);
                    }
                }
            }

            match targets.len() {
                0 => {
                    let modules = candidates
                        .iter()
                        .map(|c| format!("`{}`", c.join(".")))
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.error(
                        NOT_FOUND,
                        span,
                        format!(
                            "`{name}` not found in module {modules} (prelude: {})",
                            self.scope.prelude.id
                        ),
                    );
                    self.unresolved(name)
                }
                1 => {
                    let target = targets.remove(0);
                    self.use_target(target, name)
                }
                _ => {
                    let modules = targets
                        .iter()
                        .map(|t| format!("`{}`", qualified_module_name(t)))
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.error(
                        AMBIGUOUS,
                        span,
                        format!(
                            "`{name}` is exposed by more than one module: {modules} (prelude: {})",
                            self.scope.prelude.id
                        ),
                    );
                    self.unresolved(name)
                }
            }
        } else {
            // A qualified reference must name an import: either an import's
            // alias or its full module path (the prelude's implicit imports
            // count).
            let Some(target) = self.qualified.get(&module.join(".")).cloned() else {
                self.error(
                    NOT_FOUND,
                    span,
                    format!(
                        "module `{}` is not imported (prelude: {})",
                        module.join("."),
                        self.scope.prelude.id
                    ),
                );
                return self.unresolved(name);
            };
            self.resolve_in_module(&target, name, span)
        }
    }

    /// Records the in-package dependency, if any, and builds the name.
    ///
    /// An in-package module is recorded as a dependency under its *Elm* name,
    /// which is what a document imports and what a module result reports, but
    /// named in the IR under its module path relative to the package, the way
    /// morphir-elm does.
    fn use_target(&mut self, target: ModuleTarget, name: &str) -> FqName {
        let module = if target.in_package {
            if target.module != self.module.name {
                self.depends_on.insert(target.module.clone());
            }
            boundary::relative_module(self.scope.package, &target.module)
        } else {
            target.module
        };
        FqName {
            package: target.package,
            module,
            name: name.to_string(),
        }
    }

    /// A name this very module declares.
    fn own_name(&self, name: &str) -> FqName {
        FqName {
            package: self.scope.package.to_vec(),
            module: boundary::relative_module(self.scope.package, &self.module.name),
            name: name.to_string(),
        }
    }

    fn resolve_in_module(&mut self, module: &[String], name: &str, span: Span) -> FqName {
        let spelled = names::type_spelling(name);
        match self
            .lookup_modules(module)
            .into_iter()
            .find(|target| target.types.contains(&spelled))
        {
            Some(target) => self.use_target(target, name),
            None => {
                self.error(
                    NOT_FOUND,
                    span,
                    format!(
                        "`{name}` not found in module `{}` (prelude: {})",
                        module.join("."),
                        self.scope.prelude.id
                    ),
                );
                self.unresolved(name)
            }
        }
    }

    /// Rules 3 and 4: every module a written path can name, in priority order
    /// — an in-package module (raw path, no alias mapping), then, after
    /// prelude alias mapping, a dependency module, then a prelude platform
    /// module.
    ///
    /// Dependency packages whose name is a prefix of the target path shadow
    /// the prelude copy of that package entirely: the longest matching prefix
    /// is tried first and shorter ones after it, but if none of them declares
    /// the module, the prelude platform module is *not* consulted.
    ///
    /// Qualified references take the first entry; unqualified references
    /// consider all of them, so a name reachable through two different modules
    /// is reported as ambiguous instead of silently picking one.
    /// Every target states its type names in their document spelling, so that a
    /// reference compares against them the same way whether they came from an
    /// in-package module's interface, a dependency's, or a prelude's TOML.
    fn lookup_modules(&self, raw: &[String]) -> Vec<ModuleTarget> {
        let mut found = Vec::new();

        if let Some(iface) = (self.scope.package_modules)(raw) {
            found.push(ModuleTarget {
                package: self.scope.package.to_vec(),
                module: raw.to_vec(),
                types: iface
                    .types
                    .into_iter()
                    .map(|t| names::type_spelling(&t.name))
                    .collect(),
                in_package: true,
            });
        }

        let target = self
            .scope
            .prelude
            .alias_for(raw)
            .unwrap_or_else(|| raw.to_vec());

        let mut deps: Vec<&DependencyInterface> = self
            .scope
            .dependencies
            .iter()
            .filter(|dep| starts_with(&target, &dep.package))
            .collect();
        deps.sort_by_key(|dep| std::cmp::Reverse(dep.package.len()));
        let shadows_prelude = !deps.is_empty();
        for dep in deps {
            // A dependency's module paths come out of a document too, so they
            // are matched in the same spelling as its type names.
            let remainder = spelled_path(&target[dep.package.len()..]);
            if let Some(module) = dep
                .modules
                .iter()
                .find(|m| spelled_path(&m.name) == remainder)
            {
                found.push(ModuleTarget {
                    package: dep.package.clone(),
                    module: module.name.clone(),
                    types: module
                        .types
                        .iter()
                        .map(|t| names::type_spelling(&t.name))
                        .collect(),
                    in_package: false,
                });
                break;
            }
        }
        if shadows_prelude {
            return found;
        }

        if let Some((pkg, module)) = self.scope.prelude.platform_module(&target) {
            found.push(ModuleTarget {
                package: split_dotted(&pkg.name),
                module: split_dotted(&module.name),
                types: module
                    .types
                    .iter()
                    .map(|t| names::type_spelling(&t.name))
                    .collect(),
                in_package: false,
            });
        }
        found
    }

    /// A placeholder for a reference that failed to resolve; the error list is
    /// non-empty by then, so this value is never returned to a caller.
    fn unresolved(&self, name: &str) -> FqName {
        self.own_name(name)
    }

    fn error(&mut self, code: &'static str, span: Span, message: String) {
        self.errors.push(ResolveError {
            code,
            span,
            message,
        });
    }
}

/// Whether `target` is reachable from `from` by following one or more edges.
/// Called with `from == target` it answers whether that node sits on a circle.
fn reaches(
    from: &str,
    target: &str,
    edges: &BTreeMap<String, BTreeSet<String>>,
    visited: &mut BTreeSet<String>,
) -> bool {
    let Some(next) = edges.get(from) else {
        return false;
    };
    for step in next {
        if step == target {
            return true;
        }
        if visited.insert(step.clone()) && reaches(step, target, edges, visited) {
            return true;
        }
    }
    false
}

/// `package.module` for diagnostics, e.g. `Morphir.SDK.String`.
fn qualified_module_name(target: &ModuleTarget) -> String {
    target
        .package
        .iter()
        .chain(target.module.iter())
        .cloned()
        .collect::<Vec<_>>()
        .join(".")
}

/// A module path in the spelling a Morphir document keeps.
fn spelled_path(path: &[String]) -> Vec<String> {
    path.iter()
        .map(|segment| names::type_spelling(segment))
        .collect()
}

fn split_dotted(name: &str) -> Vec<String> {
    name.split('.').map(str::to_string).collect()
}

/// The exposing entry's type name: `"Order(..)"` exposes the type `Order`.
fn exposed_name(entry: &str) -> String {
    match entry.strip_suffix("(..)") {
        Some(name) => name.to_string(),
        None => entry.to_string(),
    }
}

fn starts_with(path: &[String], prefix: &[String]) -> bool {
    path.len() >= prefix.len() && path[..prefix.len()] == *prefix
}

/// Orders modules so that each module comes after the in-package modules it
/// imports (Kahn's algorithm). On a cycle, returns the names of every module
/// that could not be ordered.
pub fn dependency_order(
    modules: &[(Vec<String>, Vec<Vec<String>>)],
) -> Result<Vec<usize>, Vec<Vec<String>>> {
    let index: BTreeMap<&[String], usize> = modules
        .iter()
        .enumerate()
        .map(|(i, (name, _))| (name.as_slice(), i))
        .collect();

    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); modules.len()];
    let mut in_degree = vec![0usize; modules.len()];
    for (i, (_, imports)) in modules.iter().enumerate() {
        let mut seen = BTreeSet::new();
        for import in imports {
            let Some(&dep) = index.get(import.as_slice()) else {
                continue; // not an in-package module; not an edge
            };
            if dep == i || !seen.insert(dep) {
                continue;
            }
            dependents[dep].push(i);
            in_degree[i] += 1;
        }
    }

    let mut ready: VecDeque<usize> = (0..modules.len()).filter(|i| in_degree[*i] == 0).collect();
    let mut order = Vec::with_capacity(modules.len());
    while let Some(i) = ready.pop_front() {
        order.push(i);
        for &dependent in &dependents[i] {
            in_degree[dependent] -= 1;
            if in_degree[dependent] == 0 {
                ready.push_back(dependent);
            }
        }
    }

    if order.len() != modules.len() {
        let ordered: BTreeSet<usize> = order.into_iter().collect();
        return Err((0..modules.len())
            .filter(|i| !ordered.contains(i))
            .map(|i| modules[i].0.clone())
            .collect());
    }
    Ok(order)
}
