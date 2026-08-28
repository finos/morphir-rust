//! Ports of the `KbScaffoldSpec` cases (and the scaffold-adjacent
//! `KbPathsSpec` vocabulary cases) from `KbTests.scala`, plus byte-exact
//! template pins the Scala suite asserts by substring only.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use morphir_kb::scaffold::{
    add_concept, append_log_entry, insert_index_entry, new_bundle, parse_source,
};
use morphir_kb::util::slugify;
use morphir_okf::model::{Bundle, Kb, SourceRef};
use tempfile::TempDir;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 28).unwrap()
}

/// A minimal knowledge base: one ordinary bundle.
fn fixture() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let kb_root = tmp.path().join("kb");
    new_bundle(
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

fn demo(kb: &Kb) -> &Bundle {
    kb.bundle("demo").unwrap()
}

// ------------------------------------------------------------------ templates

#[test]
fn bundle_index_template_is_byte_exact() {
    let (_tmp, kb_root) = fixture();
    let index = fs::read_to_string(kb_root.join("bundles/demo/index.md")).unwrap();
    assert_eq!(
        index,
        "---\nokf_version: \"0.2\"\ntitle: Demo\ndescription: A scratch bundle.\n---\n\n# Demo\n\nA scratch bundle.\n\n## Orientation\n\n"
    );
}

#[test]
fn bundle_log_template_is_byte_exact() {
    let (_tmp, kb_root) = fixture();
    let log = fs::read_to_string(kb_root.join("bundles/demo/log.md")).unwrap();
    assert_eq!(
        log,
        "# Log\n\n## 2026-07-28\n\n* **Creation**: Bundle created.\n"
    );
}

#[test]
fn bundle_title_and_description_are_yaml_quoted_when_needed() {
    let tmp = TempDir::new().unwrap();
    let kb_root = tmp.path().join("kb");
    new_bundle(
        &kb_root,
        "tricky",
        None,
        "Tricky: title",
        "Contains, commas",
        "0.2",
        today(),
    )
    .unwrap();
    let index = fs::read_to_string(kb_root.join("bundles/tricky/index.md")).unwrap();
    assert!(index.contains("title: \"Tricky: title\"\n"));
    assert!(index.contains("description: \"Contains, commas\"\n"));
    // The body keeps the raw text.
    assert!(index.contains("# Tricky: title\n"));
}

#[test]
fn new_bundle_refuses_an_existing_directory() {
    let (_tmp, kb_root) = fixture();
    let err = new_bundle(&kb_root, "demo", None, "Demo", "Again.", "0.2", today())
        .unwrap_err()
        .to_string();
    assert!(err.contains("already exists"), "got: {err}");
}

#[test]
fn new_bundle_notes_a_missing_group_readme() {
    let (_tmp, kb_root) = fixture();
    let res = new_bundle(
        &kb_root,
        "sub",
        Some("morphir"),
        "Sub",
        "Grouped.",
        "0.2",
        today(),
    )
    .unwrap();
    assert!(kb_root.join("bundles/morphir/sub/index.md").is_file());
    assert!(
        res.notes
            .iter()
            .any(|n| n.contains("grouping directory has no README.md yet")),
        "got: {:?}",
        res.notes
    );
    assert!(
        res.notes
            .iter()
            .any(|n| n.contains("add the bundle to the Bundles table in")),
        "got: {:?}",
        res.notes
    );
}

#[test]
fn concept_template_with_all_optional_blocks_is_byte_exact() {
    let (_tmp, kb_root) = fixture();
    let kb = load(&kb_root);
    let sources = vec![
        SourceRef {
            id: Some("s1".to_string()),
            resource: "https://example.com/x.md".to_string(),
            title: Some("Example: doc".to_string()),
        },
        SourceRef {
            id: None,
            resource: "https://example.com/y.md".to_string(),
            title: None,
        },
    ];
    add_concept(
        demo(&kb),
        "naming.md",
        "Concept",
        "Naming",
        "How things are named.",
        &["Foo Bar".to_string(), "x".to_string()],
        Some("draft"),
        &sources,
        "Orientation",
        Some("process:kb-seed"),
        today(),
    )
    .unwrap();
    let concept = fs::read_to_string(kb_root.join("bundles/demo/naming.md")).unwrap();
    assert_eq!(
        concept,
        "---\n\
         type: Concept\n\
         title: Naming\n\
         description: How things are named.\n\
         tags: [foo-bar, x]\n\
         status: draft\n\
         sources:\n\
         \x20 - id: s1\n\
         \x20   resource: https://example.com/x.md\n\
         \x20   title: \"Example: doc\"\n\
         \x20 - resource: https://example.com/y.md\n\
         generated:\n\
         \x20 by: process:kb-seed\n\
         \x20 at: 2026-07-28T00:00:00Z\n\
         ---\n\
         \n\
         # Naming\n\
         \n\
         How things are named.\n\
         \n\
         <!-- TODO: write the concept body. Delete this comment when done. -->\n"
    );
}

// ---------------------------------------------------------------- add-concept

#[test]
fn add_concept_refuses_a_path_that_escapes_the_bundle() {
    // Regression in Scala: `--path ../escaped.md` wrote outside the bundle
    // while still adding a bundle-relative index entry pointing at nothing.
    let (_tmp, kb_root) = fixture();
    let kb = load(&kb_root);
    let err = add_concept(
        demo(&kb),
        "../escaped.md",
        "Concept",
        "X",
        "Y.",
        &[],
        None,
        &[],
        "Orientation",
        None,
        today(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("must stay inside the bundle"), "got: {err}");
    assert!(
        !kb_root.join("bundles/escaped.md").exists(),
        "nothing may be written outside the bundle"
    );
}

#[test]
fn add_concept_refuses_an_absolute_path() {
    let (_tmp, kb_root) = fixture();
    let kb = load(&kb_root);
    let err = add_concept(
        demo(&kb),
        "/naming.md",
        "Concept",
        "X",
        "Y.",
        &[],
        None,
        &[],
        "Orientation",
        None,
        today(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("must stay inside the bundle"), "got: {err}");
}

#[test]
fn add_concept_refuses_a_path_that_escapes_through_a_windows_separator() {
    // The guard split on `/` alone, so none of these held a separator it could
    // see; `PathBuf::push` on Windows reads all of them as escapes.
    let (_tmp, kb_root) = fixture();
    let kb = load(&kb_root);
    for path in [
        "..\\escaped.md",
        "..\\..\\outside.md",
        "a/..\\escaped.md",
        "C:\\victim.md",
        "\\victim.md",
    ] {
        let err = add_concept(
            demo(&kb),
            path,
            "Concept",
            "X",
            "Y.",
            &[],
            None,
            &[],
            "Orientation",
            None,
            today(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("must stay inside the bundle"), "got: {err}");
    }
}

#[test]
fn add_concept_accepts_a_backslash_that_stays_contained_on_unix_and_on_windows() {
    // Policy: refuse what escapes under *some* platform's reading of the path,
    // not every path a platform reads differently. `notes\draft.md` is one file
    // called `notes\draft.md` on Unix and `notes/draft.md` on Windows — inside
    // the bundle either way, so it stays legal.
    let (_tmp, kb_root) = fixture();
    let kb = load(&kb_root);
    add_concept(
        demo(&kb),
        "notes\\draft.md",
        "Concept",
        "Draft",
        "A draft.",
        &[],
        None,
        &[],
        "Orientation",
        None,
        today(),
    )
    .unwrap();
    assert!(kb_root.join("bundles/demo/notes\\draft.md").is_file());
}

#[test]
fn add_concept_refuses_the_reserved_okf_filenames() {
    let (_tmp, kb_root) = fixture();
    let kb = load(&kb_root);
    for leaf in ["index.md", "log.md"] {
        let err = add_concept(
            demo(&kb),
            leaf,
            "Concept",
            "X",
            "Y.",
            &[],
            None,
            &[],
            "Orientation",
            None,
            today(),
        )
        .unwrap_err()
        .to_string();
        assert_eq!(err, format!("{leaf} is a reserved OKF filename"));
    }
}

#[test]
fn add_concept_refuses_a_dot_segment() {
    let (_tmp, kb_root) = fixture();
    let kb = load(&kb_root);
    let err = add_concept(
        demo(&kb),
        "./naming.md",
        "Concept",
        "X",
        "Y.",
        &[],
        None,
        &[],
        "Orientation",
        None,
        today(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("must stay inside the bundle"), "got: {err}");
}

#[test]
fn add_concept_writes_the_concept_and_wires_up_index_and_log() {
    let (_tmp, kb_root) = fixture();
    let kb = load(&kb_root);
    add_concept(
        demo(&kb),
        "naming.md",
        "Concept",
        "Naming",
        "How things are named.",
        &["x".to_string()],
        Some("draft"),
        &[],
        "Orientation",
        None,
        today(),
    )
    .unwrap();
    let idx = fs::read_to_string(kb_root.join("bundles/demo/index.md")).unwrap();
    let log = fs::read_to_string(kb_root.join("bundles/demo/log.md")).unwrap();
    let concept = fs::read_to_string(kb_root.join("bundles/demo/naming.md")).unwrap();
    assert!(
        idx.contains("[Naming](/naming.md) - How things are named."),
        "index entry mirrors the description"
    );
    assert!(log.contains("**Creation**: Added [Naming](/naming.md)."));
    assert!(concept.contains("type: Concept") && concept.contains("status: draft"));
}

#[test]
fn add_concept_appends_md_when_the_extension_is_missing() {
    let (_tmp, kb_root) = fixture();
    let kb = load(&kb_root);
    add_concept(
        demo(&kb),
        "naming",
        "Concept",
        "Naming",
        "Named.",
        &[],
        None,
        &[],
        "Orientation",
        None,
        today(),
    )
    .unwrap();
    assert!(kb_root.join("bundles/demo/naming.md").is_file());
}

#[test]
fn add_concept_files_a_subdirectory_concept_in_the_sub_index() {
    let (_tmp, kb_root) = fixture();
    let design = kb_root.join("bundles/demo/design");
    fs::create_dir_all(&design).unwrap();
    fs::write(design.join("index.md"), "# Design\n\n## Orientation\n").unwrap();
    let kb = load(&kb_root);
    add_concept(
        demo(&kb),
        "design/naming.md",
        "Concept",
        "Naming",
        "Named.",
        &[],
        None,
        &[],
        "Orientation",
        None,
        today(),
    )
    .unwrap();
    let sub = fs::read_to_string(design.join("index.md")).unwrap();
    let root = fs::read_to_string(kb_root.join("bundles/demo/index.md")).unwrap();
    assert!(sub.contains("[Naming](/design/naming.md) - Named."));
    assert!(
        !root.contains("[Naming]"),
        "entry belongs in the sub-index only"
    );
}

// --------------------------------------------------------------- index edits

#[test]
fn index_entry_appends_after_the_last_bullet_of_the_section() {
    let tmp = TempDir::new().unwrap();
    let index = tmp.path().join("index.md");
    fs::write(
        &index,
        "# T\n\n## Orientation\n\n* [A](/a.md) - First.\n\n## Later\n\n* [Z](/z.md) - Last.\n",
    )
    .unwrap();
    insert_index_entry(&index, "Orientation", "B", "/b.md", "Second.").unwrap();
    let text = fs::read_to_string(&index).unwrap();
    assert_eq!(
        text,
        "# T\n\n## Orientation\n\n* [A](/a.md) - First.\n* [B](/b.md) - Second.\n\n## Later\n\n* [Z](/z.md) - Last.\n"
    );
}

#[test]
fn index_entry_creates_the_section_at_end_of_file_when_absent() {
    let tmp = TempDir::new().unwrap();
    let index = tmp.path().join("index.md");
    fs::write(&index, "# T\n\nprose\n\n").unwrap();
    insert_index_entry(&index, "Orientation", "A", "/a.md", "First.").unwrap();
    let text = fs::read_to_string(&index).unwrap();
    assert_eq!(
        text,
        "# T\n\nprose\n\n## Orientation\n\n* [A](/a.md) - First.\n"
    );
}

#[test]
fn index_entry_lands_at_section_end_when_it_has_no_bullets_yet() {
    let tmp = TempDir::new().unwrap();
    let index = tmp.path().join("index.md");
    fs::write(&index, "# T\n\n## Orientation\n\n## Later\n").unwrap();
    insert_index_entry(&index, "Orientation", "A", "/a.md", "First.").unwrap();
    let text = fs::read_to_string(&index).unwrap();
    // The blank line after the heading survives.
    assert_eq!(
        text,
        "# T\n\n## Orientation\n\n* [A](/a.md) - First.\n## Later\n"
    );
}

#[test]
fn index_entry_matches_the_section_heading_case_insensitively() {
    let tmp = TempDir::new().unwrap();
    let index = tmp.path().join("index.md");
    fs::write(&index, "# T\n\n## ORIENTATION\n\n* [A](/a.md) - First.\n").unwrap();
    insert_index_entry(&index, "Orientation", "B", "/b.md", "Second.").unwrap();
    let text = fs::read_to_string(&index).unwrap();
    assert!(text.contains("* [B](/b.md) - Second.\n"));
    assert_eq!(text.matches("##").count(), 1, "no second section created");
}

// ----------------------------------------------------------------- log edits

#[test]
fn log_entries_create_a_date_section_once_then_append_within_it() {
    let tmp = TempDir::new().unwrap();
    let log = tmp.path().join("log.md");
    fs::write(&log, "# Log\n").unwrap();
    append_log_entry(&log, today(), "**Creation**: one.").unwrap();
    append_log_entry(&log, today(), "**Update**: two.").unwrap();
    let text = fs::read_to_string(&log).unwrap();
    assert_eq!(
        text.lines()
            .filter(|l| l.starts_with("## 2026-07-28"))
            .count(),
        1,
        "one date heading, not two"
    );
    assert!(text.contains("* **Creation**: one.") && text.contains("* **Update**: two."));
}

#[test]
fn log_new_date_section_goes_on_top_newest_first() {
    let tmp = TempDir::new().unwrap();
    let log = tmp.path().join("log.md");
    fs::write(&log, "# Log\n\n## 2026-07-01\n\n* **Creation**: old.\n").unwrap();
    append_log_entry(&log, today(), "**Update**: new.").unwrap();
    let text = fs::read_to_string(&log).unwrap();
    assert_eq!(
        text,
        "# Log\n\n## 2026-07-28\n\n* **Update**: new.\n\n## 2026-07-01\n\n* **Creation**: old.\n"
    );
}

// ------------------------------------------------------------ source parsing

#[test]
fn parse_source_reads_the_three_forms() {
    assert_eq!(
        parse_source("s1=https://example.com/x"),
        SourceRef {
            id: Some("s1".to_string()),
            resource: "https://example.com/x".to_string(),
            title: None,
        }
    );
    assert_eq!(
        parse_source("s1=https://example.com/x=The Title"),
        SourceRef {
            id: Some("s1".to_string()),
            resource: "https://example.com/x".to_string(),
            title: Some("The Title".to_string()),
        }
    );
    assert_eq!(
        parse_source("https://example.com/x"),
        SourceRef {
            id: None,
            resource: "https://example.com/x".to_string(),
            title: None,
        }
    );
}

#[test]
fn parse_source_keeps_a_non_http_value_whole() {
    // `a=b` where `b` is not a URL is a bare resource, `=` and all.
    assert_eq!(
        parse_source("a=b"),
        SourceRef {
            id: None,
            resource: "a=b".to_string(),
            title: None,
        }
    );
}

// -------------------------------------------------------------------- slugify

#[test]
fn slugify_kebab_cases_free_form_names() {
    assert_eq!(slugify("  Release Labels, v2! "), "release-labels-v2");
    assert_eq!(slugify("Morphir IR v5"), "morphir-ir-v5");
    assert_eq!(slugify("--x--"), "x");
}

#[test]
fn new_bundle_refuses_a_group_that_escapes_the_bundles_directory() {
    let (tmp, kb_root) = fixture();
    let err = new_bundle(
        &kb_root,
        "escaped",
        Some("../../outside"),
        "Escaped",
        "Nope.",
        "0.2",
        today(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("must stay inside kb/bundles"), "got: {err}");
    assert!(
        !tmp.path().join("outside").exists(),
        "nothing may be written outside kb/bundles"
    );
}

#[test]
fn new_bundle_refuses_an_absolute_group() {
    // Previously accepted and silently reinterpreted as a subdirectory of
    // kb/bundles, which is not what the caller asked for.
    let (_tmp, kb_root) = fixture();
    let err = new_bundle(
        &kb_root,
        "rooted",
        Some("/morphir"),
        "Rooted",
        "Nope.",
        "0.2",
        today(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("must stay inside kb/bundles"), "got: {err}");
    assert!(!kb_root.join("bundles/morphir/rooted").exists());
}

#[test]
fn new_bundle_refuses_a_single_dot_group_segment() {
    let (_tmp, kb_root) = fixture();
    let err = new_bundle(
        &kb_root,
        "dotted",
        Some("morphir/./tools"),
        "Dotted",
        "Nope.",
        "0.2",
        today(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("must stay inside kb/bundles"), "got: {err}");
}

#[test]
fn new_bundle_accepts_a_benign_nested_group_and_names_its_real_path() {
    let (_tmp, kb_root) = fixture();
    let res = new_bundle(
        &kb_root,
        "sub",
        Some("morphir/tools"),
        "Sub",
        "Grouped.",
        "0.2",
        today(),
    )
    .unwrap();
    assert!(kb_root.join("bundles/morphir/tools/sub/index.md").is_file());
    let note = res
        .notes
        .iter()
        .find(|n| n.contains("grouping directory has no README.md yet"))
        .unwrap_or_else(|| panic!("got: {:?}", res.notes));
    assert!(
        note.ends_with("/kb/bundles/morphir/tools"),
        "the note must name the directory that was really created, got: {note}"
    );
}

#[test]
fn new_bundle_refuses_a_group_that_escapes_through_a_windows_separator() {
    let (tmp, kb_root) = fixture();
    for group in [
        "..\\..\\outside",
        "morphir/..\\outside",
        "C:\\victim",
        "\\victim",
    ] {
        let err = new_bundle(
            &kb_root,
            "escaped",
            Some(group),
            "Escaped",
            "Nope.",
            "0.2",
            today(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("must stay inside kb/bundles"), "got: {err}");
    }
    assert!(!tmp.path().join("outside").exists());
    let mut entries: Vec<String> = fs::read_dir(kb_root.join("bundles"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        vec!["demo".to_string()],
        "nothing may be scaffolded outside kb/bundles"
    );
}

#[cfg(unix)]
#[test]
fn new_bundle_refuses_a_group_directory_that_symlinks_out_of_the_knowledge_base() {
    // `shared` is a plain name and passes every lexical check, but it is a link,
    // and scaffolding through it writes outside kb/bundles entirely.
    let (tmp, kb_root) = fixture();
    let outside = tmp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, kb_root.join("bundles/shared")).unwrap();
    let err = new_bundle(
        &kb_root,
        "leaked",
        Some("shared"),
        "Leaked",
        "Nope.",
        "0.2",
        today(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("resolves outside"), "got: {err}");
    assert!(
        !outside.join("leaked").exists(),
        "nothing may be scaffolded through the link"
    );
}
