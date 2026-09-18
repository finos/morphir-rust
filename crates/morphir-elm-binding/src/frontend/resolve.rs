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
    pub prelude: &'a Prelude,
    /// Looks up a module of *this* package by its module path (the request's
    /// own modules plus the incremental baseline).
    pub package_modules: &'a dyn Fn(&[String]) -> Option<Interface>,
    pub dependencies: &'a [DependencyInterface],
}

/// The public interfaces of one dependency package.
pub struct DependencyInterface {
    pub package: Vec<String>,
    /// Module interfaces, named relative to `package`.
    pub modules: Vec<Interface>,
}

/// A resolution failure. `span` is the span of the offending reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveError {
    pub code: &'static str,
    pub span: Span,
    pub message: String,
}

const NOT_FOUND: &str = "ELM_RESOLVE_NOT_FOUND";
const AMBIGUOUS: &str = "ELM_RESOLVE_AMBIGUOUS";

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
        local: module.types.iter().map(|t| t.name().to_string()).collect(),
        visible: BTreeMap::new(),
        qualified: BTreeMap::new(),
        depends_on: BTreeSet::new(),
        errors: Vec::new(),
    };
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

    fn expose_name(&mut self, name: &str, module: &[String]) {
        let candidates = self.visible.entry(name.to_string()).or_default();
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
        if module.is_empty() {
            // Rule 1: a locally declared type wins over every import and is
            // never ambiguous.
            if self.local.contains(name) {
                return FqName {
                    package: self.scope.package.to_vec(),
                    module: self.module.name.clone(),
                    name: name.to_string(),
                };
            }
            let candidates = self.visible.get(name).cloned().unwrap_or_default();
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
                    if !target.types.iter().any(|t| t == name) {
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
    fn use_target(&mut self, target: ModuleTarget, name: &str) -> FqName {
        if target.in_package && target.module != self.module.name {
            self.depends_on.insert(target.module.clone());
        }
        FqName {
            package: target.package,
            module: target.module,
            name: name.to_string(),
        }
    }

    fn resolve_in_module(&mut self, module: &[String], name: &str, span: Span) -> FqName {
        match self
            .lookup_modules(module)
            .into_iter()
            .find(|target| target.types.iter().any(|t| t == name))
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
    fn lookup_modules(&self, raw: &[String]) -> Vec<ModuleTarget> {
        let mut found = Vec::new();

        if let Some(iface) = (self.scope.package_modules)(raw) {
            found.push(ModuleTarget {
                package: self.scope.package.to_vec(),
                module: raw.to_vec(),
                types: iface.types.into_iter().map(|t| t.name).collect(),
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
            let remainder = &target[dep.package.len()..];
            if let Some(module) = dep.modules.iter().find(|m| m.name == remainder) {
                found.push(ModuleTarget {
                    package: dep.package.clone(),
                    module: module.name.clone(),
                    types: module.types.iter().map(|t| t.name.clone()).collect(),
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
                types: module.types.iter().map(|t| t.name.clone()).collect(),
                in_package: false,
            });
        }
        found
    }

    /// A placeholder for a reference that failed to resolve; the error list is
    /// non-empty by then, so this value is never returned to a caller.
    fn unresolved(&self, name: &str) -> FqName {
        FqName {
            package: self.scope.package.to_vec(),
            module: self.module.name.clone(),
            name: name.to_string(),
        }
    }

    fn error(&mut self, code: &'static str, span: Span, message: String) {
        self.errors.push(ResolveError {
            code,
            span,
            message,
        });
    }
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
