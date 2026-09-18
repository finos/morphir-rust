//! Writing the Elm AST out as source, in the shape `elm-format` would leave it.
//!
//! The layout rules, in full:
//!
//! - Lines are 80 columns wide and indentation is four spaces.
//! - The module header is one line; a module doc comment follows after a blank
//!   line; the imports follow after another.
//! - Two blank lines sit before every declaration, whether or not it is the
//!   first one, and each declaration's doc comment sits directly above it.
//! - A type alias puts its body on the next line, indented; a custom type puts
//!   each constructor on its own line, indented, with `=` before the first and
//!   `|` before the rest.
//! - A record, tuple or function type is written on one line when it fits and
//!   broken with the separator leading each line when it does not.
//!
//! The printer has exactly one normal form: printing a module, reading the
//! result back and printing it again gives the same text, which is what
//! `tests/roundtrip.rs` pins.

use pretty::{DocAllocator, DocBuilder, RcAllocator};

use crate::ast::{Constructor, Exposed, Exposing, Field, Import, Module, TypeDecl, TypeExpr};

/// The column a line is broken at.
pub const WIDTH: usize = 80;

/// One level of indentation.
pub const INDENT: usize = 4;

type Doc<'a> = DocBuilder<'a, RcAllocator, ()>;

/// How tightly the context binds, which is what decides where parentheses go.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Prec {
    /// Anything may be written bare.
    Top,
    /// A function type needs parentheses: it is an argument's argument.
    Arg,
    /// A function type and an applied type constructor both need them.
    Atom,
}

/// The Elm source for a module.
pub fn print(module: &Module) -> String {
    let mut out = header(module);
    out.push('\n');

    if let Some(doc) = &module.doc {
        out.push('\n');
        out.push_str(&doc_comment(doc));
        out.push('\n');
    }

    if !module.imports.is_empty() {
        out.push('\n');
        for import in &module.imports {
            out.push_str(&import_line(import));
            out.push('\n');
        }
    }

    for declaration in &module.types {
        out.push_str("\n\n");
        if let Some(doc) = doc_of(declaration) {
            out.push_str(&doc_comment(doc));
            out.push('\n');
        }
        out.push_str(&declaration_lines(declaration));
        out.push('\n');
    }

    out
}

/// The type expression, on its own, as source. Exposed for callers that need a
/// type rendered outside a declaration (diagnostics, tests).
pub fn print_type(ty: &TypeExpr) -> String {
    let alloc = RcAllocator;
    render(type_doc(&alloc, ty, Prec::Top), WIDTH)
}

fn header(module: &Module) -> String {
    format!(
        "module {} exposing {}",
        module.name.join("."),
        exposing_list(&module.exposing)
    )
}

fn exposing_list(exposing: &Exposing) -> String {
    match exposing {
        Exposing::All => "(..)".to_string(),
        Exposing::Explicit(items) => {
            let items: Vec<String> = items
                .iter()
                .map(|item| match item {
                    Exposed::Type {
                        name,
                        constructors: true,
                    } => format!("{name}(..)"),
                    Exposed::Type { name, .. } => name.clone(),
                    Exposed::Value(name) => name.clone(),
                })
                .collect();
            format!("({})", items.join(", "))
        }
    }
}

fn import_line(import: &Import) -> String {
    let mut line = format!("import {}", import.module.join("."));
    if let Some(alias) = &import.alias {
        line.push_str(&format!(" as {alias}"));
    }
    if let Some(exposing) = &import.exposing {
        line.push_str(&format!(" exposing {}", exposing_list(exposing)));
    }
    line
}

/// A doc comment, written on one line when its text is one line.
fn doc_comment(text: &str) -> String {
    if text.contains('\n') {
        format!("{{-| {text}\n-}}")
    } else {
        format!("{{-| {text} -}}")
    }
}

fn doc_of(declaration: &TypeDecl) -> Option<&String> {
    match declaration {
        TypeDecl::Alias { doc, .. } => doc.as_ref(),
        TypeDecl::Custom { doc, .. } => doc.as_ref(),
    }
}

fn declaration_lines(declaration: &TypeDecl) -> String {
    let alloc = RcAllocator;
    match declaration {
        TypeDecl::Alias {
            name, params, body, ..
        } => {
            let head = format!("type alias {}{} =", name, params_suffix(params));
            let body = render(type_doc(&alloc, body, Prec::Top), WIDTH - INDENT);
            format!("{head}\n{}", indented(&body, INDENT))
        }
        TypeDecl::Custom {
            name,
            params,
            constructors,
            ..
        } => {
            let mut out = format!("type {}{}", name, params_suffix(params));
            for (index, constructor) in constructors.iter().enumerate() {
                let lead = if index == 0 { "=" } else { "|" };
                let body = render(constructor_doc(&alloc, constructor), WIDTH - INDENT - 2);
                out.push('\n');
                out.push_str(&prefixed(&body, &format!("{}{lead} ", " ".repeat(INDENT))));
            }
            out
        }
    }
}

fn params_suffix(params: &[String]) -> String {
    if params.is_empty() {
        String::new()
    } else {
        format!(" {}", params.join(" "))
    }
}

fn constructor_doc<'a>(alloc: &'a RcAllocator, constructor: &Constructor) -> Doc<'a> {
    let head = alloc.text(constructor.name.clone());
    if constructor.args.is_empty() {
        return head;
    }
    head.append(
        alloc
            .concat(
                constructor
                    .args
                    .iter()
                    .map(|arg| alloc.line().append(type_doc(alloc, arg, Prec::Atom))),
            )
            .nest(INDENT as isize),
    )
    .group()
}

fn type_doc<'a>(alloc: &'a RcAllocator, ty: &TypeExpr, prec: Prec) -> Doc<'a> {
    match ty {
        TypeExpr::Var { name, .. } => alloc.text(name.clone()),
        TypeExpr::Unit { .. } => alloc.text("()"),
        TypeExpr::Ref {
            module, name, args, ..
        } => {
            let spelled = if module.is_empty() {
                name.clone()
            } else {
                format!("{}.{name}", module.join("."))
            };
            let head = alloc.text(spelled);
            if args.is_empty() {
                return head;
            }
            let applied = head
                .append(
                    alloc
                        .concat(
                            args.iter()
                                .map(|arg| alloc.line().append(type_doc(alloc, arg, Prec::Atom))),
                        )
                        .nest(INDENT as isize),
                )
                .group();
            if prec >= Prec::Atom {
                parens(alloc, applied)
            } else {
                applied
            }
        }
        TypeExpr::Record { fields, .. } => record_doc(alloc, None, fields),
        TypeExpr::ExtensibleRecord { base, fields, .. } => record_doc(alloc, Some(base), fields),
        TypeExpr::Tuple { items, .. } => {
            if items.is_empty() {
                return alloc.text("()");
            }
            alloc
                .text("( ")
                .append(alloc.intersperse(
                    items.iter().map(|item| type_doc(alloc, item, Prec::Top)),
                    alloc.line_().append(", "),
                ))
                .append(alloc.line())
                .append(")")
                .group()
        }
        TypeExpr::Function { .. } => {
            let chain = function_chain(ty);
            let arrows = alloc
                .intersperse(
                    chain
                        .into_iter()
                        .map(|part| type_doc(alloc, part, Prec::Arg)),
                    alloc.line().append("-> "),
                )
                .group();
            if prec >= Prec::Arg {
                parens(alloc, arrows)
            } else {
                arrows
            }
        }
    }
}

/// `a -> b -> c` as `[a, b, c]`: the arrow is right-associative, so a nested
/// function on the *left* is a part of its own and keeps its parentheses.
fn function_chain(ty: &TypeExpr) -> Vec<&TypeExpr> {
    let mut parts = Vec::new();
    let mut current = ty;
    while let TypeExpr::Function { arg, result, .. } = current {
        parts.push(arg.as_ref());
        current = result.as_ref();
    }
    parts.push(current);
    parts
}

fn record_doc<'a>(alloc: &'a RcAllocator, base: Option<&str>, fields: &[Field]) -> Doc<'a> {
    let entries = alloc.intersperse(
        fields.iter().map(|field| {
            alloc
                .text(field.name.clone())
                .append(" : ")
                .append(type_doc(alloc, &field.ty, Prec::Top))
        }),
        alloc.line_().append(", "),
    );

    match base {
        None => {
            if fields.is_empty() {
                return alloc.text("{}");
            }
            alloc
                .text("{ ")
                .append(entries)
                .append(alloc.line())
                .append("}")
                .group()
        }
        Some(base) => alloc
            .text("{ ")
            .append(alloc.text(base.to_string()))
            .append(
                alloc
                    .line()
                    .append("| ")
                    .append(entries)
                    .nest(INDENT as isize),
            )
            .append(alloc.line())
            .append("}")
            .group(),
    }
}

fn parens<'a>(alloc: &'a RcAllocator, doc: Doc<'a>) -> Doc<'a> {
    alloc.text("(").append(doc).append(")")
}

fn render(doc: Doc<'_>, width: usize) -> String {
    let mut out = Vec::new();
    doc.render(width, &mut out)
        .expect("writing a document into a byte buffer cannot fail");
    String::from_utf8(out).expect("the printer only ever writes valid UTF-8")
}

/// Every line of `text` indented by `spaces`.
fn indented(text: &str, spaces: usize) -> String {
    prefixed(text, &" ".repeat(spaces))
}

/// The first line of `text` behind `prefix`, the rest behind as much blank
/// space, so that a broken document keeps its shape under a lead-in.
fn prefixed(text: &str, prefix: &str) -> String {
    let continuation = " ".repeat(prefix.chars().count());
    text.lines()
        .enumerate()
        .map(|(index, line)| {
            let lead = if index == 0 { prefix } else { &continuation };
            if line.is_empty() {
                String::new()
            } else {
                format!("{lead}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    const NONE: Span = Span { start: 0, end: 0 };

    fn reference(name: &str, args: Vec<TypeExpr>) -> TypeExpr {
        TypeExpr::Ref {
            module: Vec::new(),
            name: name.to_string(),
            args,
            span: NONE,
        }
    }

    fn field(name: &str, ty: TypeExpr) -> Field {
        Field {
            name: name.to_string(),
            ty,
        }
    }

    fn record(fields: Vec<Field>) -> TypeExpr {
        TypeExpr::Record { fields, span: NONE }
    }

    fn function(arg: TypeExpr, result: TypeExpr) -> TypeExpr {
        TypeExpr::Function {
            arg: Box::new(arg),
            result: Box::new(result),
            span: NONE,
        }
    }

    #[test]
    fn a_record_that_fits_stays_on_one_line() {
        assert_eq!(
            print_type(&record(vec![field("id", reference("Int", vec![]))])),
            "{ id : Int }"
        );
    }

    #[test]
    fn a_record_that_does_not_fit_leads_each_line_with_its_separator() {
        let long = reference("AVeryLongTypeNameIndeedYesTrulyVeryLong", vec![]);
        let printed = print_type(&record(vec![
            field("first", long.clone()),
            field("second", long.clone()),
            field("third", long),
        ]));

        assert_eq!(
            printed,
            "{ first : AVeryLongTypeNameIndeedYesTrulyVeryLong\n\
             , second : AVeryLongTypeNameIndeedYesTrulyVeryLong\n\
             , third : AVeryLongTypeNameIndeedYesTrulyVeryLong\n\
             }"
        );
    }

    #[test]
    fn an_extensible_record_names_its_base_before_the_bar() {
        assert_eq!(
            print_type(&TypeExpr::ExtensibleRecord {
                base: "r".to_string(),
                fields: vec![field("name", reference("String", vec![]))],
                span: NONE,
            }),
            "{ r | name : String }"
        );
    }

    #[test]
    fn a_tuple_is_spaced_inside_its_parentheses() {
        assert_eq!(
            print_type(&TypeExpr::Tuple {
                items: vec![reference("Int", vec![]), reference("String", vec![])],
                span: NONE,
            }),
            "( Int, String )"
        );
    }

    /// The arrow is right-associative, so only a function on the *left* of one
    /// needs parentheses.
    #[test]
    fn a_function_argument_that_is_itself_a_function_is_parenthesised() {
        let curried = function(
            reference("Int", vec![]),
            function(reference("String", vec![]), reference("Bool", vec![])),
        );
        assert_eq!(print_type(&curried), "Int -> String -> Bool");

        let higher_order = function(
            function(reference("Int", vec![]), reference("String", vec![])),
            reference("Bool", vec![]),
        );
        assert_eq!(print_type(&higher_order), "(Int -> String) -> Bool");
    }

    /// An applied type is an atom's argument, so it is parenthesised there —
    /// and a function always is.
    #[test]
    fn an_applied_type_in_argument_position_is_parenthesised() {
        let maybe_a = reference(
            "Maybe",
            vec![TypeExpr::Var {
                name: "a".to_string(),
                span: NONE,
            }],
        );
        assert_eq!(
            print_type(&reference("List", vec![maybe_a])),
            "List (Maybe a)"
        );
        assert_eq!(
            print_type(&reference(
                "List",
                vec![function(reference("Int", vec![]), reference("Int", vec![]))]
            )),
            "List (Int -> Int)"
        );
    }

    #[test]
    fn a_multi_line_doc_comment_closes_on_its_own_line() {
        assert_eq!(doc_comment("one line"), "{-| one line -}");
        assert_eq!(doc_comment("first\nsecond"), "{-| first\nsecond\n-}");
    }
}
