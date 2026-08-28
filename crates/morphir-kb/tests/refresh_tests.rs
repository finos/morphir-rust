//! Tests for the `refresh` module: index-bullet repair, missing-entry
//! reporting and appending, the generated intent index, SQLite freshness,
//! the reload-between-passes orchestration, and the render shapes.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use morphir_kb::refresh::{self, RefreshAction, RefreshKind};
use morphir_kb::{index, intent, scaffold};
use morphir_okf::model::Kb;
use morphir_okf::paths;
use tempfile::TempDir;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 28).unwrap()
}

/// A build stamp far in the past: every file on disk is newer, so the index
/// reads as stale.
fn old_build() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 0).unwrap()
}

/// A build stamp far in the future: nothing on disk is newer, so the index
/// reads as fresh.
fn future_build() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2100, 1, 1, 0, 0, 0).unwrap()
}

fn fixture(with_intent: bool) -> (TempDir, PathBuf) {
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
    if with_intent {
        intent::init_bundle(
            &kb_root,
            "intent",
            Some("pkg:pypi/demo"),
            Some("demo"),
            60,
            today(),
        )
        .unwrap();
    }
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
        &[],
        None,
        &[],
        "Orientation",
        None,
        today(),
    )
    .unwrap();
}

fn demo_index(kb_root: &Path) -> PathBuf {
    kb_root.join("bundles").join("demo").join("index.md")
}

fn markdown(kb_root: &Path, add_missing: bool, dry_run: bool) -> Vec<RefreshAction> {
    refresh::refresh_markdown(&load(kb_root), add_missing, "Orientation", dry_run, today()).unwrap()
}

// ----------------------------------------------------------------- markdown

#[test]
fn a_drifted_bullet_is_rewritten_and_the_link_is_preserved() {
    let (_tmp, kb_root) = fixture(false);
    add_concept(&kb_root, "a.md", "A", "The real description.");
    let index = demo_index(&kb_root);
    let text = fs::read_to_string(&index).unwrap();
    fs::write(&index, text.replace("The real description.", "stale text")).unwrap();

    let actions = markdown(&kb_root, false, false);
    assert_eq!(actions.len(), 1, "got {actions:?}");
    assert_eq!(actions[0].kind, RefreshKind::DescriptionFixed);
    assert!(
        actions[0].file.starts_with("kb/bundles/demo/index.md:"),
        "got {}",
        actions[0].file
    );
    assert_eq!(actions[0].detail, "/a.md — was \"stale text\"");

    let after = fs::read_to_string(&index).unwrap();
    assert!(
        after.contains("* [A](/a.md) - The real description."),
        "the link survives and the description is repaired: {after}"
    );

    // A second pass finds nothing to do.
    assert!(markdown(&kb_root, false, false).is_empty());
}

#[test]
fn a_bullet_with_no_description_reports_the_placeholder() {
    let (_tmp, kb_root) = fixture(false);
    add_concept(&kb_root, "a.md", "A", "The real description.");
    let index = demo_index(&kb_root);
    let text = fs::read_to_string(&index).unwrap();
    fs::write(
        &index,
        text.replace("* [A](/a.md) - The real description.", "* [A](/a.md)"),
    )
    .unwrap();
    let actions = markdown(&kb_root, false, false);
    assert_eq!(actions.len(), 1, "got {actions:?}");
    assert_eq!(actions[0].detail, "/a.md — was (no description)");
}

#[test]
fn a_dry_run_reports_without_writing() {
    let (_tmp, kb_root) = fixture(false);
    add_concept(&kb_root, "a.md", "A", "Real description.");
    let index = demo_index(&kb_root);
    let text = fs::read_to_string(&index).unwrap();
    fs::write(&index, text.replace("Real description.", "stale")).unwrap();

    let before = fs::read_to_string(&index).unwrap();
    let actions = markdown(&kb_root, false, true);
    let after = fs::read_to_string(&index).unwrap();
    assert!(!actions.is_empty());
    assert_eq!(before, after, "dry run must not write");
}

#[test]
fn a_lenient_difference_is_not_drift() {
    let (_tmp, kb_root) = fixture(false);
    add_concept(&kb_root, "a.md", "A", "The real description.");
    let index = demo_index(&kb_root);
    let text = fs::read_to_string(&index).unwrap();
    fs::write(
        &index,
        text.replace("The real description.", "the REAL   description"),
    )
    .unwrap();
    assert!(markdown(&kb_root, false, false).is_empty());
}

#[test]
fn unindexed_concepts_are_reported_and_appended_only_on_request() {
    let (_tmp, kb_root) = fixture(false);
    fs::write(
        kb_root.join("bundles").join("demo").join("orphan.md"),
        "---\ntype: Concept\ntitle: Orphan\ndescription: Unlinked.\n---\n\nBody.\n",
    )
    .unwrap();

    let reported = markdown(&kb_root, false, false);
    assert_eq!(reported.len(), 1, "got {reported:?}");
    assert_eq!(reported[0].kind, RefreshKind::EntryMissing);
    assert_eq!(reported[0].file, "kb/bundles/demo/orphan.md");
    assert_eq!(
        reported[0].detail,
        "not linked from any index in demo — pass --add-missing to append it"
    );
    let untouched = fs::read_to_string(demo_index(&kb_root)).unwrap();
    assert!(
        !untouched.contains("/orphan.md"),
        "reporting must not write"
    );

    let added = markdown(&kb_root, true, false);
    assert_eq!(added.len(), 1, "got {added:?}");
    assert_eq!(added[0].kind, RefreshKind::EntryAdded);
    assert_eq!(added[0].file, "kb/bundles/demo/index.md");
    assert_eq!(added[0].detail, "/orphan.md under \"Orientation\"");
    let index = fs::read_to_string(demo_index(&kb_root)).unwrap();
    assert!(
        index.contains("* [Orphan](/orphan.md) - Unlinked."),
        "got: {index}"
    );

    // Once linked, nothing is missing.
    assert!(markdown(&kb_root, true, false).is_empty());
}

#[test]
fn add_missing_under_dry_run_reports_the_target_without_writing() {
    let (_tmp, kb_root) = fixture(false);
    fs::write(
        kb_root.join("bundles").join("demo").join("orphan.md"),
        "---\ntype: Concept\ntitle: Orphan\ndescription: Unlinked.\n---\n\nBody.\n",
    )
    .unwrap();
    let actions = markdown(&kb_root, true, true);
    assert_eq!(actions.len(), 1, "got {actions:?}");
    assert_eq!(actions[0].kind, RefreshKind::EntryAdded);
    let index = fs::read_to_string(demo_index(&kb_root)).unwrap();
    assert!(!index.contains("/orphan.md"), "dry run must not write");
}

#[test]
fn a_subdirectory_concept_lands_in_its_subdirectory_index() {
    let (_tmp, kb_root) = fixture(false);
    let sub = kb_root.join("bundles").join("demo").join("design");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("index.md"), "# Design\n\n## Orientation\n").unwrap();
    fs::write(
        sub.join("naming.md"),
        "---\ntype: Concept\ntitle: Naming\ndescription: Names.\n---\n\nBody.\n",
    )
    .unwrap();

    let actions = markdown(&kb_root, true, false);
    let added: Vec<&RefreshAction> = actions
        .iter()
        .filter(|a| a.kind == RefreshKind::EntryAdded)
        .collect();
    assert!(
        added
            .iter()
            .any(|a| a.file == "kb/bundles/demo/design/index.md"
                && a.detail == "/design/naming.md under \"Orientation\""),
        "got {actions:?}"
    );
    let sub_index = fs::read_to_string(sub.join("index.md")).unwrap();
    assert!(
        sub_index.contains("* [Naming](/design/naming.md) - Names."),
        "got: {sub_index}"
    );
}

// ------------------------------------------------------------- intent index

#[test]
fn the_intent_index_is_regenerated_not_repaired() {
    let (_tmp, kb_root) = fixture(true);
    {
        let kb = load(&kb_root);
        let b = intent::find_bundle(&kb).unwrap();
        intent::create(
            b,
            "First thing",
            "Something.",
            intent::IntentKind::Feature,
            false,
            None,
            &[],
            today(),
        )
        .unwrap();
    }

    let actions = markdown(&kb_root, false, false);
    let regen: Vec<&RefreshAction> = actions
        .iter()
        .filter(|a| a.kind == RefreshKind::IntentIndexRebuilt)
        .collect();
    assert_eq!(regen.len(), 1, "got {actions:?}");
    assert_eq!(regen[0].file, "kb/bundles/intent/index.md");
    assert_eq!(regen[0].detail, "1 intent grouped by state");
    // Only the regeneration is reported for the intent bundle: no per-entry
    // repair, no coverage findings against the pre-regeneration state.
    assert!(
        actions
            .iter()
            .all(|a| a.kind == RefreshKind::IntentIndexRebuilt),
        "got {actions:?}"
    );
    let index =
        fs::read_to_string(kb_root.join("bundles").join("intent").join("index.md")).unwrap();
    assert!(index.contains("## Backlog (1)"), "got: {index}");

    // A second run finds the index already settled and reports nothing.
    assert!(markdown(&kb_root, false, false).is_empty());
}

#[test]
fn the_intent_index_dry_run_reports_without_writing() {
    let (_tmp, kb_root) = fixture(true);
    {
        let kb = load(&kb_root);
        let b = intent::find_bundle(&kb).unwrap();
        intent::create(
            b,
            "First thing",
            "Something.",
            intent::IntentKind::Feature,
            false,
            None,
            &[],
            today(),
        )
        .unwrap();
    }
    let idx = kb_root.join("bundles").join("intent").join("index.md");
    let before = fs::read_to_string(&idx).unwrap();
    let actions = markdown(&kb_root, false, true);
    assert!(
        actions
            .iter()
            .any(|a| a.kind == RefreshKind::IntentIndexRebuilt),
        "got {actions:?}"
    );
    assert_eq!(fs::read_to_string(&idx).unwrap(), before);
}

// ----------------------------------------------------------------------- db

#[test]
fn refresh_db_builds_when_absent_rebuilds_when_stale_and_rests_when_fresh() {
    let (tmp, kb_root) = fixture(false);
    let db = tmp.path().join("index.db");
    let kb = load(&kb_root);

    // Absent, dry run: reported, not built.
    let dry = refresh::refresh_db(&kb, &db, false, true, future_build()).unwrap();
    assert_eq!(dry.len(), 1);
    assert_eq!(dry[0].kind, RefreshKind::IndexRebuilt);
    assert_eq!(dry[0].detail, "absent");
    assert!(!db.exists(), "dry run must not build");

    // Absent, wet: built.
    let built = refresh::refresh_db(&kb, &db, false, false, future_build()).unwrap();
    assert_eq!(built[0].kind, RefreshKind::IndexRebuilt);
    assert_eq!(built[0].detail, "built 2 docs");
    assert!(db.exists());

    // Fresh (built in the future, so no file is newer): rest.
    let fresh = refresh::refresh_db(&kb, &db, false, false, future_build()).unwrap();
    assert_eq!(fresh[0].kind, RefreshKind::IndexFresh);
    assert_eq!(fresh[0].detail, "up to date (built 2100-01-01T00:00:00Z)");

    // Fresh but forced: rebuilt.
    let forced = refresh::refresh_db(&kb, &db, true, false, future_build()).unwrap();
    assert_eq!(forced[0].kind, RefreshKind::IndexRebuilt);
    assert_eq!(forced[0].detail, "rebuilt 2 docs (forced)");

    // Stale (built in the past, so every file is newer): rebuilt with a count.
    index::build(&kb, &db, old_build()).unwrap();
    let stale = refresh::refresh_db(&kb, &db, false, false, future_build()).unwrap();
    assert_eq!(stale[0].kind, RefreshKind::IndexRebuilt);
    assert_eq!(stale[0].detail, "rebuilt 2 docs (2 file(s) changed)");

    // Stale, dry run: the reason is reported, nothing is rebuilt.
    index::build(&kb, &db, old_build()).unwrap();
    let stale_dry = refresh::refresh_db(&kb, &db, false, true, future_build()).unwrap();
    assert_eq!(stale_dry[0].kind, RefreshKind::IndexRebuilt);
    assert_eq!(stale_dry[0].detail, "2 file(s) changed");
}

// ------------------------------------------------------------ orchestration

#[test]
fn refresh_reloads_between_the_markdown_and_db_passes() {
    let (tmp, kb_root) = fixture(false);
    add_concept(&kb_root, "a.md", "A", "The real description.");
    let index_md = demo_index(&kb_root);
    let text = fs::read_to_string(&index_md).unwrap();
    fs::write(
        &index_md,
        text.replace("The real description.", "stale text"),
    )
    .unwrap();
    let db = tmp.path().join("index.db");

    let actions = refresh::refresh(
        &kb_root,
        true,
        true,
        false,
        false,
        false,
        "Orientation",
        &db,
        today(),
        future_build(),
    )
    .unwrap();
    assert!(
        actions
            .iter()
            .any(|a| a.kind == RefreshKind::DescriptionFixed),
        "got {actions:?}"
    );
    assert!(
        actions.iter().any(|a| a.kind == RefreshKind::IndexRebuilt),
        "got {actions:?}"
    );

    // The database was built from the *rewritten* markdown: the repaired
    // bullet is in the indexed body, the stale one is not.
    let fixed = index::query(
        &db,
        "SELECT count(*) FROM doc_fts WHERE body LIKE '%[A](/a.md) - The real description.%'",
    )
    .unwrap();
    assert_eq!(fixed.rows[0][0].as_deref(), Some("1"));
    let stale = index::query(
        &db,
        "SELECT count(*) FROM doc_fts WHERE body LIKE '%stale text%'",
    )
    .unwrap();
    assert_eq!(stale.rows[0][0].as_deref(), Some("0"));
}

#[test]
fn refresh_halves_can_be_disabled() {
    let (tmp, kb_root) = fixture(false);
    add_concept(&kb_root, "a.md", "A", "Real.");
    let db = tmp.path().join("index.db");

    let md_only = refresh::refresh(
        &kb_root,
        true,
        false,
        false,
        false,
        false,
        "Orientation",
        &db,
        today(),
        future_build(),
    )
    .unwrap();
    assert!(md_only.is_empty(), "nothing drifted: {md_only:?}");
    assert!(!db.exists(), "the db half was disabled");

    let db_only = refresh::refresh(
        &kb_root,
        false,
        true,
        false,
        false,
        false,
        "Orientation",
        &db,
        today(),
        future_build(),
    )
    .unwrap();
    assert_eq!(db_only.len(), 1);
    assert_eq!(db_only[0].kind, RefreshKind::IndexRebuilt);
    assert!(db.exists());
}

// ---------------------------------------------------------------- rendering

fn sample_actions() -> Vec<RefreshAction> {
    vec![
        RefreshAction {
            kind: RefreshKind::DescriptionFixed,
            file: "kb/bundles/demo/index.md:12".to_string(),
            detail: "/a.md — was \"stale\"".to_string(),
        },
        RefreshAction {
            kind: RefreshKind::EntryMissing,
            file: "kb/bundles/demo/orphan.md".to_string(),
            detail: "not linked from any index in demo — pass --add-missing to append it"
                .to_string(),
        },
        RefreshAction {
            kind: RefreshKind::IndexFresh,
            file: "/repo/.dev/kb/index.db".to_string(),
            detail: "up to date (built 2026-07-28T00:00:00Z)".to_string(),
        },
    ]
}

#[test]
fn render_text_uses_the_verb_column_and_summary() {
    let text = refresh::render(&sample_actions(), false, false);
    let expected = "\
fixed         kb/bundles/demo/index.md:12
              /a.md — was \"stale\"
missing       kb/bundles/demo/orphan.md
              not linked from any index in demo — pass --add-missing to append it
fresh         /repo/.dev/kb/index.db
              up to date (built 2026-07-28T00:00:00Z)

1 description(s) fixed, 1 unindexed concept(s)
";
    assert_eq!(text, expected);
}

#[test]
fn render_text_dry_run_switches_the_verbs() {
    let actions = vec![
        RefreshAction {
            kind: RefreshKind::EntryAdded,
            file: "kb/bundles/demo/index.md".to_string(),
            detail: "/orphan.md under \"Orientation\"".to_string(),
        },
        RefreshAction {
            kind: RefreshKind::IntentIndexRebuilt,
            file: "kb/bundles/intent/index.md".to_string(),
            detail: "3 intent grouped by state".to_string(),
        },
        RefreshAction {
            kind: RefreshKind::IndexRebuilt,
            file: "/db".to_string(),
            detail: "absent".to_string(),
        },
    ];
    let text = refresh::render(&actions, true, false);
    assert!(
        text.contains("would add     kb/bundles/demo/index.md\n"),
        "got: {text}"
    );
    assert!(
        text.contains("would regen   kb/bundles/intent/index.md\n"),
        "got: {text}"
    );
    assert!(text.contains("would rebuild /db\n"), "got: {text}");
    assert!(
        text.ends_with("\n0 description(s) to fix, 1 entry to add\n"),
        "got: {text}"
    );
}

#[test]
fn render_text_with_nothing_to_do_says_so() {
    assert_eq!(
        refresh::render(&[], false, false),
        "nothing to refresh\n\n0 description(s) fixed\n"
    );
}

#[test]
fn render_json_matches_the_scala_shape() {
    let json = refresh::render(&sample_actions(), false, true);
    let expected = r#"{
  "dryRun": false,
  "changed": 1,
  "actions": [
    {
      "kind": "DescriptionFixed",
      "file": "kb/bundles/demo/index.md:12",
      "detail": "/a.md — was \"stale\""
    },
    {
      "kind": "EntryMissing",
      "file": "kb/bundles/demo/orphan.md",
      "detail": "not linked from any index in demo — pass --add-missing to append it"
    },
    {
      "kind": "IndexFresh",
      "file": "/repo/.dev/kb/index.db",
      "detail": "up to date (built 2026-07-28T00:00:00Z)"
    }
  ]
}
"#;
    assert_eq!(json, expected);
}

#[test]
fn render_json_with_no_actions_uses_the_empty_array_form() {
    assert_eq!(
        refresh::render(&[], true, true),
        "{\n  \"dryRun\": true,\n  \"changed\": 0,\n  \"actions\": []\n}\n"
    );
}

// The `paths` import earns its keep here: the db path inside refresh actions
// is the rendered form, which the freshness test relies on implicitly.
#[test]
fn refresh_db_reports_the_rendered_db_path() {
    let (tmp, kb_root) = fixture(false);
    let db = tmp.path().join("index.db");
    let kb = load(&kb_root);
    let actions = refresh::refresh_db(&kb, &db, false, true, future_build()).unwrap();
    assert_eq!(actions[0].file, paths::render(&db));
}
