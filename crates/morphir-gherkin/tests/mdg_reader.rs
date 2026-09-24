use morphir_gherkin::{Format, StepArgument, StepKind, read_str};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn tag_names(tags: &[morphir_gherkin::Tag]) -> Vec<&str> {
    tags.iter().map(|tag| tag.name.as_str()).collect()
}

#[test]
fn an_mdg_file_reads_into_the_same_model() {
    let text = fixture("every-construct.feature.md");
    let (doc, source) = read_str("every-construct.feature.md", &text).unwrap();
    assert_eq!(doc.format, Format::Markdown);
    let feature = doc.feature.unwrap();
    assert_eq!(feature.name, "Every construct");
    assert_eq!(feature.tags[0].name, "feature-tag");
    assert_eq!(
        feature.description.fences().next().unwrap().body,
        "outer:\n  inner: 1\n"
    );
    assert_eq!(
        feature.background.as_ref().unwrap().steps[0].text,
        "a background step"
    );

    let rule = &feature.rules[0];
    assert_eq!(rule.name, "The first rule");
    let plain = &rule.scenarios[0];
    assert_eq!(plain.tags[0].name, "scenario-tag");
    assert_eq!(plain.steps.len(), 3);
    assert_eq!(plain.steps[1].kind, StepKind::When);
    let Some(StepArgument::Table(table)) = &plain.steps[0].argument else {
        panic!("a table")
    };
    assert_eq!(table.rows, vec![vec!["a", "b"], vec!["1", "2"]]);
    let Some(StepArgument::DocString(doc_string)) = &plain.steps[1].argument else {
        panic!("a doc string")
    };
    assert_eq!(doc_string.content_type.as_deref(), Some("ion"));
    assert_eq!(doc_string.body, "(ref 'morphir/SDK:basics#add')\n");
    assert_eq!(source.line_col(plain.steps[1].span.start).line, 30);

    let outline = &rule.scenarios[1];
    assert_eq!(outline.keyword, "Scenario Outline");
    assert_eq!(outline.examples[0].tags[0].name, "examples-tag");
    assert_eq!(
        outline.examples[0].table.as_ref().unwrap().rows,
        vec![vec!["x", "y"], vec!["1", "2"]]
    );
}

#[test]
fn every_node_has_a_span_and_a_position_in_the_file() {
    let text = fixture("every-construct.feature.md");
    let (doc, source) = read_str("every-construct.feature.md", &text).unwrap();
    let feature = doc.feature.unwrap();
    assert_eq!((feature.position.line, feature.position.col), (1, 1));
    assert_eq!(source.slice(feature.tags[0].span), "@feature-tag");
    assert_eq!(
        (feature.tags[0].position.line, feature.tags[0].position.col),
        (3, 2)
    );
    assert_eq!(feature.description.prose().next().unwrap().position.line, 5);
    assert_eq!(
        feature.description.fences().next().unwrap().position.line,
        7
    );
    assert_eq!(feature.background.as_ref().unwrap().position.line, 12);

    let rule = &feature.rules[0];
    assert_eq!(rule.position.line, 16);
    let only_prose = rule.description.prose().next().unwrap();
    assert_eq!(source.slice(only_prose.span).trim(), "Rule description.");

    let plain = &rule.scenarios[0];
    assert_eq!(plain.position.line, 20);
    assert!(plain.description.blocks.is_empty(), "{plain:?}");
    assert_eq!(plain.steps[0].keyword, "Given ");
    assert_eq!(plain.steps[0].position.line, 24);
    let Some(StepArgument::Table(table)) = &plain.steps[0].argument else {
        panic!("a table")
    };
    assert_eq!(table.position.line, 26);
    let Some(StepArgument::DocString(doc_string)) = &plain.steps[1].argument else {
        panic!("a doc string")
    };
    assert_eq!((doc_string.position.line, doc_string.position.col), (32, 3));
    assert_eq!(plain.steps[2].kind, StepKind::Then);

    let outline = &rule.scenarios[1];
    assert_eq!(outline.steps[0].text, "<x> is <y>");
    let examples = &outline.examples[0];
    assert_eq!(examples.name.as_deref(), Some("Some rows"));
    assert_eq!(examples.position.line, 42);
    assert_eq!(examples.table.as_ref().unwrap().position.line, 46);
    assert!(source.slice(feature.span).ends_with("| 1 | 2 |\n"));
}

#[test]
fn a_heading_without_a_gherkin_keyword_is_prose() {
    let text = "# Feature: F\n\n## Notes\n\nSome notes.\n\n## Scenario: S\n\n* Given a step\n";
    let (doc, _) = read_str("x.feature.md", text).unwrap();
    let feature = doc.feature.unwrap();
    assert_eq!(feature.scenarios.len(), 1);
    assert_eq!(feature.description.prose().count(), 2);
}

#[test]
fn a_file_without_a_feature_heading_has_no_feature() {
    let (doc, _) = read_str("x.feature.md", "# Just a document\n\nText.\n").unwrap();
    assert!(doc.feature.is_none());
}

#[test]
fn a_tag_line_after_a_heading_is_its_own_and_one_before_a_heading_leads_it() {
    let text = "# Feature: F\n\n`@own`\n\n## Scenario: A\n\n* Given a step\n\nSome prose.\n\n`@leading`\n\n## Scenario: B\n\n* Given a step\n";
    let (doc, _) = read_str("t.feature.md", text).unwrap();
    let feature = doc.feature.unwrap();
    assert_eq!(feature.tags[0].name, "own");
    assert!(feature.scenarios[0].tags.is_empty());
    assert_eq!(feature.scenarios[1].tags[0].name, "leading");
}

#[test]
fn and_and_but_take_the_kind_of_the_step_before() {
    let text = "# Feature: F\n\n## Scenario: S\n\n* When a\n* And b\n- But c\n* Then d\n* * e\n";
    let (doc, _) = read_str("k.feature.md", text).unwrap();
    let steps = &doc.feature.unwrap().scenarios[0].steps;
    let kinds: Vec<_> = steps.iter().map(|step| step.kind).collect();
    assert_eq!(
        kinds,
        [
            StepKind::When,
            StepKind::When,
            StepKind::When,
            StepKind::Then,
            StepKind::Then
        ]
    );
    assert_eq!(steps[2].keyword, "But ");
    assert_eq!(steps[4].keyword, "* ");
    assert_eq!(steps[4].text, "e");
}

#[test]
fn a_list_that_is_not_steps_is_description_prose() {
    // A change of bullet character starts a new list.
    let text = "# Feature: F\n\n## Scenario: S\n\n- a note\n- another note\n\n* Given a step\n";
    let (doc, _) = read_str("l.feature.md", text).unwrap();
    let scenario = &doc.feature.unwrap().scenarios[0];
    assert_eq!(scenario.description.prose().count(), 1);
    assert_eq!(scenario.steps.len(), 1);
}

// The tests below follow the upstream Markdown with Gherkin test data in cucumber/gherkin
// (testdata/good/*.feature.md, MIT licence).

#[test]
fn upstream_tag_lines_before_headings_lead_them() {
    let text = "\
`@feature_tag1` `@feature_tag2`
  `@feature_tag3`
# Feature: Minimal Scenario Outline

`@scenario_tag1` `@scenario_tag2`
  `@scenario_tag3`
## Scenario: minimalistic
* Given the minimalism

`@so_tag1`  `@so_tag2`
  `@so_tag3`
## Scenario Outline: minimalistic outline
* Given the <what>

`@ex_tag1` `@ex_tag2`
  `@ex_tag3`
### Examples:
  | what       |
  | ---------- |
  | minimalism |

`@ex_tag4` `@ex_tag5`
  `@ex_tag6`
### Examples:
  | what       |
  | ---------- |
  | more minimalism |

`@comment_tag1` #a comment
## Scenario: comments
  Given a comment

`@comment_tag#2` #a comment
## Scenario: hash in tags
  Given a comment is preceded by a space

`@rule_tag`
## Rule:

`@joined_tag3``@joined_tag4`
### Scenario: joined tags
  Given the @delimits tags
";
    let (doc, source) = read_str("tags.feature.md", text).unwrap();
    let feature = doc.feature.unwrap();
    assert_eq!(feature.name, "Minimal Scenario Outline");
    assert_eq!(
        tag_names(&feature.tags),
        ["feature_tag1", "feature_tag2", "feature_tag3"]
    );
    assert_eq!(
        (feature.tags[1].position.line, feature.tags[1].position.col),
        (1, 18)
    );
    assert_eq!(source.slice(feature.tags[2].span), "@feature_tag3");
    let scenarios = &feature.scenarios;
    assert_eq!(
        tag_names(&scenarios[0].tags),
        ["scenario_tag1", "scenario_tag2", "scenario_tag3"]
    );
    assert_eq!(scenarios[0].steps[0].text, "the minimalism");
    assert_eq!(
        tag_names(&scenarios[1].tags),
        ["so_tag1", "so_tag2", "so_tag3"]
    );
    assert_eq!(scenarios[1].steps[0].text, "the <what>");
    let examples = &scenarios[1].examples;
    assert_eq!(examples.len(), 2);
    assert_eq!(examples[0].name, None);
    assert_eq!(
        tag_names(&examples[0].tags),
        ["ex_tag1", "ex_tag2", "ex_tag3"]
    );
    assert_eq!(
        examples[0].table.as_ref().unwrap().rows,
        vec![vec!["what"], vec!["minimalism"]]
    );
    assert_eq!(
        tag_names(&examples[1].tags),
        ["ex_tag4", "ex_tag5", "ex_tag6"]
    );
    assert_eq!(
        examples[1].table.as_ref().unwrap().rows,
        vec![vec!["what"], vec!["more minimalism"]]
    );
    assert_eq!(tag_names(&scenarios[2].tags), ["comment_tag1"]);
    assert!(scenarios[2].steps.is_empty());
    assert_eq!(tag_names(&scenarios[3].tags), ["comment_tag#2"]);
    let rule = &feature.rules[0];
    assert_eq!(rule.name, "");
    assert_eq!(tag_names(&rule.tags), ["rule_tag"]);
    assert_eq!(
        tag_names(&rule.scenarios[0].tags),
        ["joined_tag3", "joined_tag4"]
    );
    assert_eq!(rule.scenarios[0].tags[1].position.col, 16);
}

#[test]
fn a_first_tag_line_right_above_a_heading_leads_that_heading() {
    let text = "# Feature: F\n\n`@above`\n## Scenario: A\n\n* Given a step\n\n## Scenario: B\n\n`@own`\n\n* Given a step\n";
    let (doc, _) = read_str("t.feature.md", text).unwrap();
    let feature = doc.feature.unwrap();
    assert!(feature.tags.is_empty());
    assert_eq!(tag_names(&feature.scenarios[0].tags), ["above"]);
    assert_eq!(tag_names(&feature.scenarios[1].tags), ["own"]);
}

#[test]
fn upstream_a_table_right_under_a_step_line_is_its_data_table() {
    let text = "\
## Feature: DataTables

### Scenario: minimalistic

* Given a simple data table
  | foo | bar |
  | --- | --- |
  | boz | boo |
";
    let (doc, _) = read_str("datatables.feature.md", text).unwrap();
    let step = &doc.feature.unwrap().scenarios[0].steps[0];
    assert_eq!(step.text, "a simple data table");
    let Some(StepArgument::Table(table)) = &step.argument else {
        panic!("a table: {step:?}")
    };
    assert_eq!(table.rows, vec![vec!["foo", "bar"], vec!["boz", "boo"]]);
    assert_eq!(table.position.line, 6);
}

#[test]
fn upstream_a_fence_right_after_a_step_list_is_the_last_steps_doc_string() {
    let text = "\
## Feature: DocString variations

### Scenario: minimalistic

* And a DocString with an implicitly escaped separator inside
````
```
````
";
    let (doc, _) = read_str("docstrings.feature.md", text).unwrap();
    let step = &doc.feature.unwrap().scenarios[0].steps[0];
    assert_eq!(step.keyword, "And ");
    assert_eq!(step.kind, StepKind::Given);
    let Some(StepArgument::DocString(doc_string)) = &step.argument else {
        panic!("a doc string: {step:?}")
    };
    assert_eq!(doc_string.content_type, None);
    assert_eq!(doc_string.body, "```\n");
    assert_eq!(doc_string.position.line, 6);
}

#[test]
fn upstream_steps_take_star_and_dash_lists_with_any_marker_spacing() {
    let text = "\
# Feature: Minimal

## Scenario: minimalistic

  *  Given the minimalism

# Scenario: Something about gravity
 - Given step one
 - When step two
 - Then step three

# The world is wet

Excepteur sint occaecat cupidatat non proident.
";
    let (doc, _) = read_str("minimal.feature.md", text).unwrap();
    let scenarios = doc.feature.unwrap().scenarios;
    assert_eq!(scenarios[0].steps[0].text, "the minimalism");
    let texts: Vec<_> = scenarios[1]
        .steps
        .iter()
        .map(|step| step.text.as_str())
        .collect();
    assert_eq!(texts, ["step one", "step two", "step three"]);
    assert_eq!(scenarios[1].steps[2].kind, StepKind::Then);
}
