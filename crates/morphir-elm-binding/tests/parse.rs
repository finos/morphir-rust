use morphir_elm_binding::frontend::parse::{parse, syntax_errors};

const TYPES: &str = "module My.Types exposing (Id)\n\ntype alias Id =\n    String\n";

#[test]
fn parses_a_module_and_reports_no_errors() {
    let parsed = parse(TYPES);
    assert_eq!(parsed.tree.root_node().kind(), "file");
    assert!(syntax_errors(&parsed, TYPES).is_empty());
}

#[test]
fn reports_error_nodes_with_spans() {
    let source = "module Example exposing (add)\n\nadd =\n";
    let parsed = parse(source);
    let errors = syntax_errors(&parsed, source);
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].span.start, source.find("add =").unwrap());
    assert!(errors[0].message.contains("syntax error"));
}
