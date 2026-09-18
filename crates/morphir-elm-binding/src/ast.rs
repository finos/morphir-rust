//! The Elm type AST produced by lowering the tree-sitter CST.
//!
//! This module only models what Task 2 needs: module headers, imports, and
//! type declarations (aliases and custom types). Value declarations are not
//! lowered into an expression AST; they are recorded as [`SkippedValue`]s.

use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub name: Vec<String>,
    pub exposing: Exposing,
    pub imports: Vec<Import>,
    pub doc: Option<String>,
    pub types: Vec<TypeDecl>,
    pub skipped_values: Vec<SkippedValue>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exposing {
    All,
    Explicit(Vec<Exposed>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exposed {
    Type { name: String, constructors: bool },
    Value(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub module: Vec<String>,
    pub alias: Option<String>,
    pub exposing: Option<Exposing>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeDecl {
    Alias {
        name: String,
        params: Vec<String>,
        body: TypeExpr,
        doc: Option<String>,
        span: Span,
    },
    Custom {
        name: String,
        params: Vec<String>,
        constructors: Vec<Constructor>,
        doc: Option<String>,
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constructor {
    pub name: String,
    pub args: Vec<TypeExpr>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeExpr {
    Var {
        name: String,
        span: Span,
    },
    Ref {
        module: Vec<String>,
        name: String,
        args: Vec<TypeExpr>,
        span: Span,
    },
    Record {
        fields: Vec<Field>,
        span: Span,
    },
    ExtensibleRecord {
        base: String,
        fields: Vec<Field>,
        span: Span,
    },
    Tuple {
        items: Vec<TypeExpr>,
        span: Span,
    },
    Function {
        arg: Box<TypeExpr>,
        result: Box<TypeExpr>,
        span: Span,
    },
    Unit {
        span: Span,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub ty: TypeExpr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkippedValue {
    pub name: String,
    pub span: Span,
}

impl TypeDecl {
    pub fn name(&self) -> &str {
        match self {
            TypeDecl::Alias { name, .. } => name,
            TypeDecl::Custom { name, .. } => name,
        }
    }

    pub fn span(&self) -> Span {
        match self {
            TypeDecl::Alias { span, .. } => *span,
            TypeDecl::Custom { span, .. } => *span,
        }
    }
}

impl TypeExpr {
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
