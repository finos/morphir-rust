//! Gleam Pretty Printer - Wadler-style pretty printing
//!
//! This module uses the `pretty` crate to generate well-formatted Gleam code
//! from AST nodes. Inspired by glam (Gleam's pretty printing library) and
//! glance_printer.
//!
//! Reference:
//! - glam: https://github.com/lpil/glam
//! - glance_printer: https://github.com/lpil/glance

use pretty::{DocAllocator, DocBuilder, RcAllocator};

use crate::frontend::ast::{
    Access, BinaryOperator, BitStringOption, BitStringSegment, CaseBranch, Expr, Field, Literal,
    ModuleIR, Pattern, Statement, TypeDef, TypeExpr, ValueDef, Variant,
};

/// Default line width for pretty printing
pub const DEFAULT_WIDTH: usize = 80;

/// Default indentation level
pub const INDENT: isize = 2;

/// Type alias for our document builder
type Doc<'a> = DocBuilder<'a, RcAllocator, ()>;

/// Operator precedence levels (matching Gleam's precedence)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Precedence {
    Lowest = 0,
    Pipe = 1,           // |>
    Or = 2,             // ||
    And = 3,            // &&
    Equality = 4,       // == !=
    Comparison = 5,     // < > <= >= <. >. <=. >=.
    Concat = 6,         // <>
    Addition = 7,       // + - +. -.
    Multiplication = 8, // * / % *. /.
    Unary = 9,          // - !
    Call = 10,          // f(x)
    Access = 11,        // x.y
}

impl Precedence {
    /// Get precedence for a binary operator
    pub fn from_binary_op(op: &BinaryOperator) -> Self {
        match op {
            BinaryOperator::Pipe => Precedence::Pipe,
            BinaryOperator::Or => Precedence::Or,
            BinaryOperator::And => Precedence::And,
            BinaryOperator::Eq | BinaryOperator::NotEq => Precedence::Equality,
            BinaryOperator::LtInt
            | BinaryOperator::LtEqInt
            | BinaryOperator::GtInt
            | BinaryOperator::GtEqInt
            | BinaryOperator::LtFloat
            | BinaryOperator::LtEqFloat
            | BinaryOperator::GtFloat
            | BinaryOperator::GtEqFloat => Precedence::Comparison,
            BinaryOperator::Concatenate => Precedence::Concat,
            BinaryOperator::AddInt
            | BinaryOperator::SubInt
            | BinaryOperator::AddFloat
            | BinaryOperator::SubFloat => Precedence::Addition,
            BinaryOperator::MultInt
            | BinaryOperator::DivInt
            | BinaryOperator::RemainderInt
            | BinaryOperator::MultFloat
            | BinaryOperator::DivFloat => Precedence::Multiplication,
        }
    }
}

/// Gleam pretty printer using the `pretty` crate
pub struct GleamPrinter<'a> {
    alloc: &'a RcAllocator,
}

impl<'a> GleamPrinter<'a> {
    /// Create a new printer with the given allocator
    pub fn new(alloc: &'a RcAllocator) -> Self {
        Self { alloc }
    }

    // ========================================================================
    // Helper methods
    // ========================================================================

    /// Create a text document
    fn text<S: Into<std::borrow::Cow<'a, str>>>(&self, s: S) -> Doc<'a> {
        self.alloc.text(s)
    }

    /// Create a nil (empty) document
    fn nil(&self) -> Doc<'a> {
        self.alloc.nil()
    }

    /// Create a line break (soft - can be replaced with space in group)
    fn line(&self) -> Doc<'a> {
        self.alloc.line()
    }

    /// Create a hard line break (always breaks)
    fn hardline(&self) -> Doc<'a> {
        self.alloc.hardline()
    }

    /// Create a space
    fn space(&self) -> Doc<'a> {
        self.alloc.space()
    }

    /// Join documents with a separator
    fn join<I>(&self, docs: I, sep: Doc<'a>) -> Doc<'a>
    where
        I: IntoIterator<Item = Doc<'a>>,
    {
        self.alloc.intersperse(docs, sep)
    }

    /// Wrap document in parentheses
    fn parens(&self, doc: Doc<'a>) -> Doc<'a> {
        self.text("(").append(doc).append(self.text(")"))
    }

    /// Wrap document in braces with proper grouping
    fn braces(&self, doc: Doc<'a>) -> Doc<'a> {
        self.text("{")
            .append(doc.nest(INDENT))
            .append(self.text("}"))
            .group()
    }

    /// Wrap document in brackets
    fn brackets(&self, doc: Doc<'a>) -> Doc<'a> {
        self.text("[").append(doc).append(self.text("]"))
    }

    // ========================================================================
    // Module printing
    // ========================================================================

    /// Print a complete module
    pub fn module(&self, m: &ModuleIR) -> Doc<'a> {
        let mut parts = Vec::new();

        // Module documentation
        if let Some(doc) = &m.doc {
            parts.push(self.text(format!("/// {}", doc)));
            parts.push(self.hardline());
        }

        // Type definitions
        for type_def in &m.types {
            parts.push(self.type_def(type_def));
            parts.push(self.hardline());
            parts.push(self.hardline());
        }

        // Value definitions
        for value_def in &m.values {
            parts.push(self.value_def(value_def));
            parts.push(self.hardline());
            parts.push(self.hardline());
        }

        self.alloc.concat(parts)
    }

    /// Print a type definition
    pub fn type_def(&self, t: &TypeDef) -> Doc<'a> {
        let access = match t.access {
            Access::Public => self.text("pub "),
            Access::Private => self.nil(),
        };

        let params = if t.params.is_empty() {
            self.nil()
        } else {
            let params_doc = self.join(
                t.params.iter().map(|p| self.text(p.clone())),
                self.text(", "),
            );
            self.parens(params_doc)
        };

        // Check if this is a custom type (has variants)
        match &t.body {
            TypeExpr::CustomType { variants } => {
                let variants_doc = self.join(
                    variants.iter().map(|v| self.variant(v)),
                    self.hardline().append(self.text("| ")),
                );

                access
                    .append(self.text("type "))
                    .append(self.text(t.name.clone()))
                    .append(params)
                    .append(self.text(" {"))
                    .append(self.hardline())
                    .append(self.text("  "))
                    .append(variants_doc)
                    .append(self.hardline())
                    .append(self.text("}"))
            }
            _ => {
                // Type alias
                access
                    .append(self.text("type "))
                    .append(self.text(t.name.clone()))
                    .append(params)
                    .append(self.text(" = "))
                    .append(self.type_expr(&t.body))
            }
        }
    }

    /// Print a variant
    pub fn variant(&self, v: &Variant) -> Doc<'a> {
        let name = self.text(v.name.clone());
        if v.fields.is_empty() {
            name
        } else {
            let fields_doc = self.join(v.fields.iter().map(|f| self.type_expr(f)), self.text(", "));
            name.append(self.parens(fields_doc))
        }
    }

    /// Print a value definition
    pub fn value_def(&self, v: &ValueDef) -> Doc<'a> {
        let access = match v.access {
            Access::Public => self.text("pub "),
            Access::Private => self.nil(),
        };

        // Check if this is a function (lambda body) or constant
        match &v.body {
            Expr::Lambda { params, body } => {
                let params_doc =
                    self.join(params.iter().map(|p| self.text(p.clone())), self.text(", "));

                let annotation = if let Some(ann) = &v.type_annotation {
                    self.text(" -> ").append(self.type_expr(ann))
                } else {
                    self.nil()
                };

                access
                    .append(self.text("fn "))
                    .append(self.text(v.name.clone()))
                    .append(self.parens(params_doc))
                    .append(annotation)
                    .append(self.text(" {"))
                    .append(self.line())
                    .append(self.expr(body))
                    .nest(INDENT)
                    .append(self.line())
                    .append(self.text("}"))
                    .group()
            }
            _ => {
                // Constant
                let annotation = if let Some(ann) = &v.type_annotation {
                    self.text(": ").append(self.type_expr(ann))
                } else {
                    self.nil()
                };

                access
                    .append(self.text("const "))
                    .append(self.text(v.name.clone()))
                    .append(annotation)
                    .append(self.text(" = "))
                    .append(self.expr(&v.body))
            }
        }
    }

    // ========================================================================
    // Type expression printing
    // ========================================================================

    /// Print a type expression
    pub fn type_expr(&self, t: &TypeExpr) -> Doc<'a> {
        match t {
            TypeExpr::Variable { name } => self.text(name.clone()),
            TypeExpr::Unit => self.text("Nil"),
            TypeExpr::Function {
                parameters,
                return_type,
            } => {
                let params_doc = self.join(
                    parameters.iter().map(|p| self.type_expr(p)),
                    self.text(", "),
                );
                self.text("fn(")
                    .append(params_doc)
                    .append(self.text(") -> "))
                    .append(self.type_expr(return_type))
            }
            TypeExpr::Record { fields } => {
                if fields.is_empty() {
                    self.text("{}")
                } else {
                    let fields_doc = self.join(
                        fields.iter().map(|(name, ty)| {
                            self.text(name.clone())
                                .append(self.text(": "))
                                .append(self.type_expr(ty))
                        }),
                        self.text(", "),
                    );
                    self.braces(self.space().append(fields_doc).append(self.space()))
                }
            }
            TypeExpr::Tuple { elements } => {
                let elements_doc =
                    self.join(elements.iter().map(|e| self.type_expr(e)), self.text(", "));
                self.text("#(").append(elements_doc).append(self.text(")"))
            }
            TypeExpr::Named {
                module,
                name,
                parameters,
            } => {
                let qualified_name = if let Some(m) = module {
                    self.text(format!("{}.{}", m, name))
                } else {
                    self.text(name.clone())
                };

                if parameters.is_empty() {
                    qualified_name
                } else {
                    let params_doc = self.join(
                        parameters.iter().map(|p| self.type_expr(p)),
                        self.text(", "),
                    );
                    qualified_name.append(self.parens(params_doc))
                }
            }
            TypeExpr::CustomType { variants } => {
                let variants_doc =
                    self.join(variants.iter().map(|v| self.variant(v)), self.text(" | "));
                variants_doc
            }
            TypeExpr::Hole { name } => self.text(format!("_{}", name)),
        }
    }

    // ========================================================================
    // Expression printing
    // ========================================================================

    /// Print an expression
    pub fn expr(&self, e: &Expr) -> Doc<'a> {
        self.expr_prec(e, Precedence::Lowest)
    }

    /// Print an expression with precedence context
    fn expr_prec(&self, e: &Expr, outer_prec: Precedence) -> Doc<'a> {
        match e {
            Expr::Literal { value } => self.literal(value),
            Expr::Variable { name } => self.text(name.clone()),
            Expr::Constructor { module, name } => {
                if let Some(m) = module {
                    self.text(format!("{}.{}", m, name))
                } else {
                    self.text(name.clone())
                }
            }
            Expr::Apply {
                function,
                arguments,
            } => {
                let fn_doc = self.expr_prec(function, Precedence::Call);
                let args_doc = self.join(
                    arguments.iter().map(|a| self.field_expr(a)),
                    self.text(", "),
                );
                fn_doc.append(self.parens(args_doc))
            }
            Expr::Lambda { params, body } => {
                let params_doc =
                    self.join(params.iter().map(|p| self.text(p.clone())), self.text(", "));
                self.text("fn(")
                    .append(params_doc)
                    .append(self.text(") { "))
                    .append(self.expr(body))
                    .append(self.text(" }"))
                    .group()
            }
            Expr::Let { name, value, body } => self
                .text("let ")
                .append(self.text(name.clone()))
                .append(self.text(" = "))
                .append(self.expr(value))
                .append(self.hardline())
                .append(self.expr(body)),
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => self
                .text("case ")
                .append(self.expr(condition))
                .append(self.text(" {"))
                .append(self.line())
                .append(self.text("True -> "))
                .append(self.expr(then_branch))
                .append(self.line())
                .append(self.text("False -> "))
                .append(self.expr(else_branch))
                .nest(INDENT)
                .append(self.line())
                .append(self.text("}"))
                .group(),
            Expr::Record { fields } => {
                if fields.is_empty() {
                    self.text("{}")
                } else {
                    let fields_doc = self.join(
                        fields.iter().map(|(name, value)| {
                            self.text(name.clone())
                                .append(self.text(": "))
                                .append(self.expr(value))
                        }),
                        self.text(", ").append(self.line()),
                    );
                    self.braces(self.line().append(fields_doc).append(self.line()))
                }
            }
            Expr::FieldAccess { container, label } => self
                .expr_prec(container, Precedence::Access)
                .append(self.text("."))
                .append(self.text(label.clone())),
            Expr::Tuple { elements } => {
                let elements_doc =
                    self.join(elements.iter().map(|e| self.expr(e)), self.text(", "));
                self.text("#(").append(elements_doc).append(self.text(")"))
            }
            Expr::TupleIndex { tuple, index } => self
                .expr_prec(tuple, Precedence::Access)
                .append(self.text(format!(".{}", index))),
            Expr::Case { subjects, clauses } => {
                let subjects_doc =
                    self.join(subjects.iter().map(|s| self.expr(s)), self.text(", "));
                let clauses_doc =
                    self.join(clauses.iter().map(|c| self.case_branch(c)), self.hardline());
                self.text("case ")
                    .append(subjects_doc)
                    .append(self.text(" {"))
                    .append(self.hardline())
                    .append(clauses_doc)
                    .nest(INDENT)
                    .append(self.hardline())
                    .append(self.text("}"))
            }
            Expr::BinaryOp { op, left, right } => {
                let op_prec = Precedence::from_binary_op(op);
                let needs_parens = op_prec < outer_prec;

                let doc = self
                    .expr_prec(left, op_prec)
                    .append(self.space())
                    .append(self.binary_op(op))
                    .append(self.space())
                    .append(self.expr_prec(right, op_prec));

                if needs_parens { self.parens(doc) } else { doc }
            }
            Expr::NegateInt { value } => self
                .text("-")
                .append(self.expr_prec(value, Precedence::Unary)),
            Expr::NegateBool { value } => self
                .text("!")
                .append(self.expr_prec(value, Precedence::Unary)),
            Expr::List { elements, tail } => {
                if elements.is_empty() && tail.is_none() {
                    self.text("[]")
                } else {
                    let elements_doc =
                        self.join(elements.iter().map(|e| self.expr(e)), self.text(", "));
                    let with_tail = if let Some(t) = tail {
                        elements_doc.append(self.text(", ..")).append(self.expr(t))
                    } else {
                        elements_doc
                    };
                    self.brackets(with_tail)
                }
            }
            Expr::Block { statements } => {
                let stmts_doc = self.join(
                    statements.iter().map(|s| self.statement(s)),
                    self.hardline(),
                );
                self.braces(self.hardline().append(stmts_doc).append(self.hardline()))
            }
            Expr::Panic { message } => {
                let msg_doc = if let Some(m) = message {
                    self.parens(self.expr(m))
                } else {
                    self.nil()
                };
                self.text("panic").append(msg_doc)
            }
            Expr::Todo { message } => {
                let msg_doc = if let Some(m) = message {
                    self.parens(self.expr(m))
                } else {
                    self.nil()
                };
                self.text("todo").append(msg_doc)
            }
            Expr::Echo { expression, body } => {
                let doc = self.text("echo ").append(self.expr(expression));
                if let Some(b) = body {
                    doc.append(self.hardline()).append(self.expr(b))
                } else {
                    doc
                }
            }
            Expr::BitString { segments } => {
                let segments_doc = self.join(
                    segments.iter().map(|s| self.bit_string_segment_expr(s)),
                    self.text(", "),
                );
                self.text("<<").append(segments_doc).append(self.text(">>"))
            }
            Expr::FnCapture {
                function,
                arguments_before,
                arguments_after,
            } => {
                let fn_doc = self.expr(function);
                let before_doc = self.join(
                    arguments_before.iter().map(|a| self.field_expr(a)),
                    self.text(", "),
                );
                let after_doc = self.join(
                    arguments_after.iter().map(|a| self.field_expr(a)),
                    self.text(", "),
                );
                let args = if arguments_before.is_empty() {
                    self.text("_, ").append(after_doc)
                } else if arguments_after.is_empty() {
                    before_doc.append(self.text(", _"))
                } else {
                    before_doc.append(self.text(", _, ")).append(after_doc)
                };
                fn_doc.append(self.parens(args))
            }
            Expr::RecordUpdate {
                module,
                constructor,
                record,
                fields,
            } => {
                let constructor_doc = if let Some(m) = module {
                    self.text(format!("{}.{}", m, constructor))
                } else {
                    self.text(constructor.clone())
                };
                let fields_doc = self.join(
                    fields.iter().map(|(name, value)| {
                        self.text(name.clone())
                            .append(self.text(": "))
                            .append(self.expr(value))
                    }),
                    self.text(", "),
                );
                constructor_doc
                    .append(self.text("(.."))
                    .append(self.expr(record))
                    .append(self.text(", "))
                    .append(fields_doc)
                    .append(self.text(")"))
            }
        }
    }

    /// Print a field expression
    fn field_expr(&self, f: &Field<Expr>) -> Doc<'a> {
        match f {
            Field::Labelled { label, item } => self
                .text(label.clone())
                .append(self.text(": "))
                .append(self.expr(item)),
            Field::Shorthand { name } => self.text(name.clone()),
            Field::Unlabelled { item } => self.expr(item),
        }
    }

    /// Print a literal
    fn literal(&self, lit: &Literal) -> Doc<'a> {
        match lit {
            Literal::Bool { value } => {
                if *value {
                    self.text("True")
                } else {
                    self.text("False")
                }
            }
            Literal::Int { value } => self.text(value.to_string()),
            Literal::Float { value } => self.text(format!("{:?}", value)),
            Literal::String { value } => self.text(format!("{:?}", value)),
            Literal::Char { value } => self.text(format!("'{}'", value)),
        }
    }

    /// Print a binary operator
    fn binary_op(&self, op: &BinaryOperator) -> Doc<'a> {
        let s = match op {
            BinaryOperator::And => "&&",
            BinaryOperator::Or => "||",
            BinaryOperator::Eq => "==",
            BinaryOperator::NotEq => "!=",
            BinaryOperator::LtInt => "<",
            BinaryOperator::LtEqInt => "<=",
            BinaryOperator::GtInt => ">",
            BinaryOperator::GtEqInt => ">=",
            BinaryOperator::LtFloat => "<.",
            BinaryOperator::LtEqFloat => "<=.",
            BinaryOperator::GtFloat => ">.",
            BinaryOperator::GtEqFloat => ">=.",
            BinaryOperator::AddInt => "+",
            BinaryOperator::SubInt => "-",
            BinaryOperator::MultInt => "*",
            BinaryOperator::DivInt => "/",
            BinaryOperator::RemainderInt => "%",
            BinaryOperator::AddFloat => "+.",
            BinaryOperator::SubFloat => "-.",
            BinaryOperator::MultFloat => "*.",
            BinaryOperator::DivFloat => "/.",
            BinaryOperator::Pipe => "|>",
            BinaryOperator::Concatenate => "<>",
        };
        self.text(s)
    }

    /// Print a statement
    fn statement(&self, s: &Statement) -> Doc<'a> {
        match s {
            Statement::Use { patterns, function } => {
                let patterns_doc =
                    self.join(patterns.iter().map(|p| self.pattern(p)), self.text(", "));
                self.text("use ")
                    .append(patterns_doc)
                    .append(self.text(" <- "))
                    .append(self.expr(function))
            }
            Statement::Assignment {
                pattern,
                annotation,
                value,
            } => {
                let ann_doc = if let Some(ann) = annotation {
                    self.text(": ").append(self.type_expr(ann))
                } else {
                    self.nil()
                };
                self.text("let ")
                    .append(self.pattern(pattern))
                    .append(ann_doc)
                    .append(self.text(" = "))
                    .append(self.expr(value))
            }
            Statement::Expression(e) => self.expr(e),
        }
    }

    /// Print a case branch
    fn case_branch(&self, c: &CaseBranch) -> Doc<'a> {
        self.pattern(&c.pattern)
            .append(self.text(" -> "))
            .append(self.expr(&c.body))
    }

    /// Print a bit string segment for expressions
    fn bit_string_segment_expr(&self, seg: &BitStringSegment<Expr>) -> Doc<'a> {
        let value_doc = self.expr(&seg.value);
        if seg.options.is_empty() {
            value_doc
        } else {
            let options_doc = self.join(
                seg.options.iter().map(|o| self.bit_string_option(o)),
                self.text("-"),
            );
            value_doc.append(self.text(":")).append(options_doc)
        }
    }

    /// Print a bit string option
    fn bit_string_option(&self, opt: &BitStringOption) -> Doc<'a> {
        match opt {
            BitStringOption::Bytes => self.text("bytes"),
            BitStringOption::Int => self.text("int"),
            BitStringOption::Float => self.text("float"),
            BitStringOption::Bits => self.text("bits"),
            BitStringOption::Utf8 => self.text("utf8"),
            BitStringOption::Utf16 => self.text("utf16"),
            BitStringOption::Utf32 => self.text("utf32"),
            BitStringOption::Signed => self.text("signed"),
            BitStringOption::Unsigned => self.text("unsigned"),
            BitStringOption::Big => self.text("big"),
            BitStringOption::Little => self.text("little"),
            BitStringOption::Native => self.text("native"),
            BitStringOption::Size(expr) => self
                .text("size(")
                .append(self.expr(expr))
                .append(self.text(")")),
            BitStringOption::Unit(n) => self.text(format!("unit({})", n)),
        }
    }

    // ========================================================================
    // Pattern printing
    // ========================================================================

    /// Print a pattern
    pub fn pattern(&self, p: &Pattern) -> Doc<'a> {
        match p {
            Pattern::Wildcard => self.text("_"),
            Pattern::Variable { name } => self.text(name.clone()),
            Pattern::Discard { name } => self.text(name.clone()),
            Pattern::Literal { value } => self.literal(value),
            Pattern::Constructor {
                module,
                name,
                arguments,
                with_spread,
            } => {
                let name_doc = if let Some(m) = module {
                    self.text(format!("{}.{}", m, name))
                } else {
                    self.text(name.clone())
                };

                if arguments.is_empty() && !with_spread {
                    name_doc
                } else {
                    let args_doc = self.join(
                        arguments.iter().map(|a| self.field_pattern(a)),
                        self.text(", "),
                    );
                    let with_spread_doc = if *with_spread {
                        if arguments.is_empty() {
                            self.text("..")
                        } else {
                            self.text(", ..")
                        }
                    } else {
                        self.nil()
                    };
                    name_doc.append(self.parens(args_doc.append(with_spread_doc)))
                }
            }
            Pattern::Tuple { elements } => {
                let elements_doc =
                    self.join(elements.iter().map(|e| self.pattern(e)), self.text(", "));
                self.text("#(").append(elements_doc).append(self.text(")"))
            }
            Pattern::List { elements, tail } => {
                if elements.is_empty() && tail.is_none() {
                    self.text("[]")
                } else {
                    let elements_doc =
                        self.join(elements.iter().map(|e| self.pattern(e)), self.text(", "));
                    let with_tail = if let Some(t) = tail {
                        elements_doc
                            .append(self.text(", .."))
                            .append(self.pattern(t))
                    } else {
                        elements_doc
                    };
                    self.brackets(with_tail)
                }
            }
            Pattern::Assignment { pattern, name } => self
                .pattern(pattern)
                .append(self.text(" as "))
                .append(self.text(name.clone())),
            Pattern::Concatenate {
                prefix,
                suffix_assignment,
            } => {
                let suffix_doc = if let Some(name) = suffix_assignment {
                    self.text(" <> ").append(self.text(name.clone()))
                } else {
                    self.nil()
                };
                self.text(format!("{:?}", prefix)).append(suffix_doc)
            }
            Pattern::BitString { segments } => {
                let segments_doc = self.join(
                    segments.iter().map(|s| self.bit_string_segment_pattern(s)),
                    self.text(", "),
                );
                self.text("<<").append(segments_doc).append(self.text(">>"))
            }
        }
    }

    /// Print a field pattern
    fn field_pattern(&self, f: &Field<Pattern>) -> Doc<'a> {
        match f {
            Field::Labelled { label, item } => self
                .text(label.clone())
                .append(self.text(": "))
                .append(self.pattern(item)),
            Field::Shorthand { name } => self.text(name.clone()),
            Field::Unlabelled { item } => self.pattern(item),
        }
    }

    /// Print a bit string segment for patterns
    fn bit_string_segment_pattern(&self, seg: &BitStringSegment<Pattern>) -> Doc<'a> {
        let value_doc = self.pattern(&seg.value);
        if seg.options.is_empty() {
            value_doc
        } else {
            let options_doc = self.join(
                seg.options.iter().map(|o| self.bit_string_option(o)),
                self.text("-"),
            );
            value_doc.append(self.text(":")).append(options_doc)
        }
    }
}

// ============================================================================
// Public API
// ============================================================================

/// Render a module to a string with default width
pub fn render_module(module: &ModuleIR) -> String {
    render_module_with_width(module, DEFAULT_WIDTH)
}

/// Render a module to a string with custom width
pub fn render_module_with_width(module: &ModuleIR, width: usize) -> String {
    let alloc = RcAllocator;
    let printer = GleamPrinter::new(&alloc);
    let doc = printer.module(module);
    let mut output = Vec::new();
    doc.render(width, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

/// Render an expression to a string
pub fn render_expr(expr: &Expr) -> String {
    render_expr_with_width(expr, DEFAULT_WIDTH)
}

/// Render an expression to a string with custom width
pub fn render_expr_with_width(expr: &Expr, width: usize) -> String {
    let alloc = RcAllocator;
    let printer = GleamPrinter::new(&alloc);
    let doc = printer.expr(expr);
    let mut output = Vec::new();
    doc.render(width, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

/// Render a type expression to a string
pub fn render_type_expr(type_expr: &TypeExpr) -> String {
    render_type_expr_with_width(type_expr, DEFAULT_WIDTH)
}

/// Render a type expression to a string with custom width
pub fn render_type_expr_with_width(type_expr: &TypeExpr, width: usize) -> String {
    let alloc = RcAllocator;
    let printer = GleamPrinter::new(&alloc);
    let doc = printer.type_expr(type_expr);
    let mut output = Vec::new();
    doc.render(width, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

/// Render a pattern to a string
pub fn render_pattern(pattern: &Pattern) -> String {
    render_pattern_with_width(pattern, DEFAULT_WIDTH)
}

/// Render a pattern to a string with custom width
pub fn render_pattern_with_width(pattern: &Pattern, width: usize) -> String {
    let alloc = RcAllocator;
    let printer = GleamPrinter::new(&alloc);
    let doc = printer.pattern(pattern);
    let mut output = Vec::new();
    doc.render(width, &mut output).unwrap();
    String::from_utf8(output).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_literal_bool() {
        let expr = Expr::Literal {
            value: Literal::Bool { value: true },
        };
        assert_eq!(render_expr(&expr), "True");
    }

    #[test]
    fn test_render_literal_int() {
        let expr = Expr::Literal {
            value: Literal::Int { value: 42 },
        };
        assert_eq!(render_expr(&expr), "42");
    }

    #[test]
    fn test_render_binary_op() {
        let expr = Expr::BinaryOp {
            op: BinaryOperator::AddInt,
            left: Box::new(Expr::Literal {
                value: Literal::Int { value: 1 },
            }),
            right: Box::new(Expr::Literal {
                value: Literal::Int { value: 2 },
            }),
        };
        assert_eq!(render_expr(&expr), "1 + 2");
    }

    #[test]
    fn test_render_tuple() {
        let expr = Expr::Tuple {
            elements: vec![
                Expr::Literal {
                    value: Literal::Int { value: 1 },
                },
                Expr::Literal {
                    value: Literal::String {
                        value: "hello".to_string(),
                    },
                },
            ],
        };
        assert_eq!(render_expr(&expr), "#(1, \"hello\")");
    }

    #[test]
    fn test_render_list() {
        let expr = Expr::List {
            elements: vec![
                Expr::Literal {
                    value: Literal::Int { value: 1 },
                },
                Expr::Literal {
                    value: Literal::Int { value: 2 },
                },
                Expr::Literal {
                    value: Literal::Int { value: 3 },
                },
            ],
            tail: None,
        };
        assert_eq!(render_expr(&expr), "[1, 2, 3]");
    }

    #[test]
    fn test_render_pattern_wildcard() {
        let pattern = Pattern::Wildcard;
        assert_eq!(render_pattern(&pattern), "_");
    }

    #[test]
    fn test_render_type_function() {
        let ty = TypeExpr::Function {
            parameters: vec![TypeExpr::Named {
                module: None,
                name: "Int".to_string(),
                parameters: vec![],
            }],
            return_type: Box::new(TypeExpr::Named {
                module: None,
                name: "String".to_string(),
                parameters: vec![],
            }),
        };
        assert_eq!(render_type_expr(&ty), "fn(Int) -> String");
    }
}
