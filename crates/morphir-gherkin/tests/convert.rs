use morphir_gherkin::convert::to_feature_text;
use morphir_gherkin::{StepArgument, read_str};

#[test]
fn converted_text_reads_back_into_the_same_structure() {
    let text = std::fs::read_to_string(format!(
        "{}/tests/fixtures/every-construct.feature.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let (md, source) = read_str("every-construct.feature.md", &text).unwrap();
    let (feature_text, map) = to_feature_text(&md, &source);
    let (back, _) = read_str("every-construct.feature", &feature_text).unwrap();

    let (a, b) = (md.feature.as_ref().unwrap(), back.feature.as_ref().unwrap());
    assert_eq!(a.name, b.name);
    assert_eq!(
        a.tags.iter().map(|t| &t.name).collect::<Vec<_>>(),
        b.tags.iter().map(|t| &t.name).collect::<Vec<_>>()
    );
    assert_eq!(a.rules[0].scenarios.len(), b.rules[0].scenarios.len());
    let steps = |f: &morphir_gherkin::Feature| {
        f.rules[0].scenarios[0]
            .steps
            .iter()
            .map(|s| (s.keyword.clone(), s.text.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(steps(a), steps(b));
    let Some(StepArgument::DocString(d)) = &b.rules[0].scenarios[0].steps[1].argument else {
        panic!("a doc string")
    };
    assert_eq!(d.content_type.as_deref(), Some("ion"));
    assert_eq!(d.body, "(ref 'morphir/SDK:basics#add')\n");
    assert_eq!(
        b.description.fences().next().unwrap().body,
        "outer:\n  inner: 1\n"
    );
    assert_eq!(
        b.rules[0].scenarios[1].examples[0]
            .table
            .as_ref()
            .unwrap()
            .rows,
        vec![vec!["x", "y"], vec!["1", "2"]]
    );

    let step_line = b.rules[0].scenarios[0].steps[1].position.line;
    assert_eq!(
        map.source_line(step_line),
        a.rules[0].scenarios[0].steps[1].position.line
    );
}

#[test]
fn a_feature_file_converts_to_itself_apart_from_indent_and_table_padding() {
    // A plain .feature file has no preamble or notes, so converting it and reading the result
    // back gives the same names, tags and step texts throughout.
    let text = std::fs::read_to_string(format!(
        "{}/tests/fixtures/every-construct.feature",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let (doc, source) = read_str("every-construct.feature", &text).unwrap();
    let (feature_text, _map) = to_feature_text(&doc, &source);
    let (back, _) = read_str("every-construct.feature", &feature_text).unwrap();
    let (a, b) = (doc.feature.unwrap(), back.feature.unwrap());
    assert_eq!(a.name, b.name);
    assert_eq!(
        a.background.unwrap().steps[0].text,
        b.background.unwrap().steps[0].text
    );
    assert_eq!(a.rules[0].name, b.rules[0].name);
}

#[test]
fn an_implicit_feature_converts_to_a_valid_feature_file_with_an_empty_name() {
    // An MDG file without a Feature heading gives an implicit feature with keyword "" and
    // name "". The converter still has to write valid Gherkin: `Feature:` with nothing after it.
    let text = "# Just a title\n\nSome prose.\n\n## Scenario: S\n\n* Given a step\n";
    let (md, source) = read_str("implicit.feature.md", text).unwrap();
    assert_eq!(md.feature.as_ref().unwrap().keyword, "");
    let (feature_text, _map) = to_feature_text(&md, &source);
    assert!(
        feature_text
            .lines()
            .any(|line| line.trim_start() == "Feature:"),
        "{feature_text}"
    );
    let (back, _) = read_str("implicit.feature", &feature_text).unwrap();
    let feature = back.feature.unwrap();
    assert_eq!(feature.name, "");
    assert_eq!(feature.scenarios[0].name, "S");
}

#[test]
fn preamble_and_notes_are_written_as_comments_with_correct_source_lines() {
    // Ruling R9: Document.preamble, Step.notes and Examples.notes have no structural home in
    // plain Gherkin, so the converter writes them as `#` comment lines. Each comment line still
    // maps back to the source line it came from.
    let text = "\
# A title

Some preamble prose.

`@feature-tag`
## Feature: Notes everywhere

## Scenario Outline: S

* Given a step

A note about the step.

### Examples: rows

| x |
| - |
| 1 |

Why these rows.
";
    let (md, source) = read_str("notes.feature.md", text).unwrap();
    let (feature_text, map) = to_feature_text(&md, &source);

    let lines: Vec<&str> = feature_text.lines().collect();

    // The preamble comes above the tags and the Feature line.
    let feature_line_index = lines
        .iter()
        .position(|line| line.trim_start() == "Feature: Notes everywhere")
        .unwrap();
    let preamble_lines: Vec<&str> = lines[..feature_line_index]
        .iter()
        .filter(|line| !line.trim().is_empty())
        .copied()
        .collect();
    assert!(
        preamble_lines
            .iter()
            .any(|line| line.trim_start() == "# # A title"),
        "{preamble_lines:?}"
    );
    assert!(
        preamble_lines
            .iter()
            .any(|line| line.trim_start() == "# Some preamble prose."),
        "{preamble_lines:?}"
    );

    // The step's note comes right after the step, as a comment indented one level deeper.
    let step_index = lines
        .iter()
        .position(|line| line.trim_start() == "Given a step")
        .unwrap();
    let note_index = lines[step_index + 1..]
        .iter()
        .position(|line| line.trim_start() == "# A note about the step.")
        .map(|i| i + step_index + 1)
        .expect("the step's note as a comment");
    let step_indent = lines[step_index].len() - lines[step_index].trim_start().len();
    let note_indent = lines[note_index].len() - lines[note_index].trim_start().len();
    assert!(note_indent > step_indent, "{note_indent} > {step_indent}");

    // The examples' notes come right after the table, indented like the table.
    let table_index = lines
        .iter()
        .position(|line| line.trim_start() == "| x |")
        .unwrap();
    let table_indent = lines[table_index].len() - lines[table_index].trim_start().len();
    let examples_note_index = lines[table_index + 1..]
        .iter()
        .position(|line| line.trim_start() == "# Why these rows.")
        .map(|i| i + table_index + 1)
        .expect("the examples' note as a comment");
    let examples_note_indent =
        lines[examples_note_index].len() - lines[examples_note_index].trim_start().len();
    assert_eq!(examples_note_indent, table_indent);

    // Every comment line maps back to its own line in the source.
    let feature = md.feature.as_ref().unwrap();
    let step_notes = &feature.scenarios[0].steps[0].notes;
    let note_prose = step_notes.prose().next().unwrap();
    assert_eq!(
        map.source_line(note_index + 1),
        note_prose.position.line,
        "note line should map back to its source line"
    );
    let examples_notes = &feature.scenarios[0].examples[0].notes;
    let examples_note_prose = examples_notes.prose().next().unwrap();
    assert_eq!(
        map.source_line(examples_note_index + 1),
        examples_note_prose.position.line
    );

    let preamble_prose = md.preamble.prose().next().unwrap();
    let title_index = lines
        .iter()
        .position(|line| line.trim_start() == "# # A title")
        .unwrap();
    assert_eq!(
        map.source_line(title_index + 1),
        preamble_prose.position.line
    );
}

#[test]
fn a_description_line_that_looks_like_a_comment_is_escaped_and_reads_back() {
    // The reference Gherkin parsers treat any line whose first character (after indent) is `#`
    // as a comment. Our own reader's boundary search walks upward over such lines and can even
    // silently steal a trailing one into the next heading's lead-in. So the converter escapes a
    // description line that would start with `#` by adding a leading backslash: `\#`. CommonMark
    // reads `\#` back as a literal `#`, so a converted prose line reads back to the same text.
    let text = "\
# Feature: Hash lines
A line.
#warning right before the next heading

## Scenario: S

* Given a step
";
    let (md, source) = read_str("hash.feature.md", text).unwrap();
    let (feature_text, _map) = to_feature_text(&md, &source);

    // No description line in the converted text starts with a bare `#`: every such line was
    // escaped, so plain Gherkin never reads it as a comment.
    for line in feature_text.lines() {
        let trimmed = line.trim_start();
        assert!(
            !trimmed.starts_with('#'),
            "unescaped comment-like line in the converted text: {line:?}"
        );
    }
    assert!(
        feature_text.contains("\\#warning right before the next heading"),
        "{feature_text}"
    );

    let (back, _) = read_str("hash.feature", &feature_text).unwrap();
    let feature = back.feature.unwrap();
    let markdown: Vec<String> = feature
        .description
        .prose()
        .map(|p| p.markdown.clone())
        .collect();
    assert!(
        markdown
            .iter()
            .any(|m| m.contains("warning right before the next heading")),
        "{markdown:?}"
    );
    // The escaped line was not swallowed into the Scenario's own lead-in: the description still
    // holds it, and the Scenario keeps no tags or extra content from it.
    assert_eq!(feature.scenarios[0].name, "S");
}

#[test]
fn a_fence_line_that_looks_like_a_comment_is_written_byte_identical() {
    // R10: a description fence's body is written byte for byte, never escaped. This crate's own
    // reader always finds the fence's closing line first (it does not walk into the fence body),
    // so the body round-trips exactly, even though the `#`-led line inside it is not portable to
    // the reference Gherkin parsers (see the module documentation).
    let text = "\
# Feature: Fenced hash

```yaml
# a comment inside the fence
key: value
```

## Scenario: S

* Given a step
";
    let (md, source) = read_str("fence-hash.feature.md", text).unwrap();
    let (feature_text, _map) = to_feature_text(&md, &source);
    assert!(
        feature_text.contains("# a comment inside the fence"),
        "{feature_text}"
    );
    assert!(
        !feature_text.contains("\\# a comment inside the fence"),
        "the fence body must not be escaped: {feature_text}"
    );

    let (back, _) = read_str("fence-hash.feature", &feature_text).unwrap();
    let original_body = md
        .feature
        .as_ref()
        .unwrap()
        .description
        .fences()
        .next()
        .unwrap()
        .body
        .clone();
    let converted_body = back
        .feature
        .unwrap()
        .description
        .fences()
        .next()
        .unwrap()
        .body
        .clone();
    assert_eq!(converted_body, original_body);
    assert_eq!(converted_body, "# a comment inside the fence\nkey: value\n");
}

#[test]
fn table_cells_with_backslash_pipe_and_newline_round_trip() {
    // Critical 2: gherkin 0.16 allows only `\|`, `\\` and `\n` as backslash sequences inside a
    // table cell. A cell's own backslash, pipe and line break must be escaped the same way, and a
    // `<x>` placeholder must stay untouched, so the table reads back to the same rows.
    let text = "Feature: F\n  Scenario Outline: S\n    Given <x>\n\n    Examples:\n      \
                | x |\n      | a\\|b\\\\c\\nd |\n      | <x> |\n";
    let (doc, source) = read_str("cells.feature", text).unwrap();
    let original = doc.feature.as_ref().unwrap().scenarios[0].examples[0]
        .table
        .as_ref()
        .unwrap()
        .rows
        .clone();
    assert_eq!(original[1][0], "a|b\\c\nd");
    assert_eq!(original[2][0], "<x>");

    let (feature_text, _map) = to_feature_text(&doc, &source);
    let (back, _) = read_str("cells.feature", &feature_text).unwrap();
    let converted = back.feature.unwrap().scenarios[0].examples[0]
        .table
        .as_ref()
        .unwrap()
        .rows
        .clone();
    assert_eq!(converted, original);
}

#[test]
fn a_doc_string_with_no_conflicting_line_uses_triple_quotes() {
    let text = "\
# Feature: F

## Scenario: S

* Given a step

  ```ion
  plain content
  ```
";
    let (md, source) = read_str("plain.feature.md", text).unwrap();
    let (feature_text, _map) = to_feature_text(&md, &source);
    assert!(feature_text.contains("\"\"\"ion"), "{feature_text}");
    let (back, _) = read_str("plain.feature", &feature_text).unwrap();
    let Some(StepArgument::DocString(d)) = &back.feature.unwrap().scenarios[0].steps[0].argument
    else {
        panic!("a doc string")
    };
    assert_eq!(d.content_type.as_deref(), Some("ion"));
    assert_eq!(d.body, "plain content\n");
}

#[test]
fn a_doc_string_with_a_triple_quote_line_uses_backticks() {
    let text = "\
# Feature: F

## Scenario: S

* Given a step

  ```text
  \"\"\"looks like closing
  more text
  ```
";
    let (md, source) = read_str("backtick.feature.md", text).unwrap();
    let (feature_text, _map) = to_feature_text(&md, &source);
    assert!(feature_text.contains("```text"), "{feature_text}");
    let (back, _) = read_str("backtick.feature", &feature_text).unwrap();
    let Some(StepArgument::DocString(d)) = &back.feature.unwrap().scenarios[0].steps[0].argument
    else {
        panic!("a doc string")
    };
    assert_eq!(d.content_type.as_deref(), Some("text"));
    assert_eq!(d.body, "\"\"\"looks like closing\nmore text\n");
}

#[test]
fn a_doc_string_with_both_delimiters_stays_triple_quoted_and_escapes_them() {
    let text = "\
# Feature: F

## Scenario: S

* Given a step

  ```text
  \"\"\"looks like closing
  ```also looks like closing
  plain
  ```
";
    let (md, source) = read_str("both.feature.md", text).unwrap();
    let original_body = md.feature.as_ref().unwrap().scenarios[0].steps[0]
        .argument
        .as_ref()
        .map(|a| match a {
            StepArgument::DocString(d) => d.body.clone(),
            StepArgument::Table(_) => panic!("a doc string"),
        })
        .unwrap();
    let (feature_text, _map) = to_feature_text(&md, &source);
    assert!(feature_text.contains("\"\"\"text"), "{feature_text}");
    assert!(
        feature_text.contains("\\\"\\\"\\\"looks like closing"),
        "{feature_text}"
    );
    let (back, _) = read_str("both.feature", &feature_text).unwrap();
    let Some(StepArgument::DocString(d)) = &back.feature.unwrap().scenarios[0].steps[0].argument
    else {
        panic!("a doc string")
    };
    assert_eq!(d.content_type.as_deref(), Some("text"));
    assert_eq!(d.body, original_body);
}

#[test]
fn a_fence_info_string_round_trips_exactly() {
    // Important 4: FenceInfo does not preserve the order of bare words and `key=value` options,
    // so the converter writes `raw`, the fence's original info text, back verbatim.
    let text = "\
# Feature: F

Some prose.

```yaml key=value extra
key: value
```

## Scenario: S

* Given a step
";
    let (md, source) = read_str("info.feature.md", text).unwrap();
    let fence = md
        .feature
        .as_ref()
        .unwrap()
        .description
        .fences()
        .next()
        .unwrap();
    assert_eq!(fence.info.raw, "yaml key=value extra");

    let (feature_text, _map) = to_feature_text(&md, &source);
    assert!(
        feature_text.contains("```yaml key=value extra"),
        "{feature_text}"
    );

    let (back, _) = read_str("info.feature", &feature_text).unwrap();
    let back_feature = back.feature.unwrap();
    let back_fence = back_feature.description.fences().next().unwrap();
    assert_eq!(back_fence.info.raw, "yaml key=value extra");
}
