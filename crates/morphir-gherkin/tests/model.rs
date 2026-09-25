use morphir_gherkin::{NodePath, Segment, SourceText, Span};

#[test]
fn source_text_maps_offsets_to_one_based_lines_and_columns() {
    let text = SourceText::new("ab\ncd\n\nef");
    assert_eq!(text.line_col(0).line, 1);
    assert_eq!(text.line_col(4).line, 2);
    assert_eq!(text.line_col(4).col, 2);
    assert_eq!(text.line_col(7).line, 4);
    assert_eq!(text.slice(Span { start: 3, end: 5 }), "cd");
    assert_eq!(text.line_start(3), 6);
}

#[test]
fn node_paths_print_and_parse() {
    let path = NodePath::feature()
        .push(Segment::Rule(1))
        .push(Segment::Scenario(2))
        .push(Segment::Step(3));
    assert_eq!(path.to_string(), "feature/rule[1]/scenario[2]/step[3]");
    assert_eq!(
        "feature/rule[1]/scenario[2]/step[3]"
            .parse::<NodePath>()
            .unwrap(),
        path
    );
    assert_eq!(
        path.parent().unwrap().to_string(),
        "feature/rule[1]/scenario[2]"
    );
    assert!("feature/nope[1]".parse::<NodePath>().is_err());
}
