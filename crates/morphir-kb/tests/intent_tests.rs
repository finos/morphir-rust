//! Ports of the `KbIntentSpec`, the intent-facing `KbPathsSpec` cases, and
//! the `set_keys` cases from `KbScaffoldSpec` in `KbTests.scala`, plus pins
//! for the Rust-only `intent-duplicate-id` check (beads morphir-df9b) and
//! byte-exact template output.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{Duration, NaiveDate};
use morphir_kb::intent::{self, DocRef, Intent, IntentKind, IntentState, Transition, set_keys};
use morphir_kb::scaffold::new_bundle;
use morphir_okf::model::{Bundle, Kb, Severity};
use tempfile::TempDir;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 28).unwrap()
}

/// A minimal knowledge base: one ordinary bundle, optionally an intent bundle
/// alongside it.
fn fixture(with_intent: bool) -> (TempDir, PathBuf) {
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

/// A knowledge base with one intent already created, returned as its root.
fn with_one_intent(title: &str, kind: IntentKind) -> (TempDir, PathBuf) {
    let (tmp, kb_root) = fixture(true);
    let kb = load(&kb_root);
    let b = intent::find_bundle(&kb).unwrap();
    intent::create(b, title, "Something.", kind, false, None, &[], today()).unwrap();
    (tmp, kb_root)
}

fn intent_bundle(kb: &Kb) -> &Bundle {
    intent::find_bundle(kb).unwrap()
}

fn first(b: &Bundle) -> Intent<'_> {
    intent::find(b, "0001").unwrap()
}

// ---------------------------------------------------------------- vocabulary

#[test]
fn doc_ref_parses_bundle_and_path() {
    assert_eq!(
        DocRef::parse("morphir/morphir-scala:/x.md"),
        Some(DocRef {
            bundle: "morphir/morphir-scala".to_string(),
            path: "/x.md".to_string(),
        })
    );
}

#[test]
fn doc_ref_rejects_malformed_references() {
    assert_eq!(DocRef::parse("no-colon"), None);
    assert_eq!(DocRef::parse("bundle:relative.md"), None);
    // A Package URL — the two schemes are deliberately distinct.
    assert_eq!(DocRef::parse("pkg:maven/org/x@1.0"), None);
}

#[test]
fn state_parse_tolerates_case_and_hyphens() {
    assert_eq!(
        IntentState::parse("in-progress"),
        Some(IntentState::InProgress)
    );
    assert_eq!(IntentState::parse("RELEASED"), Some(IntentState::Released));
    assert_eq!(IntentState::parse("nonsense"), None);
}

#[test]
fn states_know_their_tier() {
    assert!(IntentState::Released.is_terminal());
    assert!(!IntentState::Backlog.is_terminal());
    // A backlog is meant to sit.
    assert!(!IntentState::Backlog.is_active());
    assert!(IntentState::Refinement.is_active() && IntentState::InProgress.is_active());
}

#[test]
fn kinds_carry_the_user_visible_tier() {
    assert!(IntentKind::parse("feature").unwrap().user_visible());
    assert!(!IntentKind::parse("build").unwrap().user_visible());
    for (label, visible) in [
        ("feature", true),
        ("bug", true),
        ("performance", true),
        ("security", true),
        ("deprecation", true),
        ("removal", true),
        ("refactor", false),
        ("docs", false),
        ("test", false),
        ("build", false),
        ("spike", false),
    ] {
        assert_eq!(
            IntentKind::parse(label).unwrap().user_visible(),
            visible,
            "kind {label}"
        );
    }
}

// ----------------------------------------------------------------- discovery

#[test]
fn finds_the_intent_bundle_by_its_marker_not_its_path() {
    let (_tmp, kb_root) = fixture(true);
    let kb = load(&kb_root);
    let b = intent::find_bundle(&kb).expect("intent bundle not found");
    let cfg = intent::config(b);
    assert_eq!(cfg.system.as_deref(), Some("pkg:pypi/demo"));
    assert_eq!(cfg.capability_bundle.as_deref(), Some("demo"));
    assert_eq!(cfg.stale_after_days, 60);
}

#[test]
fn allocates_ids_sequentially_and_finds_by_bare_number() {
    let (_tmp, kb_root) = with_one_intent("First", IntentKind::Feature);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    assert_eq!(intent::next_id(b), "0002");
    assert_eq!(
        intent::find(b, "1").map(|i| i.id()),
        Some("0001".to_string()),
        "findable by bare number"
    );
    assert!(intent::find(b, "0001-first").is_some(), "findable by slug");
}

// ----------------------------------------------------------------- templates

#[test]
fn init_bundle_index_template_is_byte_exact() {
    let (_tmp, kb_root) = fixture(true);
    let index = fs::read_to_string(kb_root.join("bundles/intent/index.md")).unwrap();
    assert_eq!(
        index,
        "---\n\
         okf_version: \"0.2\"\n\
         title: Intent\n\
         description: \"Work this project means to do, is doing, or has done — with the reasoning behind it.\"\n\
         intent: true\n\
         system: pkg:pypi/demo\n\
         capability_bundle: demo\n\
         stale_after_days: 60\n\
         ---\n\
         \n\
         # Intent\n\
         \n\
         Work this project means to do, is doing, or has done — with the reasoning behind it.\n\
         \n\
         Each entry is an Intent: future-tense, with a lifecycle. What the system actually *does* today lives\n\
         in the capability bundle, in the present tense. Releasing an Intent requires linking the Capability it\n\
         produced, which is what keeps the two in step.\n\
         \n\
         <!-- intent:index -->\n"
    );
    let log = fs::read_to_string(kb_root.join("bundles/intent/log.md")).unwrap();
    assert_eq!(
        log,
        "# Log\n\n## 2026-07-28\n\n* **Creation**: Intent bundle created.\n"
    );
}

#[test]
fn new_record_template_is_byte_exact() {
    let (_tmp, kb_root) = fixture(true);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    let file = intent::create(
        b,
        "Add thing",
        "Adds a thing.",
        IntentKind::Feature,
        true,
        Some("#42"),
        &["API Design".to_string()],
        today(),
    )
    .unwrap();
    assert!(file.ends_with("0001-add-thing.md"));
    let text = fs::read_to_string(&file).unwrap();
    assert_eq!(
        text,
        "---\n\
         type: Intent\n\
         title: Add thing\n\
         description: Adds a thing.\n\
         state: Backlog\n\
         kind: feature\n\
         breaking: true\n\
         created: 2026-07-28\n\
         state_since: 2026-07-28\n\
         issue: 42\n\
         tags: [api-design]\n\
         ---\n\
         \n\
         # 0001 — Add thing\n\
         \n\
         Adds a thing.\n\
         \n\
         ## Problem\n\
         \n\
         <!-- TODO: what problem is this solving? Resist describing a solution here. -->\n\
         \n\
         ## Approach\n\
         \n\
         <!-- TODO: fill in during Refinement. Delete if it stays trivial. -->\n"
    );
}

// ----------------------------------------------------------------- releasing

#[test]
fn releasing_demands_a_capability_for_user_visible_kinds() {
    let (_tmp, kb_root) = with_one_intent("Feature work", IntentKind::Feature);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    let err = intent::transition(
        &kb,
        b,
        &first(b),
        &Transition::to(IntentState::Released),
        today(),
    )
    .unwrap_err()
    .to_string();
    assert_eq!(
        err,
        "releasing needs --capability bundle:/path.md (the capability this produced)"
    );
}

#[test]
fn releasing_a_spike_asks_for_a_design_note() {
    let (_tmp, kb_root) = with_one_intent("Try it", IntentKind::Spike);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    // A spike is not user-visible, so the guard does not fire at all — but
    // the wording is pinned through the checker's Released branch instead.
    let res = intent::transition(
        &kb,
        b,
        &first(b),
        &Transition::to(IntentState::Released),
        today(),
    );
    assert!(
        res.is_ok(),
        "spikes are internal and may release bare: {res:?}"
    );
}

#[test]
fn releasing_rejects_a_malformed_capability_reference() {
    let (_tmp, kb_root) = with_one_intent("Feature work", IntentKind::Feature);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    let mut t = Transition::to(IntentState::Released);
    t.capability = Some("nope".to_string());
    let err = intent::transition(&kb, b, &first(b), &t, today())
        .unwrap_err()
        .to_string();
    assert_eq!(err, "`nope` is not `bundle-label:/path.md`");
}

#[test]
fn releasing_rejects_a_capability_that_resolves_to_nothing() {
    let (_tmp, kb_root) = with_one_intent("Feature work", IntentKind::Feature);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    let mut t = Transition::to(IntentState::Released);
    t.capability = Some("demo:/missing.md".to_string());
    let err = intent::transition(&kb, b, &first(b), &t, today())
        .unwrap_err()
        .to_string();
    assert_eq!(err, "`demo:/missing.md` names no concept in demo");
}

#[test]
fn releasing_rejects_a_capability_in_an_unknown_bundle() {
    let (_tmp, kb_root) = with_one_intent("Feature work", IntentKind::Feature);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    let mut t = Transition::to(IntentState::Released);
    t.capability = Some("nowhere:/x.md".to_string());
    let err = intent::transition(&kb, b, &first(b), &t, today())
        .unwrap_err()
        .to_string();
    assert!(
        err.starts_with("no bundle `nowhere` (known: "),
        "got: {err}"
    );
}

#[test]
fn releasing_needs_no_capability_for_internal_kinds() {
    // Internal work often changes nothing a reader of the knowledge base
    // needs to know; inventing a document for "added three release labels"
    // would be exactly the noise the design avoids.
    let (_tmp, kb_root) = with_one_intent("Build work", IntentKind::Build);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    let res = intent::transition(
        &kb,
        b,
        &first(b),
        &Transition::to(IntentState::Released),
        today(),
    );
    assert!(
        res.is_ok(),
        "internal kinds may release without a capability: {res:?}"
    );
    let text = fs::read_to_string(first(b).doc.file.clone()).unwrap();
    assert!(text.contains("state: Released"));
    assert!(text.contains("state_since: 2026-07-28"));
}

#[test]
fn releasing_records_capability_and_artifacts() {
    let (_tmp, kb_root) = with_one_intent("Feature work", IntentKind::Feature);
    // Give the demo bundle a capability document to point at.
    fs::write(
        kb_root.join("bundles/demo/cap.md"),
        "---\ntype: Capability\ntitle: Cap\ndescription: Does.\n---\n\n# Cap\n",
    )
    .unwrap();
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    let mut t = Transition::to(IntentState::Released);
    t.capability = Some("demo:/cap.md".to_string());
    t.artifacts = vec![
        "pkg:cargo/morphir@0.2.0".to_string(),
        "pkg:pypi/demo".to_string(),
    ];
    intent::transition(&kb, b, &first(b), &t, today()).unwrap();
    let text = fs::read_to_string(first(b).doc.file.clone()).unwrap();
    assert!(text.contains("capability: demo:/cap.md"));
    assert!(text.contains("artifacts: [pkg:cargo/morphir@0.2.0, pkg:pypi/demo]"));
}

// -------------------------------------------------- other terminal states

#[test]
fn cancelling_demands_a_reason() {
    let (_tmp, kb_root) = with_one_intent("A thing", IntentKind::Feature);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    let err = intent::transition(
        &kb,
        b,
        &first(b),
        &Transition::to(IntentState::Cancelled),
        today(),
    )
    .unwrap_err()
    .to_string();
    assert_eq!(err, "cancelling needs --reason");
}

#[test]
fn superseding_demands_a_successor() {
    let (_tmp, kb_root) = with_one_intent("A thing", IntentKind::Feature);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    let err = intent::transition(
        &kb,
        b,
        &first(b),
        &Transition::to(IntentState::Superseded),
        today(),
    )
    .unwrap_err()
    .to_string();
    assert_eq!(err, "superseding needs --by <intent-id>");
}

#[test]
fn superseding_demands_a_successor_that_exists() {
    let (_tmp, kb_root) = with_one_intent("A thing", IntentKind::Feature);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    let mut t = Transition::to(IntentState::Superseded);
    t.superseded_by = Some("0099".to_string());
    let err = intent::transition(&kb, b, &first(b), &t, today())
        .unwrap_err()
        .to_string();
    assert_eq!(err, "no intent `0099` in intent");
}

// ------------------------------------------------------- frontmatter editing

#[test]
fn set_keys_appends_after_a_trailing_block_rather_than_inside_it() {
    // Regression in Scala: anchoring on "the last top-level line" put the new
    // key between `sources:` and its children, corrupting the YAML and the
    // provenance with it.
    let tmp = TempDir::new().unwrap();
    let f = tmp.path().join("doc.md");
    fs::write(
        &f,
        "---\ntype: Intent\nstate: Backlog\nsources:\n  - id: s1\n    resource: https://x/y.md\n---\n\nbody\n",
    )
    .unwrap();
    set_keys(
        &f,
        &[("capability".to_string(), Some("demo:/cap.md".to_string()))],
    )
    .unwrap();
    let text = fs::read_to_string(&f).unwrap();
    assert_eq!(
        text,
        "---\ntype: Intent\nstate: Backlog\nsources:\n  - id: s1\n    resource: https://x/y.md\ncapability: demo:/cap.md\n---\n\nbody\n"
    );
    // And the YAML still parses, sources intact.
    let (raw, _) = morphir_okf::split_frontmatter(&text);
    let fm = morphir_okf::parse_frontmatter(&raw.unwrap()).unwrap();
    assert_eq!(fm.str_at("capability").as_deref(), Some("demo:/cap.md"));
    assert_eq!(fm.sources().len(), 1);
    assert_eq!(fm.sources()[0].resource, "https://x/y.md");
}

#[test]
fn set_keys_replaces_an_existing_key_in_place_and_leaves_the_body_alone() {
    let tmp = TempDir::new().unwrap();
    let f = tmp.path().join("doc.md");
    fs::write(
        &f,
        "---\ntype: Intent\nstate: Backlog\n---\n\n# Body\n\nprose\n",
    )
    .unwrap();
    set_keys(&f, &[("state".to_string(), Some("Released".to_string()))]).unwrap();
    let text = fs::read_to_string(&f).unwrap();
    assert_eq!(
        text,
        "---\ntype: Intent\nstate: Released\n---\n\n# Body\n\nprose\n"
    );
}

#[test]
fn set_keys_removes_a_key_and_its_indented_continuation() {
    let tmp = TempDir::new().unwrap();
    let f = tmp.path().join("doc.md");
    fs::write(
        &f,
        "---\ntype: Intent\nsources:\n  - id: s1\n    resource: https://x/y.md\nstate: Backlog\n---\n\nbody\n",
    )
    .unwrap();
    set_keys(&f, &[("sources".to_string(), None)]).unwrap();
    let text = fs::read_to_string(&f).unwrap();
    assert_eq!(text, "---\ntype: Intent\nstate: Backlog\n---\n\nbody\n");
}

#[test]
fn set_keys_never_touches_a_nested_line_that_shares_the_key_name() {
    // Only column-zero `key:` lines are considered; `  - id: s1` must not be
    // mistaken for an `id:` key.
    let tmp = TempDir::new().unwrap();
    let f = tmp.path().join("doc.md");
    fs::write(
        &f,
        "---\ntype: Intent\nsources:\n  - resource: https://x/y.md\n    title: T\n---\n\nbody\n",
    )
    .unwrap();
    set_keys(&f, &[("title".to_string(), Some("New".to_string()))]).unwrap();
    let text = fs::read_to_string(&f).unwrap();
    assert!(text.contains("    title: T\n"), "nested title untouched");
    assert!(
        text.contains("\ntitle: New\n"),
        "new top-level title appended"
    );
}

#[test]
fn set_keys_refuses_a_document_without_frontmatter() {
    let tmp = TempDir::new().unwrap();
    let f = tmp.path().join("doc.md");
    fs::write(&f, "# No frontmatter\n").unwrap();
    let err = set_keys(&f, &[("state".to_string(), Some("Released".to_string()))])
        .unwrap_err()
        .to_string();
    assert!(err.ends_with("has no frontmatter to edit"), "got: {err}");
}

// ----------------------------------------------------------- generated index

#[test]
fn generated_index_groups_by_state_and_keeps_the_preamble() {
    let (_tmp, kb_root) = with_one_intent("Open thing", IntentKind::Feature);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    assert!(intent::generate_index(b, today()).unwrap());
    let text = fs::read_to_string(&b.index.file).unwrap();
    assert!(text.contains(intent::MARKER), "marker retained");
    assert!(
        text.contains(
            "_Generated by `kb refresh` — do not edit below the marker. Last built 2026-07-28._"
        ),
        "header present"
    );
    assert!(
        text.contains("## Backlog (1)"),
        "grouped heading with a count"
    );
    assert!(
        text.contains("* [0001 Open thing — feature](/0001-open-thing.md) - Something.\n"),
        "flags in the link text, description verbatim: {text}"
    );
    // Flags belong in the link text, never after the description — otherwise
    // kb check sees drift.
    assert!(
        !text.contains("- Something. _("),
        "no flags after the description"
    );
}

#[test]
fn generated_index_regeneration_is_idempotent() {
    let (_tmp, kb_root) = with_one_intent("Open thing", IntentKind::Feature);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    assert!(
        intent::generate_index(b, today()).unwrap(),
        "first build writes"
    );
    let after_first = fs::read_to_string(&b.index.file).unwrap();
    // Reload so the in-memory index doc matches what is now on disk.
    let kb2 = load(&kb_root);
    let b2 = intent_bundle(&kb2);
    assert!(
        !intent::generate_index(b2, today()).unwrap(),
        "second build is a no-op"
    );
    assert_eq!(fs::read_to_string(&b2.index.file).unwrap(), after_first);
}

#[test]
fn generated_index_reports_an_empty_register() {
    let (_tmp, kb_root) = fixture(true);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    intent::generate_index(b, today()).unwrap();
    let text = fs::read_to_string(&b.index.file).unwrap();
    assert!(text.contains(
        "No intent recorded yet. `kb intent new --title … --description … --kind feature`"
    ));
}

// --------------------------------------------------------------------- check

fn checks(kb_root: &Path) -> Vec<morphir_okf::model::Finding> {
    let kb = load(kb_root);
    let b = intent_bundle(&kb);
    intent::check(&kb, b, today())
}

#[test]
fn check_reports_a_released_intent_with_no_capability() {
    let (_tmp, kb_root) = with_one_intent("Stuck", IntentKind::Feature);
    {
        let kb = load(&kb_root);
        let i = first(intent_bundle(&kb));
        // Hand-edited into an impossible state, to exercise the checker
        // rather than the transition guard.
        set_keys(
            &i.doc.file,
            &[("state".to_string(), Some("Released".to_string()))],
        )
        .unwrap();
    }
    let findings = checks(&kb_root);
    assert!(
        findings
            .iter()
            .any(|f| f.check == "intent-released-no-capability" && f.severity == Severity::Error),
        "got: {:?}",
        findings.iter().map(|f| &f.check).collect::<Vec<_>>()
    );
}

#[test]
fn check_warns_when_active_work_has_not_moved() {
    let (_tmp, kb_root) = with_one_intent("Stuck", IntentKind::Feature);
    {
        let kb = load(&kb_root);
        let i = first(intent_bundle(&kb));
        set_keys(
            &i.doc.file,
            &[
                ("state".to_string(), Some("Refinement".to_string())),
                ("state_since".to_string(), Some("2026-01-01".to_string())),
            ],
        )
        .unwrap();
    }
    let findings = checks(&kb_root);
    assert!(
        findings
            .iter()
            .any(|f| f.check == "intent-stale" && f.severity == Severity::Warn),
        "got: {:?}",
        findings.iter().map(|f| &f.check).collect::<Vec<_>>()
    );
}

#[test]
fn check_staleness_fires_strictly_above_the_threshold() {
    // days == stale_after_days is not stale; days == stale_after_days + 1 is.
    for (days_ago, expect_stale) in [(60i64, false), (61, true)] {
        let (_tmp, kb_root) = with_one_intent("Edge", IntentKind::Feature);
        let since = (today() - Duration::days(days_ago)).to_string();
        {
            let kb = load(&kb_root);
            let i = first(intent_bundle(&kb));
            set_keys(
                &i.doc.file,
                &[
                    ("state".to_string(), Some("InProgress".to_string())),
                    ("state_since".to_string(), Some(since.clone())),
                ],
            )
            .unwrap();
        }
        let stale = checks(&kb_root).iter().any(|f| f.check == "intent-stale");
        assert_eq!(stale, expect_stale, "{days_ago} days ago (since {since})");
    }
}

#[test]
fn check_calls_the_spike_target_a_design_note() {
    // For a released spike without a capability the checker warns rather
    // than errs — spikes are internal — and the target it names is the
    // Design Note wording, not "Capability".
    let (_tmp, kb_root) = with_one_intent("Try it", IntentKind::Spike);
    {
        let kb = load(&kb_root);
        let i = first(intent_bundle(&kb));
        set_keys(
            &i.doc.file,
            &[("state".to_string(), Some("Released".to_string()))],
        )
        .unwrap();
    }
    let findings = checks(&kb_root);
    let warn = findings
        .iter()
        .find(|f| f.check == "intent-released-no-capability-internal")
        .expect("internal warn");
    assert_eq!(warn.severity, Severity::Warn);
    assert_eq!(
        warn.message,
        "Released spike intent links to no Design Note"
    );
}

#[test]
fn check_never_calls_a_backlog_item_stale() {
    let (_tmp, kb_root) = with_one_intent("Waiting", IntentKind::Feature);
    {
        let kb = load(&kb_root);
        let i = first(intent_bundle(&kb));
        set_keys(
            &i.doc.file,
            &[("state_since".to_string(), Some("2026-01-01".to_string()))],
        )
        .unwrap();
    }
    assert!(!checks(&kb_root).iter().any(|f| f.check == "intent-stale"));
}

#[test]
fn check_reports_duplicate_numeric_id_prefixes() {
    // The addition over the Scala tool (beads morphir-df9b): `find` by id
    // would silently pick one of the two files.
    let (_tmp, kb_root) = fixture(true);
    let record = "---\ntype: Intent\ntitle: T\ndescription: D.\nstate: Backlog\nkind: feature\nbreaking: false\ncreated: 2026-07-28\nstate_since: 2026-07-28\n---\n\n# T\n";
    fs::write(kb_root.join("bundles/intent/0001-a.md"), record).unwrap();
    fs::write(kb_root.join("bundles/intent/0001-b.md"), record).unwrap();
    let findings = checks(&kb_root);
    let dups: Vec<_> = findings
        .iter()
        .filter(|f| f.check == "intent-duplicate-id")
        .collect();
    assert_eq!(
        dups.len(),
        2,
        "one finding per colliding record: {findings:?}"
    );
    assert!(dups.iter().all(|f| f.severity == Severity::Error));
    assert!(
        dups[0]
            .message
            .contains("intent id `0001` is used by 2 records in intent"),
        "got: {}",
        dups[0].message
    );
}

#[test]
fn check_reports_shape_problems_and_bundle_config() {
    let (_tmp, kb_root) = fixture(true);
    fs::write(
        kb_root.join("bundles/intent/0001-shapeless.md"),
        "---\ntype: Intent\ntitle: Shapeless\ndescription: D.\nstate: Sideways\n---\n\n# S\n",
    )
    .unwrap();
    // Drop the bundle config to trigger the bundle-level warnings.
    set_keys(
        &kb_root.join("bundles/intent/index.md"),
        &[
            ("system".to_string(), None),
            ("capability_bundle".to_string(), None),
        ],
    )
    .unwrap();
    let findings = checks(&kb_root);
    let by_check = |c: &str| findings.iter().find(|f| f.check == c);
    let state = by_check("intent-state-missing").expect("state finding");
    assert_eq!(state.message, "`state: Sideways` is not a known state");
    assert_eq!(
        state.hint.as_deref(),
        Some("one of Backlog, Refinement, InProgress, Released, Cancelled, Superseded")
    );
    let kind = by_check("intent-kind-missing").expect("kind finding");
    assert_eq!(kind.message, "intent has no `kind`");
    assert_eq!(
        kind.hint.as_deref(),
        Some(
            "one of feature, bug, performance, security, deprecation, removal, refactor, docs, test, build, spike"
        )
    );
    assert!(by_check("intent-created-missing").is_some());
    assert!(by_check("intent-state-since-missing").is_some());
    assert!(by_check("intent-no-system").is_some());
    assert!(by_check("intent-no-capability-bundle").is_some());
}

#[test]
fn check_reports_cancelled_and_superseded_obligations() {
    let (_tmp, kb_root) = fixture(true);
    let base = "---\ntype: Intent\ntitle: T\ndescription: D.\nkind: feature\nbreaking: false\ncreated: 2026-07-28\nstate_since: 2026-07-28\n";
    fs::write(
        kb_root.join("bundles/intent/0001-cancelled.md"),
        format!("{base}state: Cancelled\n---\n\n# T\n"),
    )
    .unwrap();
    fs::write(
        kb_root.join("bundles/intent/0002-superseded.md"),
        format!("{base}state: Superseded\n---\n\n# T\n"),
    )
    .unwrap();
    fs::write(
        kb_root.join("bundles/intent/0003-superseded-unknown.md"),
        format!("{base}state: Superseded\nsuperseded_by: \"0099\"\n---\n\n# T\n"),
    )
    .unwrap();
    fs::write(
        kb_root.join("bundles/intent/0004-superseded-ok.md"),
        format!("{base}state: Superseded\nsuperseded_by: \"1\"\n---\n\n# T\n"),
    )
    .unwrap();
    let findings = checks(&kb_root);
    let names: Vec<&str> = findings.iter().map(|f| f.check.as_str()).collect();
    assert!(
        names.contains(&"intent-cancelled-no-reason"),
        "got {names:?}"
    );
    assert!(
        names.contains(&"intent-superseded-no-successor"),
        "got {names:?}"
    );
    assert!(
        names.contains(&"intent-superseded-unknown"),
        "got {names:?}"
    );
    // 0004 points at `1`, which pads to intent 0001 — no finding for it.
    assert!(
        !findings
            .iter()
            .any(|f| f.check == "intent-superseded-unknown" && f.path.contains("0004")),
        "a bare `1` names 0001"
    );
}

#[test]
fn check_warns_on_non_purl_artifacts() {
    let (_tmp, kb_root) = fixture(true);
    fs::write(
        kb_root.join("bundles/intent/0001-shipped.md"),
        "---\ntype: Intent\ntitle: T\ndescription: D.\nstate: Backlog\nkind: feature\nbreaking: false\ncreated: 2026-07-28\nstate_since: 2026-07-28\nartifacts: [pkg:cargo/morphir@0.2.0, not-a-purl]\n---\n\n# T\n",
    )
    .unwrap();
    let findings = checks(&kb_root);
    let warns: Vec<_> = findings
        .iter()
        .filter(|f| f.check == "intent-artifact-not-purl")
        .collect();
    assert_eq!(warns.len(), 1, "got {findings:?}");
    assert_eq!(
        warns[0].message,
        "artifact `not-a-purl` is not a Package URL"
    );
}

// ----------------------------------------------------------------- rendering

#[test]
fn json_output_uses_the_scala_field_names() {
    let (_tmp, kb_root) = with_one_intent("Open thing", IntentKind::Feature);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    let items = intent::intents(b);
    let json = intent::render_list(b, &items, true);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["bundle"], "intent");
    assert_eq!(value["count"], 1);
    let i = &value["intents"][0];
    for key in [
        "id",
        "slug",
        "path",
        "title",
        "description",
        "state",
        "kind",
        "userVisible",
        "breaking",
        "created",
        "stateSince",
        "issue",
        "capability",
        "supersededBy",
        "artifacts",
    ] {
        assert!(i.get(key).is_some(), "missing key {key} in {i}");
    }
    assert_eq!(i["state"], "Backlog");
    assert_eq!(i["kind"], "feature");
    assert_eq!(i["userVisible"], true);
    assert_eq!(i["issue"], serde_json::Value::Null);
}

#[test]
fn text_list_groups_by_display_order_and_counts() {
    let (_tmp, kb_root) = with_one_intent("Open thing", IntentKind::Feature);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    let items = intent::intents(b);
    let text = intent::render_list(b, &items, false);
    assert!(text.contains("\nBACKLOG (1)\n"), "got: {text}");
    assert!(text.contains("  0001   Open thing"), "got: {text}");
    assert!(text.ends_with("\n1 intent\n"), "got: {text}");
    assert_eq!(intent::render_list(b, &[], false), "no matching intent\n");
}

#[test]
fn text_show_renders_the_record() {
    let (_tmp, kb_root) = with_one_intent("Open thing", IntentKind::Feature);
    let kb = load(&kb_root);
    let b = intent_bundle(&kb);
    let i = first(b);
    let text = intent::render_show(&kb, &i, false);
    assert!(
        text.starts_with("intent 0001 — Open thing\n"),
        "got: {text}"
    );
    assert!(text.contains("Something.\n"));
    assert!(text.contains("\nstate        Backlog  since 2026-07-28\n"));
    assert!(text.contains("kind         feature\n"));
    assert!(text.contains("created      2026-07-28\n"));
    assert!(text.contains("file         kb/bundles/intent/0001-open-thing.md\n"));
}
