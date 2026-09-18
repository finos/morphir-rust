//! The Elm type AST produced by lowering the tree-sitter CST.
//!
//! This module only models what Task 2 needs: module headers, imports, and
//! type declarations (aliases and custom types). Value declarations are not
//! lowered into an expression AST; they are recorded as [`SkippedValue`]s.

use crate::span::Span;

/// One Elm module: its header, its imports and its type declarations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    /// The dotted module name, one segment per element.
    pub name: Vec<String>,
    /// The module's `exposing` list.
    pub exposing: Exposing,
    /// The module's import lines, in source order.
    pub imports: Vec<Import>,
    /// The module's `{-| … -}` documentation comment.
    pub doc: Option<String>,
    /// The type declarations, in source order.
    pub types: Vec<TypeDecl>,
    /// The value declarations this frontend does not lower.
    pub skipped_values: Vec<SkippedValue>,
    /// The span of the module header.
    pub span: Span,
}

/// What a module or an import exposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exposing {
    /// `exposing (..)`.
    All,
    /// `exposing (A, b)`, with the listed entries.
    Explicit(Vec<Exposed>),
}

/// One entry of an explicit `exposing` list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exposed {
    /// A type, and whether its constructors are exposed too (`T(..)`).
    Type {
        /// The type's name.
        name: String,
        /// Whether the entry was written `T(..)`.
        constructors: bool,
    },
    /// A value, by name.
    Value(String),
}

/// One `import` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    /// The imported module's dotted path.
    pub module: Vec<String>,
    /// The `as` alias, when the line states one.
    pub alias: Option<String>,
    /// The line's `exposing` list, when it states one.
    pub exposing: Option<Exposing>,
    /// The span of the import line.
    pub span: Span,
}

/// A type declaration: `type alias` or `type`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeDecl {
    /// `type alias Name params = body`.
    Alias {
        /// The declared name.
        name: String,
        /// The type parameters, in order.
        params: Vec<String>,
        /// The type the alias stands for.
        body: TypeExpr,
        /// The declaration's documentation comment.
        doc: Option<String>,
        /// The span of the declaration.
        span: Span,
    },
    /// `type Name params = Ctor args | …`.
    Custom {
        /// The declared name.
        name: String,
        /// The type parameters, in order.
        params: Vec<String>,
        /// The constructors, in order.
        constructors: Vec<Constructor>,
        /// The declaration's documentation comment.
        doc: Option<String>,
        /// The span of the declaration.
        span: Span,
    },
}

/// One constructor of a custom type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constructor {
    /// The constructor's name.
    pub name: String,
    /// Its positional argument types.
    pub args: Vec<TypeExpr>,
    /// The span of the constructor.
    pub span: Span,
}

/// A type expression, as it was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeExpr {
    /// A type variable, such as `a`.
    Var {
        /// The variable's name.
        name: String,
        /// The span of the expression.
        span: Span,
    },
    /// A reference to a declared type, with its arguments.
    Ref {
        /// The qualifier the reference was written with, if any.
        module: Vec<String>,
        /// The referenced type's name.
        name: String,
        /// The type arguments applied to it.
        args: Vec<TypeExpr>,
        /// The span of the expression.
        span: Span,
    },
    /// A record type, `{ a : A }`.
    Record {
        /// The record's fields, in order.
        fields: Vec<Field>,
        /// The span of the expression.
        span: Span,
    },
    /// An extensible record type, `{ r | a : A }`.
    ExtensibleRecord {
        /// The base type variable, `r`.
        base: String,
        /// The fields the extension adds, in order.
        fields: Vec<Field>,
        /// The span of the expression.
        span: Span,
    },
    /// A tuple type, `( A, B )`.
    Tuple {
        /// The tuple's element types, in order.
        items: Vec<TypeExpr>,
        /// The span of the expression.
        span: Span,
    },
    /// A function type, `A -> B`.
    Function {
        /// The argument type.
        arg: Box<TypeExpr>,
        /// The result type.
        result: Box<TypeExpr>,
        /// The span of the expression.
        span: Span,
    },
    /// The unit type, `()`.
    Unit {
        /// The span of the expression.
        span: Span,
    },
}

/// One field of a record type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    /// The field's name.
    pub name: String,
    /// The field's type.
    pub ty: TypeExpr,
}

/// A value declaration this frontend records rather than lowers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedValue {
    /// The value's name.
    pub name: String,
    /// The span of the declaration.
    pub span: Span,
}

impl TypeDecl {
    /// The declared name, however the declaration was written.
    pub fn name(&self) -> &str {
        match self {
            TypeDecl::Alias { name, .. } => name,
            TypeDecl::Custom { name, .. } => name,
        }
    }

    /// The span of the whole declaration.
    pub fn span(&self) -> Span {
        match self {
            TypeDecl::Alias { span, .. } => *span,
            TypeDecl::Custom { span, .. } => *span,
        }
    }
}

impl TypeExpr {
    /// The span of this expression, whichever form it takes.
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Var { span, .. } => *span,
            TypeExpr::Ref { span, .. } => *span,
            TypeExpr::Record { span, .. } => *span,
            TypeExpr::ExtensibleRecord { span, .. } => *span,
            TypeExpr::Tuple { span, .. } => *span,
            TypeExpr::Function { span, .. } => *span,
            TypeExpr::Unit { span, .. } => *span,
        }
    }
}
