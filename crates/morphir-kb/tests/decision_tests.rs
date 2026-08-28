//! Ports of the `KbDecisionSpec` cases from `KbTests.scala`.

use std::fs;
use std::path::{Path, PathBuf};

use morphir_kb::decision::{
    self, DecisionState, ambiguous_message, decisions, decisions_in, find, find_all, findings,
};
use morphir_kb::scaffold::new_bundle;
use morphir_okf::model::Kb;
use tempfile::TempDir;

fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 7, 28).unwrap()
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

/// Writes a decision record straight to disk. Scaffolding one would go
/// through `add_concept`, but these tests care about the frontmatter the
/// checks read, not about how it got there.
fn record(kb_root: &Path, bundle: &str, name: &str, title: &str, state: &str, extra: &str) {
    let dir = kb_root.join("bundles").join(bundle).join("decisions");
    fs::create_dir_all(&dir).unwrap();
    let text = format!(
        "---\ntype: Decision Record\ntitle: {title}\ndescription: \"Something was decided.\"\nstate: {state}\ndecided: 2026-07-28\n{extra}---\n\n# {name}\n\nBody.\n"
    );
    fs::write(dir.join(format!("{name}.md")), text).unwrap();
}

// ----------------------------------------------------------------- discovery

#[test]
fn finds_records_by_type_and_reads_their_fields() {
    let (_tmp, kb_root) = fixture();
    record(&kb_root, "demo", "0001-first", "First", "Accepted", "");
    let kb = load(&kb_root);
    let ds = decisions(&kb);
    assert_eq!(ds.len(), 1);
    assert_eq!(ds[0].id(), "0001");
    assert_eq!(ds[0].slug(), "0001-first");
    assert_eq!(ds[0].state(), Some(DecisionState::Accepted));
    assert_eq!(
        ds[0].decided().map(|d| d.to_string()).as_deref(),
        Some("2026-07-28")
    );
    assert!(
        find(&kb, "1", None).is_some_and(|d| d.id() == "0001"),
        "a bare `1` should find 0001"
    );
    assert!(
        find(&kb, "0001-first", None).is_some(),
        "the slug finds it too"
    );
    assert_eq!(decisions_in(&kb, "demo").len(), 1);
    assert!(decisions_in(&kb, "other").is_empty());
}

#[test]
fn discovery_matches_the_type_case_insensitively() {
    let (_tmp, kb_root) = fixture();
    let dir = kb_root.join("bundles/demo/decisions");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("0001-first.md"),
        "---\ntype: decision record\ntitle: First\ndescription: \"D.\"\nstate: Accepted\ndecided: 2026-07-28\n---\n\n# F\n",
    )
    .unwrap();
    let kb = load(&kb_root);
    assert_eq!(decisions(&kb).len(), 1);
}

#[test]
fn id_normalization_treats_bare_padded_and_slugged_ids_alike() {
    let (_tmp, kb_root) = fixture();
    record(&kb_root, "demo", "0004-slugged", "Slugged", "Accepted", "");
    let kb = load(&kb_root);
    for needle in ["4", "0004", "0004-slugged"] {
        assert!(
            find(&kb, needle, None).is_some_and(|d| d.id() == "0004"),
            "`{needle}` should resolve"
        );
    }
}

// Ids are unique per bundle, not globally — `duplicates` only complains
// within one. Returning the first in sort order would show an unrelated
// decision with nothing to say a choice had been made.
#[test]
fn refuses_to_guess_when_an_id_means_a_record_in_more_than_one_bundle() {
    let (_tmp, kb_root) = fixture();
    new_bundle(
        &kb_root,
        "other",
        None,
        "Other",
        "A second scratch bundle.",
        "0.2",
        today(),
    )
    .unwrap();
    record(&kb_root, "demo", "0001-here", "Here", "Accepted", "");
    record(&kb_root, "other", "0001-there", "There", "Accepted", "");
    let kb = load(&kb_root);
    let matches = find_all(&kb, "0001", None);
    assert_eq!(matches.len(), 2, "both bundles number a record 0001");
    assert!(
        find(&kb, "0001", None).is_none(),
        "an ambiguous id resolves to nothing"
    );
    assert!(
        find(&kb, "0001", Some("other")).is_some_and(|d| d.slug() == "0001-there"),
        "--bundle disambiguates"
    );
    assert!(
        findings(&kb)
            .iter()
            .all(|f| f.check != "decision-duplicate-id"),
        "same id in two bundles is legal"
    );
    assert_eq!(
        ambiguous_message("0001", &matches),
        "`0001` names a decision record in 2 bundles — pass --bundle to choose:\n  demo  0001-here\n  other  0001-there"
    );
}

// An ADR mirrored from upstream is a decision record under upstream's
// conventions — `ADR-0001-…` for a filename, status and date in the body —
// so this register's schema is not its to satisfy.
#[test]
fn lists_a_mirrored_record_but_does_not_hold_it_to_this_registers_schema() {
    let (_tmp, kb_root) = fixture();
    fs::write(
        kb_root.join("bundles/demo/sync.yaml"),
        "repo: finos/morphir\nroot: sources\n",
    )
    .unwrap();
    let dir = kb_root.join("bundles/demo/sources/docs/adr");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("ADR-0001-upstream.md"),
        "---\ntype: Decision Record\ntitle: ADR 0001\ndescription: \"Upstream's own.\"\n---\n\nBody.\n",
    )
    .unwrap();
    let kb = load(&kb_root);
    assert!(
        decisions(&kb).iter().any(|d| d.doc.vendored),
        "still discovered as a decision record"
    );
    let fs_ = findings(&kb);
    assert!(
        fs_.is_empty(),
        "expected none, got {:?}",
        fs_.iter().map(|f| (&f.check, &f.path)).collect::<Vec<_>>()
    );
}

// -------------------------------------------------------------- supersession

#[test]
fn supersession_requires_a_successor_and_requires_it_to_exist() {
    let (_tmp, kb_root) = fixture();
    record(
        &kb_root,
        "demo",
        "0001-orphaned",
        "Orphaned",
        "Superseded",
        "",
    );
    record(
        &kb_root,
        "demo",
        "0002-dangling",
        "Dangling",
        "Superseded",
        "superseded_by: \"0099\"\n",
    );
    let kb = load(&kb_root);
    let fs_ = findings(&kb);
    let names: Vec<&str> = fs_.iter().map(|f| f.check.as_str()).collect();
    assert!(
        names.contains(&"decision-superseded-no-successor"),
        "got {names:?}"
    );
    assert!(
        names.contains(&"decision-superseded-unknown"),
        "got {names:?}"
    );
    let unknown = fs_
        .iter()
        .find(|f| f.check == "decision-superseded-unknown")
        .unwrap();
    assert_eq!(
        unknown.message,
        "`superseded_by: 0099` names no decision record in demo"
    );
}

#[test]
fn warns_when_supersession_is_not_mutual() {
    let (_tmp, kb_root) = fixture();
    // 0002 claims to replace 0001, but 0001 still reads as current — how a
    // chain silently breaks.
    record(&kb_root, "demo", "0001-old", "Old", "Accepted", "");
    record(
        &kb_root,
        "demo",
        "0002-new",
        "New",
        "Accepted",
        "supersedes: [\"0001\"]\n",
    );
    let kb = load(&kb_root);
    let fs_ = findings(&kb);
    let warn = fs_
        .iter()
        .find(|f| f.check == "decision-supersede-not-mutual")
        .unwrap_or_else(|| panic!("got {:?}", fs_.iter().map(|f| &f.check).collect::<Vec<_>>()));
    assert_eq!(
        warn.message,
        "this record supersedes 0001 but 0001 does not name it in `superseded_by`"
    );
    assert_eq!(
        warn.hint.as_deref(),
        Some("add `superseded_by: \"0002\"` and `state: Superseded` to 0001-old.md")
    );
}

// The mirror of the case above. Nothing anywhere carries a `supersedes`
// entry, so a forward-only check has no record to inspect and the one-way
// chain passes unreported.
#[test]
fn warns_when_only_the_retired_record_carries_the_link() {
    let (_tmp, kb_root) = fixture();
    record(
        &kb_root,
        "demo",
        "0001-old",
        "Old",
        "Superseded",
        "superseded_by: \"0002\"\n",
    );
    record(&kb_root, "demo", "0002-new", "New", "Accepted", "");
    let kb = load(&kb_root);
    let fs_ = findings(&kb);
    let warn = fs_
        .iter()
        .find(|f| f.check == "decision-supersede-not-mutual")
        .unwrap_or_else(|| panic!("got {:?}", fs_.iter().map(|f| &f.check).collect::<Vec<_>>()));
    assert_eq!(
        warn.message,
        "this record names 0002 in `superseded_by` but 0002 does not list 0001 in `supersedes`"
    );
    assert_eq!(
        warn.hint.as_deref(),
        Some("add `supersedes: [\"0001\"]` to 0002-new.md")
    );
}

// serde_yaml keeps an unquoted `2` as a number. A list accessor that kept
// only strings would drop it, and every supersession check downstream would
// then behave as if the field were absent.
#[test]
fn reads_an_unquoted_numeric_supersedes_entry() {
    let (_tmp, kb_root) = fixture();
    record(
        &kb_root,
        "demo",
        "0001-old",
        "Old",
        "Superseded",
        "superseded_by: \"0002\"\n",
    );
    record(
        &kb_root,
        "demo",
        "0002-new",
        "New",
        "Accepted",
        "supersedes: [1]\n",
    );
    let kb = load(&kb_root);
    let newer = find(&kb, "0002", None).unwrap();
    assert_eq!(newer.supersedes(), vec!["0001".to_string()]);
    let fs_ = findings(&kb);
    assert!(
        fs_.is_empty(),
        "expected none, got {:?}",
        fs_.iter().map(|f| &f.check).collect::<Vec<_>>()
    );
}

#[test]
fn is_silent_when_both_sides_agree() {
    let (_tmp, kb_root) = fixture();
    record(
        &kb_root,
        "demo",
        "0001-old",
        "Old",
        "Superseded",
        "superseded_by: \"0002\"\n",
    );
    record(
        &kb_root,
        "demo",
        "0002-new",
        "New",
        "Accepted",
        "supersedes: [\"0001\"]\n",
    );
    let kb = load(&kb_root);
    let fs_ = findings(&kb);
    assert!(
        fs_.is_empty(),
        "expected none, got {:?}",
        fs_.iter().map(|f| &f.check).collect::<Vec<_>>()
    );
}

#[test]
fn supersedes_must_name_a_real_record() {
    let (_tmp, kb_root) = fixture();
    record(
        &kb_root,
        "demo",
        "0002-new",
        "New",
        "Accepted",
        "supersedes: [\"0001\"]\n",
    );
    let kb = load(&kb_root);
    let fs_ = findings(&kb);
    let err = fs_
        .iter()
        .find(|f| f.check == "decision-supersedes-unknown")
        .unwrap();
    assert_eq!(
        err.message,
        "`supersedes: 0001` names no decision record in demo"
    );
}

// ---------------------------------------------------------------- validation

#[test]
fn rejects_an_unknown_state_a_duplicate_id_and_a_reasonless_withdrawal() {
    let (_tmp, kb_root) = fixture();
    record(&kb_root, "demo", "0001-a", "A", "Rejected", "");
    record(&kb_root, "demo", "0001-b", "B", "Accepted", "");
    record(&kb_root, "demo", "0003-c", "C", "Withdrawn", "");
    let kb = load(&kb_root);
    let fs_ = findings(&kb);
    let names: Vec<&str> = fs_.iter().map(|f| f.check.as_str()).collect();
    assert!(
        names.contains(&"decision-state-unknown"),
        "Rejected is not a state"
    );
    assert!(
        names.contains(&"decision-duplicate-id"),
        "two records both numbered 0001"
    );
    assert!(
        names.contains(&"decision-withdrawn-no-reason"),
        "got {names:?}"
    );
    let state = fs_
        .iter()
        .find(|f| f.check == "decision-state-unknown")
        .unwrap();
    assert_eq!(state.message, "`state: Rejected` is not a known state");
    assert_eq!(
        state.hint.as_deref(),
        Some("one of Proposed, Accepted, Superseded, Withdrawn")
    );
    let dup = fs_
        .iter()
        .find(|f| f.check == "decision-duplicate-id")
        .unwrap();
    assert_eq!(
        dup.message,
        "decision id `0001` is used by 2 records in demo"
    );
}

#[test]
fn reports_a_record_without_a_numeric_id_and_a_missing_decided_date() {
    let (_tmp, kb_root) = fixture();
    let dir = kb_root.join("bundles/demo/decisions");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("nameless.md"),
        "---\ntype: Decision Record\ntitle: Nameless\ndescription: \"D.\"\nstate: Accepted\n---\n\n# N\n",
    )
    .unwrap();
    let kb = load(&kb_root);
    let fs_ = findings(&kb);
    let names: Vec<&str> = fs_.iter().map(|f| f.check.as_str()).collect();
    assert!(names.contains(&"decision-no-id"), "got {names:?}");
    assert!(names.contains(&"decision-decided-missing"), "got {names:?}");
}

#[test]
fn states_know_which_are_retired() {
    assert!(DecisionState::Superseded.is_retired());
    assert!(DecisionState::Withdrawn.is_retired());
    assert!(!DecisionState::Accepted.is_retired());
    assert!(!DecisionState::Proposed.is_retired());
    assert_eq!(
        DecisionState::parse("accepted"),
        Some(DecisionState::Accepted)
    );
    assert_eq!(DecisionState::parse("in-force"), None);
}

// ----------------------------------------------------------------- rendering

#[test]
fn json_output_uses_the_scala_field_names() {
    let (_tmp, kb_root) = fixture();
    record(
        &kb_root,
        "demo",
        "0001-first",
        "First",
        "Superseded",
        "superseded_by: \"0002\"\n",
    );
    record(
        &kb_root,
        "demo",
        "0002-new",
        "New",
        "Accepted",
        "supersedes: [\"0001\"]\n",
    );
    let kb = load(&kb_root);
    let ds = decisions(&kb);
    let json = decision::render_list(&ds, true);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    let first = &value[0];
    for key in [
        "id",
        "slug",
        "title",
        "description",
        "bundle",
        "path",
        "state",
        "decided",
        "supersedes",
        "superseded_by",
        "reason",
        "tags",
    ] {
        assert!(first.get(key).is_some(), "missing key {key} in {first}");
    }
    assert_eq!(first["state"], "Superseded");
    assert_eq!(first["superseded_by"], "0002");
    assert_eq!(value[1]["supersedes"][0], "0001");
}

#[test]
fn text_list_groups_by_display_order() {
    let (_tmp, kb_root) = fixture();
    record(&kb_root, "demo", "0001-a", "A", "Proposed", "");
    record(&kb_root, "demo", "0002-b", "B", "Accepted", "");
    let kb = load(&kb_root);
    let ds = decisions(&kb);
    let text = decision::render_list(&ds, false);
    let accepted_at = text.find("\nAccepted (1)\n").expect("accepted section");
    let proposed_at = text.find("\nProposed (1)\n").expect("proposed section");
    assert!(accepted_at < proposed_at, "Accepted renders first: {text}");
    assert!(text.ends_with("\n2 decision record(s)\n"), "got: {text}");
    assert_eq!(decision::render_list(&[], false), "no decision records\n");
}

#[test]
fn text_show_renders_the_record() {
    let (_tmp, kb_root) = fixture();
    record(
        &kb_root,
        "demo",
        "0001-first",
        "First",
        "Accepted",
        "supersedes: [\"0000\"]\n",
    );
    let kb = load(&kb_root);
    // `supersedes: [0000]` names nothing, but show does not run the checks.
    let d = find(&kb, "0001", None).unwrap();
    let text = decision::render_show(&d, true, false);
    assert!(text.starts_with("0001 — First\n"), "got: {text}");
    assert!(text.contains("\nbundle:     demo\n"));
    assert!(text.contains("state:      Accepted\n"));
    assert!(text.contains("decided:    2026-07-28\n"));
    assert!(text.contains("supersedes: 0000\n"));
    assert!(text.contains("path:       demo:/decisions/0001-first.md\n"));
    assert!(text.contains("\nBody.\n"), "body requested: {text}");
}
