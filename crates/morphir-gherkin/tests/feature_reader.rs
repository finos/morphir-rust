use std::path::Path;

use morphir_gherkin::{
    DescriptionBlock, Format, ReadError, StepArgument, StepKind, read_document, read_str,
};

fn fixture_path(name: &str) -> String {
    format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(fixture_path(name)).unwrap()
}

#[test]
fn a_description_fence_keeps_every_line_and_blank_line() {
    let text = fixture("indented.feature");
    let (doc, source) = read_str("indented.feature", &text).unwrap();
    assert_eq!(doc.format, Format::Feature);
    let feature = doc.feature.unwrap();
    let fence = feature.description.fences().next().unwrap();
    assert_eq!(fence.body, "outer:\n  inner: 1\n\n  after_blank: 2\n");
    assert_eq!(fence.position.line, 5);
    assert!(source.slice(fence.span).starts_with("  ```yaml morphir"));
    assert!(matches!(
        feature.description.blocks[0],
        DescriptionBlock::Prose(_)
    ));
    assert_eq!(feature.description.prose().count(), 2);
}

#[test]
fn a_doc_string_gives_its_content_type_and_a_dedented_body() {
    let text = fixture("indented.feature");
    let (doc, _) = read_str("indented.feature", &text).unwrap();
    let scenario = &doc.feature.unwrap().scenarios[0];
    let Some(StepArgument::DocString(ion)) = &scenario.steps[0].argument else {
        panic!("a doc string")
    };
    assert_eq!(ion.content_type.as_deref(), Some("ion"));
    assert_eq!(ion.body, "(ref 'morphir/SDK:basics#add')\n  (deeper)\n");
    assert_eq!(ion.position.line, 17);
    let Some(StepArgument::DocString(yaml)) = &scenario.steps[1].argument else {
        panic!("a doc string")
    };
    assert_eq!(yaml.content_type.as_deref(), Some("yaml"));
    assert_eq!(yaml.body, "Reference:\n  name: morphir/SDK:basics#add\n");
}

#[test]
fn namespaced_tags_keep_their_value_and_span() {
    let text = fixture("indented.feature");
    let (doc, source) = read_str("indented.feature", &text).unwrap();
    let feature = doc.feature.unwrap();
    assert_eq!(feature.tags[0].namespaced(), Some(("node", "Value")));
    assert_eq!(source.slice(feature.tags[1].span), "@version:4");
    assert_eq!(feature.scenarios[0].tags[0].name, "spelling");
}

#[test]
fn every_construct_is_read_with_positions() {
    let text = fixture("every-construct.feature");
    let (doc, _) = read_str("every-construct.feature", &text).unwrap();
    let feature = doc.feature.unwrap();
    assert_eq!(feature.background.as_ref().unwrap().steps.len(), 1);
    let rule = &feature.rules[0];
    assert_eq!(rule.name, "The first rule");
    assert_eq!(
        rule.background.as_ref().unwrap().steps[0].text,
        "a rule background step"
    );
    let plain = &rule.scenarios[0];
    assert_eq!(plain.tags[0].name, "scenario-tag");
    assert_eq!(plain.steps[2].kind, StepKind::When);
    let Some(StepArgument::Table(table)) = &plain.steps[0].argument else {
        panic!("a table")
    };
    assert_eq!(table.rows, vec![vec!["a", "b"], vec!["1", "2"]]);
    let outline = &rule.scenarios[1];
    assert_eq!(outline.keyword, "Scenario Outline");
    assert_eq!(outline.examples[0].tags[0].name, "examples-tag");
    assert_eq!(outline.examples[0].table.as_ref().unwrap().rows.len(), 2);
    assert_eq!(plain.position.line, 15);
}

#[test]
fn a_syntax_error_names_the_file_and_line() {
    let err = read_str(
        "bad.feature",
        "Feature: x\n  Scenario: y\n    Given a\n    not a step\n",
    )
    .unwrap_err();
    assert!(err.to_string().starts_with("bad.feature:"), "{err}");
}

#[test]
fn an_unknown_extension_is_refused() {
    assert!(read_str("notes.txt", "Feature: x\n").is_err());
}

#[test]
fn each_description_holds_only_its_own_lines() {
    let text = fixture("every-construct.feature");
    let (doc, source) = read_str("every-construct.feature", &text).unwrap();
    let feature = doc.feature.unwrap();
    let only_prose = |description: &morphir_gherkin::Description| -> String {
        assert_eq!(description.blocks.len(), 1, "{description:?}");
        let prose = description.prose().next().unwrap();
        source.slice(prose.span).trim().to_owned()
    };
    assert_eq!(only_prose(&feature.description), "Feature description.");
    assert_eq!(
        only_prose(&feature.rules[0].description),
        "Rule description."
    );
    assert!(
        feature
            .background
            .as_ref()
            .unwrap()
            .description
            .blocks
            .is_empty()
    );
    for scenario in &feature.rules[0].scenarios {
        assert!(scenario.description.blocks.is_empty(), "{scenario:?}");
    }
}

#[test]
fn tags_on_several_lines_stay_out_of_the_description_above() {
    let text = "\
Feature: Tags on several lines
  Feature prose.

  @first @second
  # A comment between tag lines.
  @third
  Scenario: Tagged
    Given a step
";
    let (doc, source) = read_str("tags.feature", text).unwrap();
    let feature = doc.feature.unwrap();
    assert_eq!(feature.description.blocks.len(), 1);
    let prose = feature.description.prose().next().unwrap();
    assert_eq!(source.slice(prose.span).trim(), "Feature prose.");
    let tags = &feature.scenarios[0].tags;
    let names: Vec<_> = tags.iter().map(|tag| tag.name.as_str()).collect();
    assert_eq!(names, ["first", "second", "third"]);
    assert_eq!(source.slice(tags[1].span), "@second");
    assert_eq!(
        (tags[0].position.line, tags[0].position.col),
        (4, 3),
        "{tags:?}"
    );
    assert_eq!(tags[1].position.col, 10);
    assert_eq!(tags[2].position.line, 6);
}

#[test]
fn a_scenario_without_steps_keeps_its_description_away_from_the_next_scenario() {
    let text = "\
Feature: Scenarios without steps
  Scenario: First
    First prose.

  @next
  Scenario: Second
    Second prose.
";
    let (doc, source) = read_str("empty.feature", text).unwrap();
    let scenarios = doc.feature.unwrap().scenarios;
    assert_eq!(scenarios.len(), 2);
    for (scenario, expected) in scenarios.iter().zip(["First prose.", "Second prose."]) {
        assert_eq!(scenario.description.blocks.len(), 1, "{scenario:?}");
        let prose = scenario.description.prose().next().unwrap();
        assert_eq!(source.slice(prose.span).trim(), expected);
    }
}

#[test]
fn a_doc_string_unescapes_its_own_delimiter_and_keeps_the_other() {
    let text = r#"Feature: Doc strings
  Scenario: Escapes
    Given a quoted doc string
      """
      a \"\"\" inside
        deeper
    shallow
      """
    And a backtick doc string
      # A comment between a step and its doc string.
      ```json
      \`\`\` and """
      ```
    Then an empty doc string
      """
      """
"#;
    let (doc, source) = read_str("doc-strings.feature", text).unwrap();
    let steps = &doc.feature.unwrap().scenarios[0].steps;
    let doc_string = |index: usize| match &steps[index].argument {
        Some(StepArgument::DocString(doc_string)) => doc_string.clone(),
        other => panic!("a doc string, not {other:?}"),
    };

    let quoted = doc_string(0);
    assert_eq!(quoted.content_type, None);
    assert_eq!(quoted.body, "a \"\"\" inside\n  deeper\nshallow\n");
    assert_eq!((quoted.position.line, quoted.position.col), (4, 7));
    assert!(source.slice(quoted.span).trim_end().ends_with("\"\"\""));

    let backtick = doc_string(1);
    assert_eq!(backtick.content_type.as_deref(), Some("json"));
    assert_eq!(backtick.body, "``` and \"\"\"\n");
    assert_eq!(backtick.position.line, 11);

    let empty = doc_string(2);
    assert_eq!(empty.content_type, None);
    assert_eq!(empty.body, "");
}

#[test]
fn read_document_reads_a_file_and_names_it_in_errors() {
    let path = fixture_path("every-construct.feature");
    let (doc, _) = read_document(Path::new(&path)).unwrap();
    assert_eq!(doc.path, Path::new(&path));
    assert_eq!(doc.feature.unwrap().name, "Every construct");

    let missing = fixture_path("missing.feature");
    let err = read_document(Path::new(&missing)).unwrap_err();
    assert!(matches!(err, ReadError::Io { .. }), "{err}");
    assert!(err.to_string().starts_with(&missing), "{err}");
}

#[test]
fn a_file_with_only_comments_has_no_feature() {
    let (doc, _) = read_str("empty.feature", "# language: en\n\n# Nothing here yet.\n").unwrap();
    assert_eq!(doc.format, Format::Feature);
    assert!(doc.feature.is_none());
}

#[test]
fn a_file_without_a_final_line_break_keeps_spans_inside_the_text() {
    let text = "@tag\nFeature: No final newline\n  Scenario: Last\n    Last prose.";
    let (doc, source) = read_str("short.feature", text).unwrap();
    let feature = doc.feature.unwrap();
    assert!(feature.span.end <= text.len());
    let scenario = &feature.scenarios[0];
    let prose = scenario.description.prose().next().unwrap();
    assert_eq!(source.slice(prose.span).trim(), "Last prose.");
    assert_eq!(source.slice(feature.tags[0].span), "@tag");
}
