//! Extract expected YAML bytes from the shared kit, independently of the YAML reader/writer.
//! Gherkin syntax, doc-string indentation and table escapes belong to morphir-gherkin.

use morphir_gherkin::{Background, Document, Scenario, Step, StepArgument};
use std::path::{Component, Path, PathBuf};

#[derive(Debug, PartialEq)]
enum ExpectedYaml {
    Inline(String),
    File(PathBuf),
}

#[derive(Debug, PartialEq)]
pub(super) struct Spelling {
    pub case: String,
    expected: ExpectedYaml,
}

impl Spelling {
    pub fn read(&self, kit: &Path) -> Result<String, String> {
        match &self.expected {
            ExpectedYaml::Inline(text) => Ok(text.clone()),
            ExpectedYaml::File(path) => std::fs::read_to_string(kit.join(path))
                .map_err(|error| format!("{}: {}: {error}", self.case, path.display())),
        }
    }
}

fn newline(mut text: String) -> String {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

#[derive(Clone, Copy)]
enum Substitution {
    StepText,
    DocString,
}

// Match the primary kit's placeholder grammar. Unknown names in doc strings
// are literal model text; step text must name an actual example column.
fn substitute(
    text: &str,
    bindings: Option<&[(&str, &str)]>,
    mode: Substitution,
) -> Result<String, String> {
    let Some(bindings) = bindings else {
        return Ok(text.to_owned());
    };
    let mut out = String::new();
    let mut remaining = text;
    while let Some((before, after)) = remaining.split_once('<') {
        out.push_str(before);
        match after.split_once('>').filter(|(name, _)| {
            !name.is_empty() && !name.contains(|c: char| c.is_whitespace() || c == '<')
        }) {
            Some((name, rest)) => {
                match bindings.iter().find(|(key, _)| *key == name) {
                    Some((_, value)) => out.push_str(value),
                    None => match mode {
                        Substitution::StepText => {
                            return Err(format!("missing outline parameter <{name}>"));
                        }
                        Substitution::DocString => {
                            out.push('<');
                            out.push_str(name);
                            out.push('>');
                        }
                    },
                }
                remaining = rest;
            }
            None => {
                out.push('<');
                remaining = after;
            }
        }
    }
    out.push_str(remaining);
    Ok(out)
}

fn expected(
    step: &Step,
    bindings: Option<&[(&str, &str)]>,
) -> Result<Option<ExpectedYaml>, String> {
    let text = substitute(&step.text, bindings, Substitution::StepText)?;
    let (profile, body) = match text.strip_prefix("its canonical ") {
        Some(rest) => rest
            .split_once(" spelling is")
            .ok_or("invalid canonical spelling step")?,
        None => return Ok(None),
    };
    if profile != "YAML" && profile != "text" {
        return Ok(None);
    }
    let body = if body == ":" {
        let Some(StepArgument::DocString(doc)) = &step.argument else {
            return Err("canonical spelling has no doc string".into());
        };
        if doc.content_type.as_deref() != Some(if profile == "YAML" { "yaml" } else { "text" }) {
            return Err("canonical doc-string content type differs from its profile".into());
        }
        substitute(&doc.body, bindings, Substitution::DocString)?
    } else {
        if step.argument.is_some() {
            return Err("inline spelling also has a step argument".into());
        }
        body.strip_prefix(' ')
            .ok_or("canonical spelling has no body")?
            .to_owned()
    };
    if profile == "YAML" {
        return Ok(Some(ExpectedYaml::Inline(newline(body))));
    }
    let relative = body
        .trim()
        .strip_prefix("spec/ir/mck/")
        .unwrap_or(body.trim());
    let path = Path::new(relative);
    if relative.is_empty()
        || relative.contains('\\')
        || !path.components().all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("canonical file must be relative to the kit".into());
    }
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("yaml" | "yml") => Ok(Some(ExpectedYaml::File(path.to_owned()))),
        Some("json" | "ion") => Ok(None),
        _ => Err("unknown canonical file profile".into()),
    }
}

fn append_steps(
    scenario: &Scenario,
    backgrounds: &[&Background],
    bindings: Option<&[(&str, &str)]>,
    label: String,
    out: &mut Vec<Spelling>,
) -> Result<(), String> {
    for step in backgrounds
        .iter()
        .flat_map(|background| &background.steps)
        .chain(&scenario.steps)
    {
        if let Some(expected) = expected(step, bindings)? {
            out.push(Spelling {
                case: label.clone(),
                expected,
            });
        }
    }
    Ok(())
}

fn scenario(
    scenario: &Scenario,
    backgrounds: &[&Background],
    out: &mut Vec<Spelling>,
) -> Result<(), String> {
    if scenario.examples.is_empty() {
        if scenario.keyword.contains("Outline") || scenario.keyword.contains("Template") {
            return Err("scenario outline has no examples".into());
        }
        return append_steps(scenario, backgrounds, None, scenario.name.clone(), out);
    }
    for (example_index, example) in scenario.examples.iter().enumerate() {
        let table = example
            .table
            .as_ref()
            .ok_or("outline has no example table")?;
        let headers = table.rows.first().ok_or("outline table has no headers")?;
        if headers.is_empty() || table.rows.len() < 2 {
            return Err("outline table has no example rows".into());
        }
        for (index, header) in headers.iter().enumerate() {
            if header.is_empty() || headers[..index].contains(header) {
                return Err("invalid outline table headers".into());
            }
        }
        for (row_index, row) in table.rows.iter().skip(1).enumerate() {
            if row.len() != headers.len() {
                return Err("outline row width differs from its headers".into());
            }
            let bindings: Vec<_> = headers
                .iter()
                .zip(row)
                .map(|(key, value)| (key.as_str(), value.as_str()))
                .collect();
            append_steps(
                scenario,
                backgrounds,
                Some(&bindings),
                format!(
                    "{} examples {} row {}",
                    scenario.name,
                    example_index + 1,
                    row_index + 1
                ),
                out,
            )?;
        }
    }
    Ok(())
}

pub(super) fn spellings(document: &Document) -> Result<Vec<Spelling>, String> {
    let feature = document.feature.as_ref().ok_or("kit file has no feature")?;
    let mut out = Vec::new();
    let backgrounds: Vec<_> = feature.background.iter().collect();
    for item in &feature.scenarios {
        scenario(item, &backgrounds, &mut out)?
    }
    for rule in &feature.rules {
        let backgrounds: Vec<_> = backgrounds
            .iter()
            .copied()
            .chain(rule.background.iter())
            .collect();
        for item in &rule.scenarios {
            scenario(item, &backgrounds, &mut out)?
        }
    }
    Ok(out)
}

#[test]
fn outlines_doc_strings_and_rules_preserve_expected_bytes() {
    let text = r#"Feature: Oracle
  Scenario Outline: examples-0001 Quoting and table escapes
    Then its canonical <format> spelling is <spelling>
    Examples:
      | format | spelling       |
      | YAML   | "a\|b"         |
      | JSON   | "ignored"      |
  Rule: Nested examples
    Scenario: examples-0002 Multiline strings
      Then its canonical YAML spelling is:
        """yaml
        note: |
          first

          last
        """
"#;
    let (document, _) = morphir_gherkin::read_str("oracle.feature", text).unwrap();
    let actual = spellings(&document).unwrap();
    assert_eq!(actual.len(), 2);
    assert_eq!(actual[0].read(Path::new("unused")).unwrap(), "\"a|b\"\n");
    assert_eq!(
        actual[1].read(Path::new("unused")).unwrap(),
        "note: |\n  first\n\n  last\n"
    );
}

#[test]
fn outline_doc_string_preserves_literal_angle_brackets() {
    let text = r#"Feature: Oracle
  Scenario Outline: examples-0001 Literal text
    Then its canonical YAML spelling is:
      """yaml
      message: <unknown>
      less: <
      empty: <>
      space: <two words>
      known: <value>
      """
    Examples:
      | value |
      | 42    |
"#;
    let (document, _) = morphir_gherkin::read_str("oracle.feature", text).unwrap();
    let actual = spellings(&document).unwrap();
    assert_eq!(
        actual[0].read(Path::new("unused")).unwrap(),
        "message: <unknown>\nless: <\nempty: <>\nspace: <two words>\nknown: 42\n"
    );
}

#[test]
fn referenced_yaml_is_an_explicit_file_expectation() {
    let text = "Feature: Oracle\n  Scenario: examples-0001 File\n    Then its canonical text spelling is spec/ir/mck/documents/example.yaml\n";
    let (document, _) = morphir_gherkin::read_str("oracle.feature", text).unwrap();
    let actual = spellings(&document).unwrap();
    assert_eq!(
        actual[0].expected,
        ExpectedYaml::File("documents/example.yaml".into())
    );
}

#[test]
fn malformed_expected_spelling_fails_instead_of_disappearing() {
    for text in [
        "Feature: Oracle\n  Scenario: examples-0001 Missing body\n    Then its canonical YAML spelling is:\n",
        "Feature: Oracle\n  Scenario Outline: examples-0001 Missing parameter\n    Then its canonical YAML spelling is <missing>\n    Examples:\n      | other |\n      | value |\n",
        "Feature: Oracle\n  Scenario: examples-0001 Escaping file\n    Then its canonical text spelling is ../example.yaml\n",
    ] {
        let (document, _) = morphir_gherkin::read_str("oracle.feature", text).unwrap();
        assert!(spellings(&document).is_err(), "{text}");
    }
}
