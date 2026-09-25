use morphir_gherkin::visit::{Node, Visitor, Walk, walk};
use morphir_gherkin::{NodePath, Segment, StepArgument, read_str};

fn doc() -> morphir_gherkin::Document {
    let text = std::fs::read_to_string(format!(
        "{}/tests/fixtures/every-construct.feature",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    read_str("every-construct.feature", &text).unwrap().0
}

struct Paths(Vec<String>);
impl<'d> Visitor<'d> for Paths {
    fn enter(&mut self, path: &NodePath, _node: Node<'d>) -> Walk {
        self.0.push(path.to_string());
        Walk::Children
    }
}

#[test]
fn the_visitor_walks_in_source_order() {
    let doc = doc();
    let mut paths = Paths(Vec::new());
    walk(&doc, &mut paths);
    assert_eq!(paths.0[0], "feature");
    assert_eq!(paths.0[1], "feature/prose[0]");
    assert_eq!(paths.0[2], "feature/background");
    assert!(
        paths
            .0
            .contains(&"feature/rule[0]/scenario[0]/step[3]".to_owned())
    );
    assert!(
        paths
            .0
            .contains(&"feature/rule[0]/scenario[1]/examples[0]".to_owned())
    );
    let rule = paths.0.iter().position(|p| p == "feature/rule[0]").unwrap();
    let scenario = paths
        .0
        .iter()
        .position(|p| p == "feature/rule[0]/scenario[0]")
        .unwrap();
    assert!(rule < scenario);
}

#[test]
fn skip_does_not_enter_children() {
    struct SkipRules(Vec<String>);
    impl<'d> Visitor<'d> for SkipRules {
        fn enter(&mut self, path: &NodePath, node: Node<'d>) -> Walk {
            self.0.push(path.to_string());
            if matches!(node, Node::Rule(_)) {
                Walk::Skip
            } else {
                Walk::Children
            }
        }
    }
    let doc = doc();
    let mut visitor = SkipRules(Vec::new());
    walk(&doc, &mut visitor);
    assert!(!visitor.0.iter().any(|p| p.starts_with("feature/rule[0]/")));
}

#[test]
fn a_path_finds_its_node_and_an_offset_finds_its_path() {
    let doc = doc();
    let path: NodePath = "feature/rule[0]/scenario[0]/step[1]".parse().unwrap();
    let Some(Node::Step(step)) = doc.node(&path) else {
        panic!("a step")
    };
    assert_eq!(step.text, "a step");
    assert_eq!(doc.at(step.span.start + 2), Some(path));
    assert_eq!(doc.node(&"feature/rule[5]".parse().unwrap()), None);
}

#[test]
fn the_cursor_moves_between_parent_children_and_siblings() {
    let doc = doc();
    let cursor = doc.cursor().unwrap();
    assert_eq!(cursor.path().to_string(), "feature");
    let rule = cursor
        .children()
        .into_iter()
        .find(|c| matches!(c.node(), Node::Rule(_)))
        .unwrap();
    let first = rule
        .children()
        .into_iter()
        .find(|c| matches!(c.node(), Node::Scenario(_)))
        .unwrap();
    let second = first.next_sibling().unwrap();
    assert_eq!(second.path().to_string(), "feature/rule[0]/scenario[1]");
    assert_eq!(second.previous_sibling().unwrap().path(), first.path());
    assert_eq!(second.parent().unwrap().path(), rule.path());
}

#[test]
fn cursor_is_none_for_a_comment_only_document() {
    let (doc, _) = read_str("comment-only.feature", "# just a comment\n").unwrap();
    assert!(doc.feature.is_none());
    assert!(doc.cursor().is_none());
}

// R6: the preamble, a step's notes, and an examples block's notes are also navigable.

#[test]
fn a_preamble_parses_and_walks_before_the_feature() {
    let text = "\
# A title

Some intro.

```yaml
key: value
```

# Feature: F

## Scenario: S

* Given a step
";
    let (doc, _) = read_str("preamble.feature.md", text).unwrap();
    // "# A title" and "Some intro." are two prose blocks, then the fence.
    assert_eq!(doc.preamble.blocks.len(), 3);

    let mut paths = Paths(Vec::new());
    walk(&doc, &mut paths);
    assert_eq!(paths.0[0], "preamble/prose[0]");
    assert_eq!(paths.0[1], "preamble/prose[1]");
    assert_eq!(paths.0[2], "preamble/fence[0]");
    let feature_at = paths.0.iter().position(|p| p == "feature").unwrap();
    assert_eq!(feature_at, 3);

    let prose_path: NodePath = "preamble/prose[0]".parse().unwrap();
    let Some(Node::Prose(prose)) = doc.node(&prose_path) else {
        panic!("a prose block")
    };
    assert_eq!(doc.at(prose.span.start + 1), Some(prose_path));

    let fence_path: NodePath = "preamble/fence[0]".parse().unwrap();
    let Some(Node::Fence(fence)) = doc.node(&fence_path) else {
        panic!("a fence block")
    };
    assert_eq!(fence.body, "key: value\n");
}

#[test]
fn a_document_with_no_feature_still_walks_its_preamble() {
    let text = "Just a title.\n\nJust some text.\n";
    let (doc, _) = read_str("no-feature.feature.md", text).unwrap();
    assert!(doc.feature.is_none());

    let mut paths = Paths(Vec::new());
    walk(&doc, &mut paths);
    assert_eq!(paths.0, vec!["preamble/prose[0]", "preamble/prose[1]"]);
}

#[test]
fn preamble_cursor_is_none_without_a_preamble_and_moves_between_blocks_when_present() {
    let plain = doc();
    assert!(plain.preamble.blocks.is_empty());
    assert!(plain.preamble_cursor().is_none());

    let text = "\
Intro text.

```text
fence body
```

More text.

# Feature: F

## Scenario: S

* Given a step
";
    let (doc, _) = read_str("preamble-cursor.feature.md", text).unwrap();
    let first = doc.preamble_cursor().unwrap();
    assert_eq!(first.path().to_string(), "preamble/prose[0]");
    assert!(first.parent().is_none());

    let second = first.next_sibling().unwrap();
    assert_eq!(second.path().to_string(), "preamble/fence[0]");
    let third = second.next_sibling().unwrap();
    assert_eq!(third.path().to_string(), "preamble/prose[1]");
    assert!(third.next_sibling().is_none());
    assert_eq!(third.previous_sibling().unwrap().path(), second.path());
    assert!(third.parent().is_none());
}

#[test]
fn a_steps_notes_are_children_in_source_order() {
    let text = "\
# Feature: F

## Scenario: S

* Given a step
* When a step
* Then a step

A note paragraph.

```text
a note fence
```
";
    let (doc, source) = read_str("step-notes.feature.md", text).unwrap();
    let step_path: NodePath = "feature/scenario[0]/step[2]".parse().unwrap();
    let Some(Node::Step(step)) = doc.node(&step_path) else {
        panic!("a step")
    };
    assert_eq!(step.text, "a step");

    let prose_path = step_path.push(Segment::Prose(0));
    let Some(Node::Prose(prose)) = doc.node(&prose_path) else {
        panic!("a prose note")
    };
    assert_eq!(source.slice(prose.span).trim(), "A note paragraph.");

    let fence_path = step_path.push(Segment::Fence(0));
    let Some(Node::Fence(fence)) = doc.node(&fence_path) else {
        panic!("a fence note")
    };
    assert_eq!(fence.body, "a note fence\n");

    let mut paths = Paths(Vec::new());
    walk(&doc, &mut paths);
    let at = paths
        .0
        .iter()
        .position(|p| p == "feature/scenario[0]/step[2]")
        .unwrap();
    assert_eq!(paths.0[at + 1], "feature/scenario[0]/step[2]/prose[0]");
    assert_eq!(paths.0[at + 2], "feature/scenario[0]/step[2]/fence[0]");
}

#[test]
fn an_examples_blocks_notes_continue_its_description_indexes() {
    let text = "\
# Feature: F

## Scenario Outline: O

* Given <x>

### Examples: Some rows

First description line.

Second description line.

| x |
| - |
| 1 |

Notes line.
";
    let (doc, source) = read_str("examples-notes.feature.md", text).unwrap();
    let examples_path: NodePath = "feature/scenario[0]/examples[0]".parse().unwrap();
    let Some(Node::Examples(examples)) = doc.node(&examples_path) else {
        panic!("an examples block")
    };
    assert_eq!(examples.description.prose().count(), 2);
    assert_eq!(examples.notes.prose().count(), 1);

    let first = examples_path.push(Segment::Prose(0));
    assert!(matches!(doc.node(&first), Some(Node::Prose(_))));
    let second = examples_path.push(Segment::Prose(1));
    assert!(matches!(doc.node(&second), Some(Node::Prose(_))));

    let notes_path = examples_path.push(Segment::Prose(2));
    let Some(Node::Prose(notes)) = doc.node(&notes_path) else {
        panic!("the notes prose, at prose[2]")
    };
    assert_eq!(source.slice(notes.span).trim(), "Notes line.");
}

#[test]
fn from_segments_builds_a_path_from_a_slice() {
    let path = NodePath::from_segments(&[Segment::Feature, Segment::Rule(0), Segment::Scenario(1)]);
    assert_eq!(path.to_string(), "feature/rule[0]/scenario[1]");
    assert_eq!(path, "feature/rule[0]/scenario[1]".parse().unwrap());
}

// R7 (fix round 1): `Document::at` finds the node that owns an offset inside a step's argument
// (its doc string or table) or its notes, rather than the enclosing scenario or background, whose
// span already covers that offset. A blank gap between two steps still resolves to the enclosing
// scenario, since it belongs to no step.

fn step0_path() -> NodePath {
    "feature/scenario[0]/step[0]".parse().unwrap()
}

#[test]
fn at_an_offset_in_a_feature_steps_doc_string_resolves_to_the_step() {
    let text = "\
Feature: F
  Scenario: S
    Given a step
      \"\"\"
      doc string body
      \"\"\"
    When another step
";
    let (doc, _) = read_str("doc-string.feature", text).unwrap();
    let step = &doc.feature.as_ref().unwrap().scenarios[0].steps[0];
    let Some(StepArgument::DocString(doc_string)) = &step.argument else {
        panic!("a doc string")
    };
    let offset = doc_string.span.start + doc_string.body.find("doc string body").unwrap();
    assert_eq!(doc.at(offset), Some(step0_path()));
}

#[test]
fn at_an_offset_in_an_mdg_steps_doc_string_resolves_to_the_step() {
    let text = "\
# Feature: F

## Scenario: S

* Given a step

  ```text
  doc string body
  ```

* When another step
";
    let (doc, _) = read_str("doc-string.feature.md", text).unwrap();
    let step = &doc.feature.as_ref().unwrap().scenarios[0].steps[0];
    let Some(StepArgument::DocString(doc_string)) = &step.argument else {
        panic!("a doc string")
    };
    let offset = doc_string.span.start + doc_string.body.find("doc string body").unwrap();
    assert_eq!(doc.at(offset), Some(step0_path()));
}

#[test]
fn at_an_offset_in_a_feature_steps_table_resolves_to_the_step() {
    let text = "\
Feature: F
  Scenario: S
    Given a step
      | a | b |
      | 1 | 2 |
    When another step
";
    let (doc, _) = read_str("table.feature", text).unwrap();
    let step = &doc.feature.as_ref().unwrap().scenarios[0].steps[0];
    let Some(StepArgument::Table(table)) = &step.argument else {
        panic!("a table")
    };
    assert_eq!(doc.at(table.span.start + 2), Some(step0_path()));
}

#[test]
fn at_an_offset_in_an_mdg_steps_table_resolves_to_the_step() {
    let text = "\
# Feature: F

## Scenario: S

* Given a step

  | a | b |
  | - | - |
  | 1 | 2 |

* When another step
";
    let (doc, _) = read_str("table.feature.md", text).unwrap();
    let step = &doc.feature.as_ref().unwrap().scenarios[0].steps[0];
    let Some(StepArgument::Table(table)) = &step.argument else {
        panic!("a table")
    };
    assert_eq!(doc.at(table.span.start + 2), Some(step0_path()));
}

#[test]
fn at_an_offset_in_a_steps_note_resolves_to_the_note_block() {
    let text = "\
# Feature: F

## Scenario: S

* Given a step

A note paragraph.

* When another step
";
    let (doc, source) = read_str("step-note.feature.md", text).unwrap();
    let step = &doc.feature.as_ref().unwrap().scenarios[0].steps[0];
    let note = step.notes.prose().next().unwrap();
    assert_eq!(source.slice(note.span).trim(), "A note paragraph.");
    assert_eq!(
        doc.at(note.span.start + 1),
        Some(step0_path().push(Segment::Prose(0)))
    );
}

#[test]
fn at_an_offset_in_the_gap_between_two_feature_steps_resolves_to_the_scenario() {
    let text = "Feature: F\n  Scenario: S\n    Given a step\n\n    When another step\n";
    let (doc, source) = read_str("gap.feature", text).unwrap();
    // Line 4 (1-based) is the blank line between the two steps.
    let offset = source.line_start(4);
    assert_eq!(doc.at(offset), Some("feature/scenario[0]".parse().unwrap()));
}

// This shape does not distinctly exist for `.feature.md`: the Markdown reader gives a step list
// item's span through the blank line that follows it, up to the start of the next item, so a
// blank line between two step items already belongs to the first step's own span, not to a gap
// the scenario owns.
