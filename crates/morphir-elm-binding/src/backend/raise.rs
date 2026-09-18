//! Raising a resolved module back into the Elm AST.
//!
//! This is the inverse of [`crate::frontend::resolve`]: resolution turned every
//! written reference into a fully qualified name, and raising has to decide, for
//! each of those names, how a reader would write it — and what the module has to
//! import for that spelling to mean what the document says.
//!
//! # Import and qualification policy
//!
//! A reference is written in exactly one of three ways:
//!
//! 1. **Bare, because it is local.** The declaration is in this very module.
//! 2. **Bare, because the prelude already exposes it.** `Int`, `String`, `List`
//!    and the rest arrive through the prelude's implicit imports, which stand in
//!    for the imports Elm gives every module; naming them costs no import. A
//!    name this module also declares is never written this way — the local
//!    declaration would win — so it falls through to the third case.
//! 3. **Qualified by its full module path**, with a plain `import <Module>`
//!    line. `import My.Other` and `My.Other.Thing`; `import Dict` and
//!    `Dict.Dict`.
//!
//! The third case is deliberately the blunt one. An `as` alias or an
//! `exposing (…)` list would read better, but either can make one written name
//! mean two things once a second import is added, and this backend has no way
//! to ask the author which was meant. A full path cannot collide: it names one
//! module, and [`crate::frontend::resolve`] looks a qualified reference up under
//! that same full path, so everything generated here reads back as itself.
//!
//! The synthesised imports are sorted and deduplicated, so two documents that
//! say the same thing generate the same text.

use std::collections::{BTreeSet, HashSet};

use crate::ast;
use crate::names;
use crate::prelude::Prelude;
use crate::resolved::{Access, FqName, RField, RType, ResolvedBody, ResolvedModule, ResolvedType};
use crate::span::Span;

/// Generated nodes carry no source, so every span is empty.
const NONE: Span = Span { start: 0, end: 0 };

/// The Elm module a resolved module describes, with the imports its references
/// need.
///
/// `package` is the path of the package the module belongs to: it is what tells
/// a reference to a sibling module apart from a reference into the SDK.
pub fn raise(package: &[String], module: &ResolvedModule, prelude: &Prelude) -> ast::Module {
    let declared: HashSet<String> = module
        .types
        .iter()
        .map(|declaration| names::type_spelling(&declaration.name))
        .collect();
    let mut raiser = Raiser {
        package,
        module,
        prelude,
        declared,
        imports: BTreeSet::new(),
    };

    // A custom type with no constructors cannot be written in Elm at all. The
    // decoder drops the declarations it knows to be unwritable; this is the
    // backstop for any that reach here anyway, so that the module still parses.
    let writable: Vec<&ResolvedType> = module.types.iter().filter(|ty| writable(ty)).collect();

    let types: Vec<ast::TypeDecl> = writable
        .iter()
        .map(|declaration| raiser.declaration(declaration))
        .collect();

    let exposing = exposing(&writable);
    let imports = raiser
        .imports
        .into_iter()
        .map(|module| ast::Import {
            module,
            alias: None,
            exposing: None,
            span: NONE,
        })
        .collect();

    ast::Module {
        name: module
            .name
            .iter()
            .map(|s| names::type_spelling(s))
            .collect(),
        exposing,
        imports,
        doc: module.doc.clone(),
        types,
        skipped_values: Vec::new(),
        span: NONE,
    }
}

/// Whether Elm has a form for this declaration.
fn writable(declaration: &ResolvedType) -> bool {
    match &declaration.body {
        ResolvedBody::Alias(_) => true,
        ResolvedBody::Custom { constructors, .. } => !constructors.is_empty(),
    }
}

/// The module's exposing list: every public type in declaration order, with
/// `(..)` on the ones whose constructors are public too.
///
/// Elm forbids an empty exposing list, so a module with nothing public says
/// `exposing (..)` — the only other thing it can say. Nothing is lost: a module
/// no one can name a type in has no dependents to mislead.
fn exposing(types: &[&ResolvedType]) -> ast::Exposing {
    let exposed: Vec<ast::Exposed> = types
        .iter()
        .filter(|declaration| declaration.access == Access::Public)
        .map(|declaration| ast::Exposed::Type {
            name: names::type_spelling(&declaration.name),
            constructors: matches!(
                declaration.body,
                ResolvedBody::Custom {
                    constructor_access: Access::Public,
                    ..
                }
            ),
        })
        .collect();
    if exposed.is_empty() {
        // Elm requires a non-empty exposing list, and a module with no public
        // types has nothing meaningful to name in one, so `(..)` is the only
        // legal spelling left.
        ast::Exposing::All
    } else {
        ast::Exposing::Explicit(exposed)
    }
}

struct Raiser<'a> {
    package: &'a [String],
    module: &'a ResolvedModule,
    prelude: &'a Prelude,
    /// Type names this module declares; they shadow every import.
    declared: HashSet<String>,
    imports: BTreeSet<Vec<String>>,
}

impl Raiser<'_> {
    fn declaration(&mut self, declaration: &ResolvedType) -> ast::TypeDecl {
        let name = names::type_spelling(&declaration.name);
        let params: Vec<String> = declaration
            .params
            .iter()
            .map(|param| names::value_spelling(param))
            .collect();
        match &declaration.body {
            ResolvedBody::Alias(body) => ast::TypeDecl::Alias {
                name,
                params,
                body: self.ty(body),
                doc: declaration.doc.clone(),
                span: NONE,
            },
            ResolvedBody::Custom { constructors, .. } => ast::TypeDecl::Custom {
                name,
                params,
                constructors: constructors
                    .iter()
                    .map(|constructor| ast::Constructor {
                        name: names::type_spelling(&constructor.name),
                        args: constructor.args.iter().map(|arg| self.ty(arg)).collect(),
                        span: NONE,
                    })
                    .collect(),
                doc: declaration.doc.clone(),
                span: NONE,
            },
        }
    }

    fn ty(&mut self, value: &RType) -> ast::TypeExpr {
        match value {
            RType::Var(variable) => ast::TypeExpr::Var {
                name: names::value_spelling(variable),
                span: NONE,
            },
            RType::Ref(reference, arguments) => {
                let args = arguments.iter().map(|arg| self.ty(arg)).collect();
                let (module, name) = self.reference(reference);
                ast::TypeExpr::Ref {
                    module,
                    name,
                    args,
                    span: NONE,
                }
            }
            RType::Record(fields) => ast::TypeExpr::Record {
                fields: fields.iter().map(|field| self.field(field)).collect(),
                span: NONE,
            },
            RType::ExtensibleRecord(variable, fields) => ast::TypeExpr::ExtensibleRecord {
                base: names::value_spelling(variable),
                fields: fields.iter().map(|field| self.field(field)).collect(),
                span: NONE,
            },
            RType::Tuple(elements) => ast::TypeExpr::Tuple {
                items: elements.iter().map(|element| self.ty(element)).collect(),
                span: NONE,
            },
            RType::Function(argument, result) => ast::TypeExpr::Function {
                arg: Box::new(self.ty(argument)),
                result: Box::new(self.ty(result)),
                span: NONE,
            },
            RType::Unit => ast::TypeExpr::Unit { span: NONE },
        }
    }

    fn field(&mut self, field: &RField) -> ast::Field {
        ast::Field {
            name: names::value_spelling(&field.name),
            ty: self.ty(&field.ty),
        }
    }

    /// How this reference is written, and what that costs in imports.
    fn reference(&mut self, reference: &FqName) -> (Vec<String>, String) {
        let name = names::type_spelling(&reference.name);

        if reference.package == self.package {
            if reference.module == self.module.name {
                return (Vec::new(), name);
            }
            self.imports.insert(reference.module.clone());
            return (reference.module.clone(), name);
        }

        // Outside the package, the module a reader writes is the one the
        // prelude maps onto the resolved one: `Morphir.SDK.Dict` is written
        // `Dict`.
        let resolved: Vec<String> = reference
            .package
            .iter()
            .chain(reference.module.iter())
            .cloned()
            .collect();
        let written = self
            .prelude
            .unalias(&resolved)
            .unwrap_or_else(|| resolved.clone());

        // A name this module declares itself would win over any import, so an
        // implicitly imported name that collides with one is still qualified.
        if !self.declared.contains(&name) && self.implicitly_exposes(&written, &name) {
            return (Vec::new(), name);
        }

        self.imports.insert(written.clone());
        (written, name)
    }

    /// Whether the prelude's implicit imports already put `name` in scope
    /// unqualified, as `import <written> exposing (…)` would.
    fn implicitly_exposes(&self, written: &[String], name: &str) -> bool {
        let dotted = written.join(".");
        self.prelude
            .implicit_import
            .iter()
            .filter(|implicit| implicit.module == dotted)
            .flat_map(|implicit| implicit.exposing.iter())
            .any(|entry| {
                if entry == ".." {
                    self.platform_declares(written, name)
                } else {
                    names::type_spelling(exposed_name(entry)) == name
                }
            })
    }

    /// Whether the platform module `written` names declares `name`, for an
    /// implicit `exposing (..)`.
    fn platform_declares(&self, written: &[String], name: &str) -> bool {
        let target = self
            .prelude
            .alias_for(written)
            .unwrap_or_else(|| written.to_vec());
        self.prelude
            .platform_module(&target)
            .is_some_and(|(_, module)| {
                module
                    .types
                    .iter()
                    .any(|ty| names::type_spelling(&ty.name) == name)
            })
    }
}

/// The exposing entry's type name: `"Order(..)"` exposes the type `Order`.
fn exposed_name(entry: &str) -> &str {
    entry.strip_suffix("(..)").unwrap_or(entry)
}
