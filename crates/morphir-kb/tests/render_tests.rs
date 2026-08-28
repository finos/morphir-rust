//! Tests for the `render` module: the `kb list` / `kb show` / `kb search`
//! text layouts and the JSON shapes, which must match the Scala CLI's ujson
//! output byte for byte (field names, key order, indent-2 layout).

use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use morphir_kb::render;
use morphir_kb::scaffold::{self, ScaffoldResult};
use morphir_okf::model::Kb;
use morphir_okf::paths;
use tempfile::TempDir;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 28).unwrap()
}

fn fixture() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let kb_root = tmp.path().join("kb");
    scaffold::new_bundle(
        &kb_root,
        "demo",
        None,
        "Demo",
        "A scratch bundle.",
        "0.2",
        today(),
    )
    .unwrap();
    (tmp, kb_root)
}

fn load(kb_root: &Path) -> Kb {
    morphir_okf::store::load(kb_root).unwrap()
}

fn add_concept(kb_root: &Path, path: &str, title: &str, description: &str) {
    let kb = load(kb_root);
    let b = kb.bundle("demo").unwrap();
    scaffold::add_concept(
        b,
        path,
        "Concept",
        title,
        description,
        &["x".to_string()],
        Some("draft"),
        &[],
        "Orientation",
        None,
        today(),
    )
    .unwrap();
}

// --------------------------------------------------------------------- list

#[test]
fn list_bundles_text_is_the_aligned_table_with_counts() {
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "a.md", "A", "First.");
    let kb = load(&kb_root);
    let expected = "\
BUNDLE  OKF   CONCEPTS  TITLE
demo    0.2   1         Demo

1 bundle(s), 1 concept(s)
";
    assert_eq!(render::list_bundles(&kb, false), expected);
}

#[test]
fn list_bundles_text_warns_about_strays() {
    let (_tmp, kb_root) = fixture();
    fs::write(kb_root.join("bundles").join("loose.md"), "# Loose\n").unwrap();
    let kb = load(&kb_root);
    let text = render::list_bundles(&kb, false);
    assert!(
        text.ends_with("1 stray markdown file(s) outside any bundle — run `kb check`\n"),
        "got: {text}"
    );
}

#[test]
fn list_bundles_json_matches_the_scala_shape() {
    let (_tmp, kb_root) = fixture();
    let kb = load(&kb_root);
    let expected = format!(
        r#"{{
  "root": "{}",
  "bundles": [
    {{
      "label": "demo",
      "name": "demo",
      "group": null,
      "okfVersion": "0.2",
      "title": "Demo",
      "description": "A scratch bundle.",
      "concepts": 0,
      "subIndexes": 0,
      "hasLog": true
    }}
  ]
}}
"#,
        paths::render(&kb_root)
    );
    assert_eq!(render::list_bundles(&kb, true), expected);
}

#[test]
fn list_concepts_text_shows_type_status_and_description() {
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "naming.md", "Naming", "How things are named.");
    let kb = load(&kb_root);
    let b = kb.bundle("demo").unwrap();
    let expected = "\
demo — Demo
A scratch bundle.

/naming.md  Concept [draft]
            How things are named.

1 concept(s)
";
    assert_eq!(render::list_concepts(&kb, b, false), expected);
}

#[test]
fn list_concepts_json_matches_the_scala_shape() {
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "naming.md", "Naming", "How things are named.");
    let kb = load(&kb_root);
    let b = kb.bundle("demo").unwrap();
    let file = paths::render(&kb_root.join("bundles").join("demo").join("naming.md"));
    let expected = format!(
        r#"{{
  "bundle": "demo",
  "concepts": [
    {{
      "path": "/naming.md",
      "file": "{file}",
      "type": "Concept",
      "title": "Naming",
      "description": "How things are named.",
      "status": "draft",
      "tags": [
        "x"
      ],
      "sources": []
    }}
  ]
}}
"#
    );
    assert_eq!(render::list_concepts(&kb, b, true), expected);
}

// --------------------------------------------------------------------- show

#[test]
fn show_text_renders_fields_outline_and_optional_body() {
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "naming.md", "Naming", "How things are named.");
    let kb = load(&kb_root);
    let file = paths::render(&kb_root.join("bundles").join("demo").join("naming.md"));

    let text = render::show(&kb, "/naming.md", None, false, false);
    let expected = format!(
        "demo/naming.md\n\
         file:        {file}\n\
         kind:        Concept\n\
         type:        Concept\n\
         title:       Naming\n\
         description: How things are named.\n\
         status:      draft\n\
         tags:        x\n\
         \noutline:\n  # Naming\n"
    );
    assert_eq!(text, expected);

    let with_body = render::show(&kb, "/naming.md", None, true, false);
    assert!(
        with_body.contains("\n---\n\n# Naming\n"),
        "got: {with_body}"
    );
}

#[test]
fn show_lists_outbound_links_excluding_external_and_anchors() {
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "a.md", "A", "First.");
    let a = kb_root.join("bundles").join("demo").join("a.md");
    let text = fs::read_to_string(&a).unwrap();
    fs::write(
        &a,
        format!("{text}\nSee [b](/b.md), [ext](https://example.com) and [here](#anchor).\n"),
    )
    .unwrap();
    let kb = load(&kb_root);
    let shown = render::show(&kb, "/a.md", None, false, false);
    assert!(
        shown.contains("\noutbound links (1):\n  /b.md\n"),
        "got: {shown}"
    );
}

#[test]
fn show_resolves_a_path_suffix_and_respects_the_bundle_hint() {
    let (_tmp, kb_root) = fixture();
    scaffold::new_bundle(
        &kb_root,
        "other",
        None,
        "Other",
        "A second bundle.",
        "0.2",
        today(),
    )
    .unwrap();
    add_concept(&kb_root, "naming.md", "Naming", "How things are named.");
    {
        let kb = load(&kb_root);
        let b = kb.bundle("other").unwrap();
        scaffold::add_concept(
            b,
            "naming.md",
            "Concept",
            "Other Naming",
            "Different.",
            &[],
            None,
            &[],
            "Orientation",
            None,
            today(),
        )
        .unwrap();
    }
    let kb = load(&kb_root);
    // A path suffix finds a document without knowing its bundle.
    let by_suffix = render::show(&kb, "demo/naming.md", None, false, false);
    assert!(
        by_suffix.starts_with("demo/naming.md\n"),
        "got: {by_suffix}"
    );
    // A bundle-relative path is ambiguous across bundles; the hint chooses.
    let hinted = render::show(&kb, "/naming.md", Some("other"), false, false);
    assert!(
        hinted.contains("title:       Other Naming"),
        "got: {hinted}"
    );
}

#[test]
fn show_reports_a_miss_in_both_text_and_json() {
    let (_tmp, kb_root) = fixture();
    let kb = load(&kb_root);
    assert_eq!(
        render::show(&kb, "/nope.md", None, false, false),
        "not found: /nope.md\n"
    );
    assert_eq!(
        render::show(&kb, "/nope.md", None, false, true),
        "{\n  \"found\": false,\n  \"query\": \"/nope.md\"\n}\n"
    );
}

#[test]
fn show_json_appends_found_bundle_kind_and_links_to_the_concept_shape() {
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "a.md", "A", "First.");
    let a = kb_root.join("bundles").join("demo").join("a.md");
    fs::write(
        &a,
        "---\ntype: Concept\ntitle: A\ndescription: First.\ntags: [x]\nstatus: draft\n---\n\nSee [b](/b.md).\n",
    )
    .unwrap();
    let kb = load(&kb_root);
    let file = paths::render(&a);
    let expected = format!(
        r#"{{
  "path": "/a.md",
  "file": "{file}",
  "type": "Concept",
  "title": "A",
  "description": "First.",
  "status": "draft",
  "tags": [
    "x"
  ],
  "sources": [],
  "found": true,
  "bundle": "demo",
  "kind": "Concept",
  "links": [
    {{
      "dest": "/b.md",
      "line": 9,
      "external": false
    }}
  ]
}}
"#
    );
    assert_eq!(render::show(&kb, "/a.md", None, false, true), expected);
}

#[test]
fn show_json_includes_the_body_when_asked() {
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "a.md", "A", "First.");
    let kb = load(&kb_root);
    let json = render::show(&kb, "/a.md", None, true, true);
    assert!(
        json.contains("\"body\": \"\\n# A\\n\\nFirst.\\n\\n"),
        "got: {json}"
    );
}

// ------------------------------------------------------------------- search

fn search_fixture() -> (TempDir, PathBuf) {
    let (tmp, kb_root) = fixture();
    add_concept(&kb_root, "naming.md", "Naming", "How things are named.");
    let other = kb_root.join("bundles").join("demo").join("layout.md");
    fs::write(
        &other,
        "---\ntype: Guide\ntitle: Layout\ndescription: Where files live.\ntags: [structure]\nstatus: stable\n---\n\n\
         The layout narrows.\nA needle here.\nAnother needle line.\nA third needle.\nA fourth needle.\n",
    )
    .unwrap();
    (tmp, kb_root)
}

#[test]
fn search_filters_are_and_combined_and_case_insensitive() {
    let (_tmp, kb_root) = search_fixture();
    let kb = load(&kb_root);
    let hit = render::search(&kb, Some("NAMED"), false, None, &[], None, None, false);
    assert!(hit.contains("demo/naming.md"), "got: {hit}");
    assert!(hit.ends_with("\n1 match(es)\n"), "got: {hit}");

    // The query matches, but the type filter does not: AND semantics.
    let miss = render::search(
        &kb,
        Some("NAMED"),
        false,
        Some("Guide"),
        &[],
        None,
        None,
        false,
    );
    assert_eq!(miss, "no matches\n");

    // Filters alone, with no query at all.
    let by_facets = render::search(
        &kb,
        None,
        false,
        Some("guide"),
        &["STRUCTURE".to_string()],
        Some("Stable"),
        Some("demo"),
        false,
    );
    assert!(by_facets.contains("demo/layout.md"), "got: {by_facets}");
    assert!(by_facets.ends_with("\n1 match(es)\n"), "got: {by_facets}");
}

#[test]
fn search_body_hits_show_three_lines_then_elide() {
    let (_tmp, kb_root) = search_fixture();
    let kb = load(&kb_root);
    let text = render::search(&kb, Some("needle"), true, None, &[], None, None, false);
    assert!(text.contains("  3: A needle here.\n"), "got: {text}");
    assert!(text.contains("  4: Another needle line.\n"), "got: {text}");
    assert!(text.contains("  5: A third needle.\n"), "got: {text}");
    assert!(text.contains("  … 1 more line(s)\n"), "got: {text}");
    // Without --body the same query misses: nothing in the metadata says needle.
    assert_eq!(
        render::search(&kb, Some("needle"), false, None, &[], None, None, false),
        "no matches\n"
    );
}

#[test]
fn search_json_matches_the_scala_shape() {
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "naming.md", "Naming", "How things are named.");
    let kb = load(&kb_root);
    let file = paths::render(&kb_root.join("bundles").join("demo").join("naming.md"));
    let expected = format!(
        r#"{{
  "matches": 1,
  "results": [
    {{
      "path": "/naming.md",
      "file": "{file}",
      "type": "Concept",
      "title": "Naming",
      "description": "How things are named.",
      "status": "draft",
      "tags": [
        "x"
      ],
      "sources": [],
      "bundle": "demo",
      "bodyHits": []
    }}
  ]
}}
"#
    );
    assert_eq!(
        render::search(&kb, Some("naming"), false, None, &[], None, None, true),
        expected
    );
}

#[test]
fn search_json_reports_body_hits_with_line_numbers() {
    let (_tmp, kb_root) = search_fixture();
    let kb = load(&kb_root);
    let json = render::search(&kb, Some("fourth"), true, None, &[], None, None, true);
    assert!(
        json.contains("\"bodyHits\": [\n        {\n          \"line\": 6,\n          \"text\": \"A fourth needle.\"\n        }\n      ]"),
        "got: {json}"
    );
}

// ----------------------------------------------------------------- scaffold

#[test]
fn scaffold_render_lists_created_updated_and_notes() {
    let r = ScaffoldResult {
        created: vec![PathBuf::from("/kb/bundles/demo/index.md")],
        updated: vec![PathBuf::from("/kb/bundles/demo/log.md")],
        notes: vec!["add the bundle to the Bundles table".to_string()],
    };
    let expected = "\
created  /kb/bundles/demo/index.md
updated  /kb/bundles/demo/log.md
note     add the bundle to the Bundles table

next: write the body, then run `kb check`
";
    assert_eq!(render::scaffold(&r, false), expected);
    let expected_json = r#"{
  "created": [
    "/kb/bundles/demo/index.md"
  ],
  "updated": [
    "/kb/bundles/demo/log.md"
  ],
  "notes": [
    "add the bundle to the Bundles table"
  ]
}
"#;
    assert_eq!(render::scaffold(&r, true), expected_json);
}
