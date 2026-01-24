//! Gleam parser - converts Gleam source code to Morphir IR
//!
//! This implementation uses `chumsky` for parsing, following patterns from
//! the official Gleam implementations (glance).

use chumsky::input::{IterInput, ValueInput};
use chumsky::prelude::*;
use chumsky::span::SimpleSpan;

use crate::frontend::ast::{
    Access, CaseBranch, Expr, Field, Literal, ModuleIR, Pattern, TypeDef, TypeExpr, ValueDef,
    Variant,
};
use crate::frontend::errors::{ParseError, to_parse_error};
use crate::frontend::lexer::{Token, tokenize};

// ============================================================================
// Parser Combinators (Chumsky 0.12 API)
// ============================================================================

/// Statement enum
#[derive(Debug, Clone)]
enum Statement {
    TypeDef(TypeDef),
    ValueDef(ValueDef),
}

/// Main module parser
fn module_parser<'src, I>()
-> impl Parser<'src, I, ModuleIR, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    // Parse module-level statements
    let stmt = statement_parser().then_ignore(just(Token::Semicolon).or_not());

    stmt.repeated().collect::<Vec<_>>().map(|stmts| {
        let mut types = Vec::new();
        let mut values = Vec::new();

        for stmt in stmts {
            match stmt {
                Statement::TypeDef(td) => types.push(td),
                Statement::ValueDef(vd) => values.push(vd),
            }
        }

        ModuleIR {
            name: String::new(), // Will be set from path
            doc: None,
            types,
            values,
        }
    })
}

/// Statement parser
fn statement_parser<'src, I>()
-> impl Parser<'src, I, Statement, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    type_def_parser()
        .map(Statement::TypeDef)
        .or(value_def_parser().map(Statement::ValueDef))
}

/// Type definition parser
fn type_def_parser<'src, I>()
-> impl Parser<'src, I, TypeDef, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    let access = just(Token::Pub)
        .to(Access::Public)
        .or_not()
        .map(|opt| opt.unwrap_or(Access::Private));

    let type_params = identifier_parser()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .or_not()
        .map(|opt: Option<Vec<String>>| opt.unwrap_or_default());

    access
        .then_ignore(just(Token::Type))
        .then(type_identifier_parser())
        .then(type_params)
        .then_ignore(just(Token::LBrace))
        .then(custom_type_body_parser())
        .then_ignore(just(Token::RBrace))
        .map(|(((access, name), params), body)| TypeDef {
            name,
            params,
            body,
            access,
        })
}

/// Custom type body parser (variants)
/// In Gleam, variants are listed consecutively without separators (whitespace is skipped)
fn custom_type_body_parser<'src, I>()
-> impl Parser<'src, I, TypeExpr, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    variant_parser()
        .repeated()
        .at_least(1)
        .collect::<Vec<_>>()
        .map(|variants| TypeExpr::CustomType { variants })
}

/// Variant parser
fn variant_parser<'src, I>()
-> impl Parser<'src, I, Variant, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    let variant_fields = type_expr_parser()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<_>>()
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .or_not()
        .map(|opt: Option<Vec<TypeExpr>>| opt.unwrap_or_default());

    type_identifier_parser()
        .then(variant_fields)
        .map(|(name, fields)| Variant { name, fields })
}

/// Value definition parser
fn value_def_parser<'src, I>()
-> impl Parser<'src, I, ValueDef, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    let access = just(Token::Pub)
        .to(Access::Public)
        .or_not()
        .map(|opt| opt.unwrap_or(Access::Private));

    let params = identifier_parser()
        .separated_by(just(Token::Comma))
        .allow_trailing()
        .collect::<Vec<String>>()
        .delimited_by(just(Token::LParen), just(Token::RParen))
        .or_not()
        .map(|opt: Option<Vec<String>>| opt.unwrap_or_default());

    let type_ann = just(Token::Colon).ignore_then(type_expr_parser()).or_not();

    access
        .then_ignore(just(Token::Fn))
        .then(identifier_parser())
        .then(params)
        .then(type_ann)
        .then_ignore(just(Token::LBrace))
        .then(expr_parser())
        .then_ignore(just(Token::RBrace))
        .map(
            |((((access, name), _params), type_annotation), body)| ValueDef {
                name,
                type_annotation,
                body,
                access,
            },
        )
}

/// Expression parser
fn expr_parser<'src, I>() -> impl Parser<'src, I, Expr, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    recursive(|expr| {
        // Literals
        let literal = literal_parser().map(|lit| Expr::Literal { value: lit });

        // Variables
        let variable = identifier_parser().map(|name| Expr::Variable { name });

        // Constructors (uppercase identifiers)
        let constructor =
            type_identifier_parser().map(|name| Expr::Constructor { module: None, name });

        // Tuples: #(expr, expr, ...)
        let tuple = just(Token::Hash)
            .ignore_then(
                expr.clone()
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .map(|elements| Expr::Tuple { elements });

        // Records: { field: expr, ... }
        let record_field = identifier_parser()
            .then_ignore(just(Token::Colon))
            .then(expr.clone());

        let record = record_field
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .map(|fields| Expr::Record { fields });

        // Lambda: fn(param) { expr }
        let lambda = just(Token::Fn)
            .ignore_then(identifier_parser().delimited_by(just(Token::LParen), just(Token::RParen)))
            .then_ignore(just(Token::LBrace))
            .then(expr.clone())
            .then_ignore(just(Token::RBrace))
            .map(|(param, body)| Expr::Lambda {
                params: vec![param],
                body: Box::new(body),
            });

        // Let binding: let name = expr { expr }
        let let_binding = just(Token::Let)
            .ignore_then(identifier_parser())
            .then_ignore(just(Token::Equals))
            .then(expr.clone())
            .then(
                expr.clone()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace)),
            )
            .map(|((name, value), body)| Expr::Let {
                name,
                value: Box::new(value),
                body: Box::new(body),
            });

        // If expression: if expr { expr } else { expr }
        let if_expr = just(Token::If)
            .ignore_then(expr.clone())
            .then(
                expr.clone()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace)),
            )
            .then_ignore(just(Token::Else))
            .then(
                expr.clone()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace)),
            )
            .map(|((condition, then_branch), else_branch)| Expr::If {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            });

        // Case expression: case expr { pattern -> expr, ... }
        let case_branch = pattern_parser()
            .then_ignore(just(Token::Arrow))
            .then(expr.clone())
            .map(|(pattern, body)| CaseBranch { pattern, body });

        let case_expr = just(Token::Case)
            .ignore_then(expr.clone())
            .then(
                case_branch
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::LBrace), just(Token::RBrace)),
            )
            .map(|(subject, clauses)| Expr::Case {
                subjects: vec![subject],
                clauses,
            });

        // Parenthesized expression
        let paren_expr = expr
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen));

        // Primary expressions (atoms)
        let atom = literal
            .or(variable)
            .or(constructor)
            .or(tuple)
            .or(record)
            .or(paren_expr);

        // Field access: expr.field
        let field_access = just(Token::Dot).ignore_then(identifier_parser());

        // Function application: expr(expr)
        let application = expr
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen));

        // Build up expressions with postfix operators
        let postfix = atom.foldl(
            field_access
                .map(PostfixOp::Field)
                .or(application.map(PostfixOp::Apply))
                .repeated(),
            |lhs, op| match op {
                PostfixOp::Field(label) => Expr::FieldAccess {
                    container: Box::new(lhs),
                    label,
                },
                PostfixOp::Apply(argument) => Expr::Apply {
                    function: Box::new(lhs),
                    arguments: vec![Field::Unlabelled { item: argument }],
                },
            },
        );

        // All expression forms
        postfix
            .or(lambda)
            .or(let_binding)
            .or(if_expr)
            .or(case_expr)
            .boxed()
    })
}

/// Helper enum for postfix operators
#[derive(Clone)]
enum PostfixOp {
    Field(String),
    Apply(Expr),
}

/// Type expression parser
fn type_expr_parser<'src, I>()
-> impl Parser<'src, I, TypeExpr, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    recursive(|type_expr| {
        // Type variable (lowercase)
        let type_var = identifier_parser().map(|name| TypeExpr::Variable { name });

        // Unit type: ()
        let unit = just(Token::LParen)
            .then(just(Token::RParen))
            .to(TypeExpr::Unit);

        // Tuple type: #(Type, Type, ...)
        let tuple_type = just(Token::Hash)
            .ignore_then(
                type_expr
                    .clone()
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .map(|elements| TypeExpr::Tuple { elements });

        // Record type: { field: Type, ... }
        let record_field = identifier_parser()
            .then_ignore(just(Token::Colon))
            .then(type_expr.clone());

        let record_type = record_field
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LBrace), just(Token::RBrace))
            .map(|fields| TypeExpr::Record { fields });

        // Reference type: TypeName or TypeName(Type, ...)
        let type_args = type_expr
            .clone()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .or_not()
            .map(|opt: Option<Vec<TypeExpr>>| opt.unwrap_or_default());

        let ref_type = type_identifier_parser()
            .then(type_args)
            .map(|(name, parameters)| TypeExpr::Named {
                module: None,
                name,
                parameters,
            });

        // Parenthesized type
        let paren_type = type_expr
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen));

        // Primary types
        let primary = unit
            .or(tuple_type)
            .or(record_type)
            .or(ref_type)
            .or(type_var)
            .or(paren_type);

        // Function types: Type -> Type (right-associative)
        // Note: For simplicity, we treat `A -> B` as fn(A) -> B
        primary
            .foldl(
                just(Token::Arrow).ignore_then(type_expr).repeated(),
                |param, return_type| TypeExpr::Function {
                    parameters: vec![param],
                    return_type: Box::new(return_type),
                },
            )
            .boxed()
    })
}

/// Pattern parser
fn pattern_parser<'src, I>()
-> impl Parser<'src, I, Pattern, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    recursive(|pattern| {
        // Wildcard: _
        let wildcard = just(Token::Underscore).to(Pattern::Wildcard);

        // Variable pattern (lowercase)
        let var_pattern = identifier_parser().map(|name| Pattern::Variable { name });

        // Literal pattern
        let lit_pattern = literal_parser().map(|lit| Pattern::Literal { value: lit });

        // Tuple pattern: #(pattern, ...)
        let tuple_pattern = just(Token::Hash)
            .ignore_then(
                pattern
                    .clone()
                    .separated_by(just(Token::Comma))
                    .allow_trailing()
                    .collect::<Vec<_>>()
                    .delimited_by(just(Token::LParen), just(Token::RParen)),
            )
            .map(|elements| Pattern::Tuple { elements });

        // Constructor pattern: ConstructorName or ConstructorName(pattern, ...)
        let constructor_args = pattern
            .clone()
            .separated_by(just(Token::Comma))
            .allow_trailing()
            .collect::<Vec<_>>()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .or_not()
            .map(|opt: Option<Vec<Pattern>>| opt.unwrap_or_default());

        let constructor_pattern = type_identifier_parser()
            .then(constructor_args)
            .map(|(name, args)| Pattern::Constructor {
                module: None,
                name,
                arguments: args.into_iter().map(|p| Field::Unlabelled { item: p }).collect(),
                with_spread: false,
            });

        // Parenthesized pattern
        let paren_pattern = pattern
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen));

        wildcard
            .or(lit_pattern)
            .or(tuple_pattern)
            .or(constructor_pattern)
            .or(var_pattern)
            .or(paren_pattern)
            .boxed()
    })
}

/// Literal parser
fn literal_parser<'src, I>()
-> impl Parser<'src, I, Literal, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    select! {
        Token::True => Literal::Bool { value: true },
        Token::False => Literal::Bool { value: false },
        Token::Int(i) => Literal::Int { value: i },
        Token::Float(f) => Literal::Float { value: f },
        Token::String(s) => Literal::String { value: s },
    }
    .labelled("literal")
}

/// Identifier parser (lowercase)
fn identifier_parser<'src, I>()
-> impl Parser<'src, I, String, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    select! {
        Token::Ident(name) => name,
    }
    .labelled("identifier")
}

/// Type identifier parser (uppercase)
fn type_identifier_parser<'src, I>()
-> impl Parser<'src, I, String, extra::Err<Rich<'src, Token, SimpleSpan>>>
where
    I: ValueInput<'src, Token = Token, Span = SimpleSpan>,
{
    select! {
        Token::TypeIdent(name) => name,
    }
    .labelled("type identifier")
}

// ============================================================================
// Public API
// ============================================================================

/// Parse Gleam source code into ModuleIR
#[allow(clippy::result_large_err)]
pub fn parse_gleam(path: &str, source: &str) -> Result<ModuleIR, ParseError> {
    // Tokenize
    let tokens = tokenize(source);

    if tokens.is_empty() {
        return Ok(ModuleIR {
            name: extract_module_name(path),
            doc: None,
            types: vec![],
            values: vec![],
        });
    }

    // Create end-of-input span
    let eoi = SimpleSpan::from(source.len()..source.len());

    // Create IterInput from tokens for parsing (handles (Token, Span) tuples)
    let input = IterInput::new(tokens.into_iter(), eoi);

    // Parse using chumsky 0.12 API
    let parser = module_parser();
    match parser.parse(input).into_result() {
        Ok(mut module) => {
            // Set module name from path
            module.name = extract_module_name(path);
            Ok(module)
        }
        Err(errors) => {
            // Return first error (could be enhanced to return multiple)
            if let Some(err) = errors.first() {
                Err(to_parse_error(err, source))
            } else {
                Err(ParseError {
                    message: "Unknown parse error".to_string(),
                    span: 0..0,
                    expected: vec![],
                    found: None,
                    hint: None,
                    source_snippet: None,
                })
            }
        }
    }
}

/// Extract module name from file path
fn extract_module_name(path: &str) -> String {
    path.trim_end_matches(".gleam").replace(['/', '\\'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::lexer::{Token, tokenize};

    #[test]
    fn test_tokenize_simple() {
        let source = "pub fn hello() { \"world\" }";
        let tokens = tokenize(source);

        assert!(!tokens.is_empty());
        assert_eq!(tokens[0].0, Token::Pub);
        assert_eq!(tokens[1].0, Token::Fn);
    }

    #[test]
    fn test_tokenize_literals() {
        let source = "42 True False \"hello\" 3.14";
        let tokens = tokenize(source);

        assert!(tokens.iter().any(|(t, _)| matches!(t, Token::Int(_))));
        assert!(tokens.iter().any(|(t, _)| t == &Token::True));
        assert!(tokens.iter().any(|(t, _)| t == &Token::False));
        assert!(tokens.iter().any(|(t, _)| matches!(t, Token::String(_))));
        assert!(tokens.iter().any(|(t, _)| matches!(t, Token::Float(_))));
    }

    #[test]
    fn test_tokenize_identifiers() {
        let source = "hello world MyType";
        let tokens = tokenize(source);

        assert!(tokens.iter().any(|(t, _)| matches!(t, Token::Ident(_))));
        assert!(tokens.iter().any(|(t, _)| matches!(t, Token::TypeIdent(_))));
    }

    #[test]
    fn test_parse_simple_function() {
        let source = r#"
pub fn hello() {
    "world"
}
"#;

        let result = parse_gleam("example.gleam", source);
        assert!(result.is_ok());
        let module = result.unwrap();
        assert_eq!(module.values.len(), 1);
        assert_eq!(module.values[0].name, "hello");
    }

    #[test]
    fn test_parse_type_definition() {
        let source = r#"
pub type Maybe {
    Just
    Nothing
}
"#;

        let result = parse_gleam("example.gleam", source);
        assert!(result.is_ok());
        let module = result.unwrap();
        assert_eq!(module.types.len(), 1);
        assert_eq!(module.types[0].name, "Maybe");
    }

    #[test]
    fn test_parse_empty_module() {
        let source = "";
        let result = parse_gleam("example.gleam", source);
        assert!(result.is_ok());
        let module = result.unwrap();
        assert_eq!(module.types.len(), 0);
        assert_eq!(module.values.len(), 0);
    }
}
