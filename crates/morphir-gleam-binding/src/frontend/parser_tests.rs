use super::*;

#[test]
fn official_parser_preserves_declarations_and_imports() {
    let source = "//// Example module\nimport domain/model.{type Model as Entity, make as build} as model\n/// An identifier\npub opaque type Id { Id(value: Int) }\n/// A name\npub type Name = String\n/// Greeting\npub const greeting = \"hello\"\n/// Identity\npub fn identity(value input: Int) -> Int { input }";
    let module = parse_gleam("src/example.gleam", source).unwrap();
    assert_eq!(module.name, "src/example");
    assert_eq!(module.doc.as_deref(), Some("Example module"));
    assert_eq!(module.imports[0].types, [("Model".into(), "Entity".into())]);
    assert_eq!(module.imports[0].values, [("make".into(), "build".into())]);
    assert_eq!(module.types[0].constructor_access, Access::Private);
    assert_eq!(module.types[0].doc.as_deref(), Some("An identifier"));
    let TypeExpr::CustomType { variants } = &module.types[0].body else {
        panic!("custom type")
    };
    assert_eq!(variants[0].labels, [Some("value".into())]);
    assert_eq!(module.values[0].doc.as_deref(), Some("Greeting"));
    assert_eq!(module.values[1].params, ["input"]);
    assert_eq!(module.values[1].param_labels, [Some("value".into())]);
    assert_eq!(
        &source[module.values[1].span.start..module.values[1].span.end],
        "pub fn identity(value input: Int) -> Int { input }"
    );
}

#[test]
fn gleam_literals_use_official_escape_and_radix_rules() {
    let module = parse_gleam(
        "literals.gleam",
        r#"const hex: Int = 0xff
const binary: Int = 0b1010
const text: String = "line\n\u{1f600}\\"
const yes: Bool = True"#,
    )
    .unwrap();
    assert!(matches!(
        module.values[0].body,
        Expr::Literal {
            value: Literal::Int { value: 255 }
        }
    ));
    assert!(matches!(
        module.values[1].body,
        Expr::Literal {
            value: Literal::Int { value: 10 }
        }
    ));
    assert!(
        matches!(&module.values[2].body, Expr::Literal { value: Literal::String { value }} if value == "line\n😀\\")
    );
    assert!(matches!(
        module.values[3].body,
        Expr::Literal {
            value: Literal::Bool { value: true }
        }
    ));
}

#[test]
fn official_precedence_and_function_bodies_are_preserved() {
    let module = parse_gleam(
        "math.gleam",
        "pub fn calculate(x: Int) -> Int { let y = x + 2 * 3 y }",
    )
    .unwrap();
    let Expr::Block { statements } = &module.values[0].body else {
        panic!("block")
    };
    let Statement::Assignment { value, .. } = &statements[0] else {
        panic!("assignment")
    };
    let Expr::BinaryOp {
        op: BinaryOperator::AddInt,
        right,
        ..
    } = value.as_ref()
    else {
        panic!("addition")
    };
    assert!(matches!(
        right.as_ref(),
        Expr::BinaryOp {
            op: BinaryOperator::MultInt,
            ..
        }
    ));
}

#[test]
fn non_gleam_syntax_is_rejected() {
    for source in [
        "pub fn test() { if True { 1 } else { 2 } }",
        "pub type Record = { field: Int }",
        "pub type Unit = ()",
        "pub fn test() { (1 + 2) }",
        "pub fn test() { 1; }",
    ] {
        assert!(parse_gleam("invalid.gleam", source).is_err(), "{source}");
    }
}

#[test]
fn unsupported_values_keep_types_and_exact_locations() {
    let source = "pub type Id { Id(Int) }\npub fn guarded(x) { case x { n if n > 0 -> n _ -> 0 } }";
    let module = parse_gleam("guard.gleam", source).unwrap();
    assert_eq!(module.types.len(), 1);
    let Expr::Block { statements } = &module.values[0].body else {
        panic!("block")
    };
    let Statement::Expression(Expr::Unsupported { feature, span }) = &statements[0] else {
        panic!("unsupported guard")
    };
    assert!(feature.contains("guard"));
    assert_eq!(
        &source[span.start..span.end],
        "case x { n if n > 0 -> n _ -> 0 }"
    );
}

#[test]
fn external_function_is_not_replaced_by_a_fake_body() {
    let module = parse_gleam(
        "external.gleam",
        "@external(erlang, \"erlang\", \"abs\")\npub fn absolute(x: Int) -> Int",
    )
    .unwrap();
    assert!(
        matches!(&module.values[0].body, Expr::Unsupported { feature, .. } if feature.contains("external"))
    );
}

#[test]
fn incomplete_language_server_nodes_are_rejected() {
    for source in [
        "pub type Broken = module.",
        "pub fn broken(x) { case x }",
        "pub fn broken(x) { x. }",
        "pub fn broken(x) { case x { n if n > 0 -> x. _ -> 0 } }",
        "pub fn broken(x) { echo x. }",
    ] {
        assert!(parse_gleam("incomplete.gleam", source).is_err());
    }
}

#[test]
fn upstream_parse_error_has_a_location_and_readable_message() {
    let source = "//// 😀\npub fn broken( {";
    let error = parse_gleam("broken.gleam", source).unwrap_err();
    assert!(error.span.start > 8);
    assert!(error.message.contains("Syntax error"));
    assert_eq!(
        error
            .to_diagnostic("broken.gleam", source)
            .location
            .unwrap()
            .range
            .start
            .line,
        1
    );
}

#[test]
fn out_of_range_integers_are_explicitly_unsupported() {
    let module = parse_gleam("large.gleam", "const huge: Int = 9223372036854775808").unwrap();
    assert!(
        matches!(&module.values[0].body, Expr::Unsupported { feature, .. } if feature.contains("integer"))
    );
}

#[test]
fn function_without_body_requires_an_external_binding() {
    let source = "pub fn missing(x: Int) -> Int";
    let error = parse_gleam("missing.gleam", source).unwrap_err();
    assert!(error.message.contains("function body"));
    assert_eq!(&source[error.span], source);
}

#[test]
fn unexpected_eof_uses_the_official_diagnostic_span() {
    let source = "pub fn missing(";
    let error = parse_gleam("missing.gleam", source).unwrap_err();
    assert_eq!(error.span.start, source.len() - 1);
    assert_eq!(error.span.end, source.len() - 1);
    assert_eq!(error.source_snippet, None);
}
