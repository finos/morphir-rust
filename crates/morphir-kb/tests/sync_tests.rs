//! The vendoring engine, and above all its one load-bearing invariant.
//!
//! `project(inject(bytes)) == bytes` is what makes an export safe. If it slips,
//! the knowledge base silently rewrites somebody else's repository — so the corpus
//! below is deliberately hostile: CRLF, nested YAML blocks, fractional numbers,
//! `---` in the body, and no frontmatter at all.
//!
//! Ported case-for-case from `KbSyncSpec` in `KbTests.scala`, plus the extra state
//! machine, lockfile stability and generated-index coverage the port spec demands.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use morphir_kb::sync::{self, FileStatus, LockEntry, SyncKind, SyncLock, SyncState};
use morphir_okf::{DocKind, Kb, Severity, store};
use tempfile::TempDir;

// ------------------------------------------------------------------ helpers

/// Documents that have all previously been plausible ways to break the projection.
fn corpus() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "plain frontmatter",
            "---\ntitle: Types\ndescription: The type system\n---\n\n# Types\n\nProse.\n",
        ),
        (
            "no frontmatter at all",
            "# Agents\n\nA working agreement.\n",
        ),
        (
            "CRLF throughout",
            "---\r\ntitle: Types\r\n---\r\n\r\n# Types\r\n",
        ),
        (
            "nested block",
            "---\ntitle: IR v4\nstatus: partial\ntracking:\n  beads: [morphir-8fx]\n  github_issues: [398]\n---\n\nBody.\n",
        ),
        (
            "fractional sidebar_position",
            "---\ntitle: Attributes\nsidebar_position: 2.5\n---\n\nBody.\n",
        ),
        (
            "body contains a fence",
            "---\ntitle: X\n---\n\nBefore.\n\n---\n\nAfter.\n",
        ),
        ("empty frontmatter block", "---\n---\n\nBody.\n"),
        (
            "no trailing newline",
            "---\ntitle: X\n---\n\nBody without a newline",
        ),
        (
            "unterminated frontmatter",
            "---\ntitle: never closed\n\n# Body\n",
        ),
        // Beyond the Scala corpus: the whole-block variant must also survive CRLF.
        ("CRLF without frontmatter", "# Agents\r\n\r\nProse.\r\n"),
    ]
}

fn keys() -> Vec<(String, String)> {
    vec![
        ("type".to_string(), "Specification Source".to_string()),
        (
            "kb_upstream".to_string(),
            "docs/spec/draft/types.md".to_string(),
        ),
    ]
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 28).unwrap()
}

fn write_file(p: &Path, content: &str) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

fn write_bin(p: &Path, bytes: &[u8]) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, bytes).unwrap();
}

const FIXTURE_MANIFEST: &str = "upstream:\n  repo: acme/spec\n  refs_path: acme/spec\nroot: sources\nmappings:\n  - \"docs/**\"\n  - \"schemas/**\"\ntype_map:\n  \"docs/**\": Specification Source\n";

/// A knowledge base with one sync bundle, plus a fake upstream checkout to pull
/// from — the same shape as the Scala `syncFixture`.
fn sync_fixture() -> (TempDir, PathBuf, PathBuf, PathBuf) {
    let dir = TempDir::new().unwrap();
    let kb_root = dir.path().join("kb");
    let bundle_root = kb_root.join("bundles").join("vendored");
    let upstream = dir.path().join("upstream");
    write_file(
        &bundle_root.join("index.md"),
        "---\nokf_version: \"0.2\"\ntitle: Vendored\ndescription: Mirrored upstream material.\nsync: true\n---\n\n# Vendored\n\nMirrored upstream material.\n\n## Orientation\n",
    );
    write_file(&bundle_root.join("sync.yaml"), FIXTURE_MANIFEST);
    write_file(
        &upstream.join("docs").join("types.md"),
        "---\ntitle: Types\ndescription: The types.\n---\n\n# Types\n",
    );
    write_file(
        &upstream.join("docs").join("index.md"),
        "---\ntitle: Docs\ndescription: The docs.\n---\n\n# Docs\n",
    );
    write_file(
        &upstream.join("schemas").join("thing.yaml"),
        "$id: thing\ntype: object\n",
    );
    (dir, kb_root, bundle_root, upstream)
}

fn load_sync(kb_root: &Path) -> (Kb, sync::SyncBundle) {
    let kb = store::load(kb_root).unwrap();
    let b = sync::find_bundle(&kb, None).expect("a bundle declares sync: true");
    let sb = sync::load(b).unwrap();
    (kb, sb)
}

/// First pull plus lockfile write — most scenarios start from a settled mirror.
fn seeded(kb_root: &Path, upstream: &Path) -> sync::SyncBundle {
    let (_, sb0) = load_sync(kb_root);
    let result = sync::pull(&sb0, upstream, "deadbeef", today(), false, false, false).unwrap();
    sync::write_lock(&sb0, &result.lock).unwrap();
    load_sync(kb_root).1
}

/// Rewrites the fixture's manifest with a different `type_map`, which is the edit
/// that used to have no effect in the reference implementation.
fn retype(bundle_root: &Path, t: &str) {
    write_file(
        &bundle_root.join("sync.yaml"),
        &format!(
            "upstream:\n  repo: acme/spec\n  refs_path: acme/spec\nroot: sources\nmappings:\n  - \"docs/**\"\n  - \"schemas/**\"\ntype_map:\n  \"docs/**\": {t}\n"
        ),
    );
}

fn parse_manifest(raw: &str) -> sync::SyncManifest {
    sync::parse_manifest(raw).unwrap()
}

fn state_of<'a>(rows: &'a [FileStatus], path: &str) -> &'a FileStatus {
    rows.iter().find(|r| r.path == path).unwrap()
}

// -------------------------------------------------------- round-trip invariant

#[test]
fn roundtrip_survives_the_corpus() {
    for (label, text) in corpus() {
        let injected = sync::inject(text, &keys());
        assert_eq!(
            sync::project(&injected),
            Ok(text.to_string()),
            "round-trip changed the bytes for {label}"
        );
    }
}

#[test]
fn inject_produces_keys_that_parse_as_yaml() {
    let injected = sync::inject(corpus()[0].1, &keys());
    let split = sync::split(&injected).unwrap();
    let fm = morphir_okf::parse_frontmatter(&split.fm).unwrap();
    assert_eq!(fm.doc_type().as_deref(), Some("Specification Source"));
    assert_eq!(
        fm.title().as_deref(),
        Some("Types"),
        "upstream's own keys must survive injection"
    );
}

#[test]
fn project_leaves_a_never_injected_document_alone() {
    assert_eq!(sync::project(corpus()[0].1), Ok(corpus()[0].1.to_string()));
}

#[test]
fn project_removes_whole_block_when_upstream_had_no_frontmatter() {
    let injected = sync::inject("# Agents\n\nProse.\n", &keys());
    assert!(
        injected.starts_with("---\n"),
        "a block is added to carry the keys"
    );
    assert_eq!(
        sync::project(&injected),
        Ok("# Agents\n\nProse.\n".to_string())
    );
}

#[test]
fn project_refuses_a_damaged_fence_rather_than_guessing() {
    let broken = "---\ntitle: X\n# kb:begin\ntype: Y\n---\n\nBody.\n";
    assert!(
        sync::project(broken).is_err(),
        "an unclosed fence must not silently export"
    );
}

// ------------------------------------------------------------- injected keys

#[test]
fn injected_keys_supplies_title_and_description_only_when_upstream_omits_them() {
    let m = parse_manifest("upstream:\n  repo: acme/spec\nmappings:\n  - docs/**\n");
    let with_both = sync::injected_keys(&m, "docs/a.md", "---\ntitle: A\ndescription: B\n---\n");
    let without = sync::injected_keys(&m, "docs/some-file.md", "# No frontmatter\n");
    let names = |ks: &[(String, String)]| ks.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>();
    assert_eq!(names(&with_both), vec!["type", "kb_upstream"]);
    assert_eq!(
        names(&without),
        vec!["type", "title", "description", "kb_upstream"]
    );
    assert_eq!(
        without
            .iter()
            .find(|(k, _)| k == "title")
            .map(|(_, v)| v.as_str()),
        Some("Some File")
    );
}

// -------------------------------------------------------------- re-injection

fn reinjection_manifest() -> sync::SyncManifest {
    parse_manifest(
        "upstream:\n  repo: acme/spec\nmappings:\n  - \"docs/**\"\ntype_map:\n  \"docs/**\": Design Source\n",
    )
}

#[test]
fn reinject_rewrites_the_keys_the_injection_owns() {
    let manifest = reinjection_manifest();
    let before = sync::inject(
        "---\ntitle: Types\ndescription: The types.\n---\n\n# Types\n",
        &keys(),
    );
    let after = sync::reinjected(&manifest, "docs/types.md", &before).unwrap();
    assert!(after.contains("type: Design Source"), "got: {after}");
    assert!(
        !after.contains("Specification Source"),
        "the old value goes"
    );
    assert!(
        after.contains("title: Types"),
        "upstream's own keys are outside the fence and untouched"
    );
}

#[test]
fn reinject_keeps_keys_a_human_added_inside_the_fence() {
    let manifest = reinjection_manifest();
    let before = sync::inject("---\ntitle: Types\n---\n\n# Types\n", &keys())
        .replace("# kb:end", "reviewed_by: us\n# kb:end");
    let after = sync::reinjected(&manifest, "docs/types.md", &before).unwrap();
    assert!(
        after.contains("reviewed_by: us"),
        "a hand-added key must survive: {after}"
    );
    assert!(
        after.contains("type: Design Source"),
        "while the generated ones are recomputed"
    );
}

#[test]
fn reinject_leaves_the_upstream_form_exactly_as_it_was() {
    // The whole point: re-injection is a change to our block, never to the bytes
    // an export would send.
    let manifest = reinjection_manifest();
    for (label, text) in corpus() {
        let injected = sync::inject(text, &keys());
        let after = sync::reinjected(&manifest, "docs/types.md", &injected).unwrap();
        assert_eq!(
            sync::project(&after),
            Ok(text.to_string()),
            "round-trip changed the bytes for {label}"
        );
    }
}

#[test]
fn reinject_is_idempotent_so_a_second_pull_finds_nothing_to_do() {
    let manifest = reinjection_manifest();
    let once = sync::reinjected(
        &manifest,
        "docs/types.md",
        &sync::inject(corpus()[0].1, &keys()),
    )
    .unwrap();
    assert!(
        !sync::injection_stale(&manifest, "docs/types.md", &once),
        "the first pass settles it"
    );
    assert_eq!(
        sync::reinjected(&manifest, "docs/types.md", &once),
        Ok(once)
    );
}

#[test]
fn reinject_gives_a_fence_to_a_mirrored_file_that_has_lost_one() {
    let manifest = reinjection_manifest();
    let after = sync::reinjected(
        &manifest,
        "docs/types.md",
        "---\ntitle: Types\n---\n\n# Types\n",
    )
    .unwrap();
    assert!(after.contains("# kb:begin"), "got: {after}");
    assert!(after.contains("type: Design Source"));
}

#[test]
fn reinject_refuses_a_damaged_fence_rather_than_rewriting_it() {
    let manifest = reinjection_manifest();
    let broken = "---\ntitle: X\n# kb:begin\ntype: Y\n---\n\nBody.\n";
    assert!(sync::reinject(broken, &keys()).is_err());
    assert!(
        !sync::injection_stale(&manifest, "docs/types.md", broken),
        "and it is reported as unreadable, not stale"
    );
}

#[test]
fn reinject_drops_an_injected_fallback_upstream_has_since_supplied_itself() {
    // The file was seeded when upstream had neither key, and upstream has since
    // added both. Keeping ours would leave `title` in the frontmatter twice, which
    // a duplicate-rejecting parser refuses outright — the document would stop
    // loading.
    let manifest = reinjection_manifest();
    let stale = "---\ntitle: Types\ndescription: Theirs.\n\
                 # kb:begin — added by the knowledge base; removed on export\n\
                 type: Design Source\ntitle: Types\n\
                 description: \"Upstream source document acme/spec:docs/types.md.\"\n\
                 kb_upstream: docs/types.md\n# kb:end\n---\n\n# Types\n";
    assert!(
        sync::injection_stale(&manifest, "docs/types.md", stale),
        "a duplicated key is staleness by any reading"
    );
    let after = sync::reinjected(&manifest, "docs/types.md", stale).unwrap();
    assert_eq!(
        after.lines().filter(|l| l.starts_with("title: ")).count(),
        1,
        "one title, not two: {after}"
    );
    assert!(
        !after.contains("Upstream source document"),
        "and the fallback description goes with it: {after}"
    );
    let fm = sync::split(&after)
        .and_then(|s| morphir_okf::parse_frontmatter(&s.fm).ok())
        .unwrap();
    assert_eq!(
        fm.description().as_deref(),
        Some("Theirs."),
        "leaving upstream's own"
    );
}

// -------------------------------------------------------------------- globs

#[test]
fn glob_double_star_spans_directories_and_also_matches_zero_of_them() {
    assert!(sync::glob_matches("docs/**", "docs/spec/draft/types.md"));
    assert!(
        sync::glob_matches("docs/**/x.md", "docs/x.md"),
        "**/ must match zero directories"
    );
    assert!(!sync::glob_matches("docs/**", "wit/types.wit"));
}

#[test]
fn glob_single_star_stops_at_a_separator() {
    assert!(sync::glob_matches(
        "website/static/schemas/morphir-*.yaml",
        "website/static/schemas/morphir-ir-v4.yaml"
    ));
    assert!(!sync::glob_matches("docs/*.md", "docs/spec/types.md"));
}

#[test]
fn glob_dot_is_literal_not_any_character() {
    assert!(!sync::glob_matches("docs/a.md", "docs/axmd"));
}

// ----------------------------------------------------------------- manifest

#[test]
fn manifest_needs_a_repo_and_at_least_one_mapping() {
    let no_repo = sync::parse_manifest("mappings:\n  - docs/**\n");
    assert_eq!(
        no_repo.unwrap_err().to_string(),
        "sync.yaml needs `upstream.repo`, e.g. `finos/morphir`"
    );
    let no_mappings = sync::parse_manifest("upstream:\n  repo: a/b\n");
    assert_eq!(
        no_mappings.unwrap_err().to_string(),
        "sync.yaml needs at least one entry under `mappings:`"
    );
}

#[test]
fn manifest_refuses_a_root_that_leaves_the_bundle() {
    // `root` is resolved segment by segment onto the bundle directory, so a `..`
    // in it wrote the mirror outside the bundle — and `pull --prune` then deleted
    // files out there. Caught at parse time, so every command refuses it.
    let escaping =
        sync::parse_manifest("upstream:\n  repo: a/b\nroot: ../shared\nmappings:\n  - docs/**\n");
    assert_eq!(
        escaping.unwrap_err().to_string(),
        "sync.yaml `root: ../shared` leaves the bundle — a root is a plain directory inside it, e.g. `sources`, with no `.` or `..` segments"
    );
    let far = sync::parse_manifest(
        "upstream:\n  repo: a/b\nroot: ../../../victim\nmappings:\n  - docs/**\n",
    );
    assert_eq!(
        far.unwrap_err().to_string(),
        "sync.yaml `root: ../../../victim` leaves the bundle — a root is a plain directory inside it, e.g. `sources`, with no `.` or `..` segments"
    );
    let absolute = sync::parse_manifest(
        "upstream:\n  repo: a/b\nroot: /etc/morphir\nmappings:\n  - docs/**\n",
    );
    assert_eq!(
        absolute.unwrap_err().to_string(),
        "sync.yaml `root: /etc/morphir` must be relative to the bundle, e.g. `sources` — an absolute path is refused rather than silently reread as a bundle subdirectory"
    );
}

#[test]
fn manifest_refuses_a_root_that_leaves_the_bundle_through_a_windows_separator() {
    // The guard used to split on `/` alone, so `..\victim` held no separator it
    // could see, passed validation, and was handed to `PathBuf::push` — which on
    // Windows, where the release workflow ships two targets, reads it as two
    // segments and climbs out of the bundle. Same story for a drive designator
    // and for a bare leading `\`.
    //
    // Validation is uniformly strict rather than platform-dependent: sync.yaml is
    // committed and read on every platform, so a root that is only safe on Linux
    // is still a bad root, and the manifest must be refused wherever it is read.
    for root in ["..\\victim", "..\\..\\outside", "a/..\\b"] {
        let err = sync::parse_manifest(&format!(
            "upstream:\n  repo: a/b\nroot: '{root}'\nmappings:\n  - docs/**\n"
        ))
        .unwrap_err()
        .to_string();
        assert_eq!(
            err,
            format!(
                "sync.yaml `root: {root}` leaves the bundle \
                 — a root is a plain directory inside it, e.g. `sources`, with no `.` or `..` segments"
            )
        );
    }
    for root in ["C:\\victim", "\\victim", "C:victim"] {
        let err = sync::parse_manifest(&format!(
            "upstream:\n  repo: a/b\nroot: '{root}'\nmappings:\n  - docs/**\n"
        ))
        .unwrap_err()
        .to_string();
        assert_eq!(
            err,
            format!(
                "sync.yaml `root: {root}` must be relative to the bundle, e.g. `sources` \
                 — an absolute path is refused rather than silently reread as a bundle subdirectory"
            )
        );
    }
}

#[test]
fn safe_relative_accepts_a_backslash_that_stays_contained_on_unix_and_on_windows() {
    // The policy is containment, not separator purity. `a\b.md` is one ordinary
    // filename on Unix and the file `b.md` inside the directory `a` on Windows —
    // inside the mirror either way — so refusing it would break a legitimate
    // upstream file for no gain. What is refused is what *escapes* under some
    // platform's reading: `..\`, `\` at the front, and a drive designator.
    assert!(sync::safe_relative("docs/a\\b.md"));
    let m = parse_manifest("upstream:\n  repo: a/b\nroot: 'a\\b'\nmappings:\n  - docs/**\n");
    assert_eq!(m.root, "a\\b");
}

#[test]
fn safe_relative_refuses_windows_separators_drive_letters_and_bare_roots() {
    assert!(!sync::safe_relative("..\\victim"));
    assert!(!sync::safe_relative("..\\..\\outside"));
    assert!(!sync::safe_relative("a/..\\b"));
    assert!(!sync::safe_relative("C:\\victim"));
    assert!(
        !sync::safe_relative("C:victim"),
        "drive-relative escapes too"
    );
    assert!(!sync::safe_relative("\\victim"));
    assert!(!sync::safe_relative(".\\here"));
    assert!(sync::safe_relative("docs/spec/types.md"));
}

#[test]
fn manifest_keeps_a_nested_root_and_defaults_an_absent_or_empty_one() {
    let nested =
        parse_manifest("upstream:\n  repo: a/b\nroot: vendor/sources\nmappings:\n  - docs/**\n");
    assert_eq!(nested.root, "vendor/sources");
    let absent = parse_manifest("upstream:\n  repo: a/b\nmappings:\n  - docs/**\n");
    assert_eq!(absent.root, "sources");
    let empty = parse_manifest("upstream:\n  repo: a/b\nroot: \"\"\nmappings:\n  - docs/**\n");
    assert_eq!(
        empty.root, "sources",
        "an empty root falls back like an absent one"
    );
}

#[test]
fn manifest_resolves_type_by_first_matching_glob_in_declaration_order() {
    let m = parse_manifest(
        "upstream:\n  repo: a/b\nmappings:\n  - docs/**\ntype_map:\n  \"docs/design/**\": Design Source\n  \"docs/**\": Specification Source\n",
    );
    assert_eq!(m.type_for("docs/design/ir/values.md"), "Design Source");
    assert_eq!(m.type_for("docs/spec/types.md"), "Specification Source");
    assert_eq!(
        m.type_for("wit/x.wit"),
        "Source Document",
        "unmatched paths fall back"
    );
}

#[test]
fn manifest_selects_by_mapping_minus_exclusions() {
    let m = parse_manifest(
        "upstream:\n  repo: a/b\nmappings:\n  - { from: \"website/**\", exclude: [\"**/*.json\"] }\nexclude:\n  - \"**/big.yaml\"\n",
    );
    assert!(m.selects("website/static/schemas/x.yaml"));
    assert!(
        !m.selects("website/static/schemas/x.json"),
        "mapping-level exclude"
    );
    assert!(
        !m.selects("website/static/schemas/big.yaml"),
        "manifest-level exclude"
    );
}

#[test]
fn manifest_refuses_a_type_map_entry_naming_a_type_a_register_owns() {
    // The live instance: `docs/adr/**` mapped to `Decision Record`, which is
    // exactly what the decision register discovers by.
    let bad = sync::parse_manifest(
        "upstream:\n  repo: a/b\nmappings:\n  - docs/**\ntype_map:\n  \"docs/adr/**\": Decision Record\n",
    );
    let msg = bad.unwrap_err().to_string();
    assert!(msg.contains("docs/adr/**"), "must say which entry: {msg}");
    assert!(
        msg.contains("Register-owned: Decision Record"),
        "got: {msg}"
    );
    assert!(
        sync::parse_manifest(
            "upstream:\n  repo: a/b\nmappings:\n  - docs/**\ntype_map:\n  \"docs/adr/**\": Decision Source\n"
        )
        .is_ok(),
        "naming what the file *is* stays allowed"
    );
    // Case and surrounding space cannot be used to smuggle one past: the register
    // matches case-insensitively on the trimmed value.
    assert!(
        sync::parse_manifest(
            "upstream:\n  repo: a/b\nmappings:\n  - docs/**\ntype_map:\n  \"docs/adr/**\": \" decision record \"\n"
        )
        .is_err(),
        "the comparison is the register's own, not a literal one"
    );
}

#[test]
fn manifest_classifies_markdown_as_concept_and_everything_else_as_asset() {
    let m = parse_manifest("upstream:\n  repo: a/b\nmappings:\n  - \"**\"\n");
    assert_eq!(m.kind_of("docs/x.md"), SyncKind::Concept);
    // .mdx carries JSX, which commonmark has no business parsing.
    assert_eq!(m.kind_of("docs/x.mdx"), SyncKind::Asset);
    assert_eq!(m.kind_of("schemas/x.yaml"), SyncKind::Asset);
}

// ----------------------------------------------------------------- lockfile

#[test]
fn lock_unquoted_date_parses_back_as_a_date_not_as_nothing() {
    // SnakeYAML resolved `2026-07-28` to a Date and the Scala accessor dropped it;
    // serde_yaml keeps it a string, and this pins that the round trip holds.
    let lock = SyncLock {
        base_commit: "abc123".to_string(),
        imported_at: "2026-07-28".to_string(),
        files: vec![LockEntry {
            path: "a.md".to_string(),
            kind: SyncKind::Concept,
            upstream_sha256: "h".to_string(),
        }],
    };
    let back = sync::parse_lock(&sync::render_lock(&lock)).unwrap();
    assert_eq!(back.imported_at, "2026-07-28");
}

#[test]
fn lock_leaves_the_date_alone_when_a_pull_imports_nothing() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let (_, sb0) = load_sync(&kb_root);
    let first = sync::pull(&sb0, &upstream, "deadbeef", today(), false, false, false).unwrap();
    sync::write_lock(&sb0, &first.lock).unwrap();
    let lock_path = sb0.bundle.root.join(sync::LOCK_NAME);
    let bytes_before = fs::read(&lock_path).unwrap();
    let (_, sb) = load_sync(&kb_root);
    let later = today() + chrono::Days::new(30);
    let again = sync::pull(&sb, &upstream, "deadbeef", later, false, false, false).unwrap();
    let written = sync::write_lock(&sb, &again.lock).unwrap();
    assert!(
        again.actions.is_empty(),
        "the second pull imports nothing: {:?}",
        again.actions
    );
    assert_eq!(
        written.imported_at,
        today().to_string(),
        "so the date holds"
    );
    // Byte stability: a no-op pull leaves no diff in the lockfile.
    assert_eq!(fs::read(&lock_path).unwrap(), bytes_before);
}

#[test]
fn lock_round_trips_through_render_and_parse() {
    let lock = SyncLock {
        base_commit: "abc123".to_string(),
        imported_at: "2026-07-28".to_string(),
        files: vec![
            LockEntry {
                path: "docs/b.md".to_string(),
                kind: SyncKind::Concept,
                upstream_sha256: "hash-b".to_string(),
            },
            LockEntry {
                path: "wit/a.wit".to_string(),
                kind: SyncKind::Asset,
                upstream_sha256: "hash-a".to_string(),
            },
        ],
    };
    let rendered = sync::render_lock(&lock);
    let back = sync::parse_lock(&rendered).unwrap();
    assert_eq!(back.base_commit, "abc123");
    assert_eq!(back.get("wit/a.wit").map(|e| e.kind), Some(SyncKind::Asset));
    // Sorted on write, so a pull that changes nothing produces no diff.
    assert!(
        rendered.find("docs/b.md").unwrap() < rendered.find("wit/a.wit").unwrap(),
        "entries are written in path order"
    );
}

#[test]
fn lock_leaves_an_ordinary_path_unquoted() {
    // The lockfile is committed, so the rendering for ordinary paths must stay
    // byte-for-byte what it was before quoting arrived — and what the Scala
    // `renderLock` still writes.
    let lock = SyncLock {
        base_commit: "abc123".to_string(),
        imported_at: "2026-07-28".to_string(),
        files: vec![LockEntry {
            path: "docs/spec/types.md".to_string(),
            kind: SyncKind::Concept,
            upstream_sha256: "deadbeef".to_string(),
        }],
    };
    assert_eq!(
        sync::render_lock(&lock),
        "# Generated by `kb sync pull`. Do not edit by hand.\nbase_commit: abc123\nimported_at: 2026-07-28\nfiles:\n  - { path: docs/spec/types.md, kind: concept, upstream_sha256: deadbeef }\n"
    );
}

#[test]
fn lock_round_trips_paths_holding_flow_mapping_punctuation() {
    // A comma used to cut `docs/a,b.md` down to `docs/a`; `:`, `{` and `}` made the
    // lockfile fail to parse outright. All of them are legal filename bytes.
    let hostile = [
        "docs/a,b.md",
        "docs/a:b.md",
        "docs/{a}.md",
        "docs/a#b.md",
        "docs/a b.md",
        "docs/plain.md",
    ];
    let lock = SyncLock {
        base_commit: "abc123".to_string(),
        imported_at: "2026-07-28".to_string(),
        files: hostile
            .iter()
            .map(|p| LockEntry {
                path: (*p).to_string(),
                kind: SyncKind::Concept,
                upstream_sha256: "hash".to_string(),
            })
            .collect(),
    };
    let rendered = sync::render_lock(&lock);
    let back = sync::parse_lock(&rendered).unwrap();
    assert_eq!(back.files.len(), hostile.len(), "rendered as: {rendered}");
    for p in hostile {
        assert_eq!(
            back.get(p).map(|e| e.upstream_sha256.clone()),
            Some("hash".to_string()),
            "{p} did not survive the round trip: {rendered}"
        );
    }
}

// --------------------------------------------------------------------- pull

#[test]
fn pull_imports_concepts_and_assets_and_records_both_in_the_lock() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let (_, sb) = load_sync(&kb_root);
    let result = sync::pull(&sb, &upstream, "deadbeef", today(), false, false, false).unwrap();
    sync::write_lock(&sb, &result.lock).unwrap();
    let types_text = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    let schema_text = fs::read_to_string(sb.local_file("schemas/thing.yaml")).unwrap();
    assert_eq!(
        result.actions.iter().filter(|a| a.verb == "added").count(),
        3,
        "got {:?}",
        result.actions
    );
    assert!(
        types_text.contains("type: Specification Source"),
        "concepts gain a kb block"
    );
    assert!(
        types_text.contains("title: Types"),
        "upstream frontmatter is preserved"
    );
    assert_eq!(
        schema_text, "$id: thing\ntype: object\n",
        "assets are byte-identical"
    );
    assert_eq!(
        result
            .lock
            .files
            .iter()
            .filter(|e| e.kind == SyncKind::Asset)
            .count(),
        1
    );
}

#[test]
fn pull_writes_nothing_under_dry_run() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let (_, sb) = load_sync(&kb_root);
    let result = sync::pull(&sb, &upstream, "deadbeef", today(), true, false, false).unwrap();
    assert!(
        !result.actions.is_empty(),
        "it still reports what it would do"
    );
    assert!(
        !sb.local_file("docs/types.md").exists(),
        "dry run must not write"
    );
}

#[test]
fn a_mirrored_index_md_is_a_concept_not_a_sub_index() {
    // `kind_of` reserves index.md for the bundle. Inside a mirror the name is
    // upstream's, and upstream puts frontmatter in it — which used to be reported
    // as `subindex-has-frontmatter`.
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    seeded(&kb_root, &upstream);
    let kb = store::load(&kb_root).unwrap();
    let b = kb.bundle("vendored").unwrap();
    let mirrored = b
        .concepts
        .iter()
        .find(|d| d.rel.join("/") == "sources/docs/index.md")
        .expect("mirrored index.md is discovered as a concept");
    assert_eq!(mirrored.kind, DocKind::Concept);
    assert!(mirrored.vendored, "and it is marked as vendored");
}

#[test]
fn safe_relative_refuses_a_manifest_path_that_escapes_the_mirror() {
    // The same hole `add-concept` had: a relative path with `..` writes outside
    // the bundle it claims to be in.
    assert!(!sync::safe_relative("../escaped.md"));
    assert!(!sync::safe_relative("/etc/passwd"));
    assert!(sync::safe_relative("docs/spec/types.md"));
}

#[cfg(unix)]
#[test]
fn pull_refuses_a_mirror_root_that_symlinks_out_of_the_bundle() {
    // Every containment check up to here is lexical, and `sources` is as plain a
    // name as there is — so a mirror root that is a *symlink* to somewhere else
    // passed all of them, and then every read, write and delete followed the
    // link. `pull --prune` deleting a file it never owned is the worst of those.
    let (dir, kb_root, bundle_root, upstream) = sync_fixture();
    let victim = dir.path().join("victim");
    write_file(&victim.join("docs/gone.md"), "# Victim\n");
    // A lock entry for a file that upstream does not have, so a successful prune
    // would delete `victim/docs/gone.md` outright.
    write_file(
        &bundle_root.join(sync::LOCK_NAME),
        &format!(
            "base_commit: deadbeef\nimported_at: 2026-07-28\nfiles:\n  - {{ path: docs/gone.md, kind: concept, upstream_sha256: {} }}\n",
            sync::sha256(b"# Victim\n")
        ),
    );
    std::os::unix::fs::symlink(&victim, bundle_root.join("sources")).unwrap();
    let (_, sb) = load_sync(&kb_root);
    for prune in [false, true] {
        let err = sync::pull(&sb, &upstream, "deadbeef", today(), false, false, prune)
            .unwrap_err()
            .to_string();
        assert!(err.contains("resolves outside the bundle"), "got: {err}");
    }
    assert_eq!(
        fs::read_to_string(victim.join("docs/gone.md")).unwrap(),
        "# Victim\n",
        "nothing outside the bundle may be deleted or rewritten through the link"
    );
    assert!(
        !victim.join("docs/types.md").exists(),
        "nor may anything be imported through it"
    );
}

#[test]
fn relative_links_in_mirrored_documents_resolve_to_paths_that_exist() {
    // Regression from the reference implementation: relative links used to resolve
    // against a rebuilt segment list and every one of them missed.
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    write_file(
        &upstream.join("docs").join("guide.md"),
        "---\ntitle: Guide\ndescription: A guide.\n---\n\nSee [types](types.md) and [gone](missing.md).\n",
    );
    seeded(&kb_root, &upstream);
    let kb = store::load(&kb_root).unwrap();
    let b = kb.bundle("vendored").unwrap();
    let guide = b.concepts.iter().find(|d| d.name() == "guide.md").unwrap();
    let resolved: Vec<PathBuf> = guide
        .links
        .iter()
        .filter_map(|l| store::resolve_link(guide, l))
        .collect();
    assert_eq!(resolved.len(), 2, "got {resolved:?}");
    // Compare with Path::ends_with, which matches whole components. A string
    // suffix would hardcode "/" and never match on Windows, where the resolved
    // path is spelled with backslashes.
    assert!(
        resolved
            .iter()
            .any(|p| p.exists() && p.ends_with("sources/docs/types.md")),
        "got {resolved:?}"
    );
    assert!(
        resolved
            .iter()
            .any(|p| !p.exists() && p.ends_with("missing.md")),
        "and a real miss still misses"
    );
}

// ------------------------------------- the injected block follows the manifest

#[test]
fn a_type_map_edit_reaches_a_file_that_is_already_clean() {
    // The bug behind morphir-scala#947: the injected block is invisible to status,
    // so pull passed over every clean file — a manifest edit was write-once, and a
    // wrong `type` sat there forever.
    let (_dir, kb_root, bundle_root, upstream) = sync_fixture();
    seeded(&kb_root, &upstream);
    retype(&bundle_root, "Design Source");
    let (_, sb) = load_sync(&kb_root);
    let rows = sync::status(&sb, Some(&upstream)).unwrap();
    let stale: Vec<&str> = rows
        .iter()
        .filter(|r| r.injection_stale)
        .map(|r| r.path.as_str())
        .collect();
    assert_eq!(
        stale,
        vec!["docs/index.md", "docs/types.md"],
        "both concepts are stale, the asset is not"
    );
    assert!(
        rows.iter().all(|r| r.state == SyncState::Clean),
        "and staleness is orthogonal to state: {rows:?}"
    );
    let again = sync::pull(&sb, &upstream, "deadbeef", today(), false, false, false).unwrap();
    let after = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    assert_eq!(
        again
            .actions
            .iter()
            .filter(|a| a.verb == "re-injected")
            .count(),
        2,
        "got {:?}",
        again.actions
    );
    assert!(after.contains("type: Design Source"), "got: {after}");
    assert!(
        after.contains("title: Types"),
        "upstream's frontmatter is untouched"
    );
    let (_, sb2) = load_sync(&kb_root);
    let settled = sync::status(&sb2, Some(&upstream)).unwrap();
    assert!(
        settled.iter().all(|r| !r.injection_stale),
        "and a second pass has nothing left to do"
    );
}

#[test]
fn pull_keeps_a_key_added_inside_the_fence_by_hand() {
    let (_dir, kb_root, bundle_root, upstream) = sync_fixture();
    let sb0 = seeded(&kb_root, &upstream);
    let before = fs::read_to_string(sb0.local_file("docs/types.md")).unwrap();
    fs::write(
        sb0.local_file("docs/types.md"),
        before.replace("# kb:end", "reviewed_by: us\n# kb:end"),
    )
    .unwrap();
    retype(&bundle_root, "Design Source");
    let (_, sb) = load_sync(&kb_root);
    sync::pull(&sb, &upstream, "deadbeef", today(), false, false, false).unwrap();
    let after = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    assert!(
        after.contains("reviewed_by: us"),
        "the hand-added key survives: {after}"
    );
    assert!(
        after.contains("type: Design Source"),
        "and the generated one is corrected"
    );
    let rows = sync::status(&sb, Some(&upstream)).unwrap();
    assert_eq!(
        state_of(&rows, "docs/types.md").state,
        SyncState::Clean,
        "re-injection does not disturb the upstream form"
    );
}

#[test]
fn pull_reports_the_rewrite_under_dry_run_without_performing_it() {
    let (_dir, kb_root, bundle_root, upstream) = sync_fixture();
    seeded(&kb_root, &upstream);
    retype(&bundle_root, "Design Source");
    let (_, sb) = load_sync(&kb_root);
    let dry = sync::pull(&sb, &upstream, "deadbeef", today(), true, false, false).unwrap();
    let after = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    assert_eq!(
        dry.actions
            .iter()
            .filter(|a| a.verb == "re-injected")
            .count(),
        2,
        "got {:?}",
        dry.actions
    );
    // Its own verb, so a bulk re-injection across a mirror does not read as an
    // import from upstream.
    assert!(
        dry.actions
            .iter()
            .all(|a| a.verb != "added" && a.verb != "updated"),
        "got {:?}",
        dry.actions
    );
    assert!(
        after.contains("type: Specification Source"),
        "dry run must not write"
    );
}

#[test]
fn check_reports_a_stale_block_without_a_reference_checkout() {
    let (_dir, kb_root, bundle_root, upstream) = sync_fixture();
    seeded(&kb_root, &upstream);
    retype(&bundle_root, "Design Source");
    let (kb, sb) = load_sync(&kb_root);
    // Staleness is decided from the local file alone: no upstream checkout here.
    let rows = sync::status(&sb, None).unwrap();
    assert_eq!(rows.iter().filter(|r| r.injection_stale).count(), 2);
    let findings = sync::check_findings(&kb, &sb, None).unwrap();
    assert_eq!(
        findings
            .iter()
            .filter(|f| f.check == "sync-injection-stale" && f.severity == Severity::Warn)
            .count(),
        2,
        "got {:?}",
        findings
            .iter()
            .map(|f| (&f.check, &f.path))
            .collect::<Vec<_>>()
    );
}

// -------------------------------------------------------- review regressions

#[test]
fn a_binary_asset_survives_status_and_push_as_bytes() {
    // Assets were decoded as UTF-8 to hash and to export. Any invalid sequence
    // became U+FFFD, so a freshly pulled binary looked locally modified — and push
    // then wrote the replacement characters over upstream's bytes.
    let binary: [u8; 7] = [0xff, 0xfe, 0x00, 0x01, 0x80, b'o', b'k'];
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    write_bin(&upstream.join("schemas").join("blob.bin"), &binary);
    let sb = seeded(&kb_root, &upstream);
    let mirrored = fs::read(sb.local_file("schemas/blob.bin")).unwrap();
    let rows = sync::status(&sb, Some(&upstream)).unwrap();
    assert_eq!(
        mirrored, binary,
        "the mirror holds upstream's bytes verbatim"
    );
    assert_eq!(state_of(&rows, "schemas/blob.bin").state, SyncState::Clean);
    let target = TempDir::new().unwrap();
    let before = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    fs::write(sb.local_file("docs/types.md"), before + "\nedit\n").unwrap();
    let (_, sb2) = load_sync(&kb_root);
    sync::push(&sb2, target.path(), Some(&upstream), false, false).unwrap();
    assert!(
        !target.path().join("schemas").join("blob.bin").exists(),
        "an unchanged asset is not rewritten"
    );
}

#[test]
fn a_file_deleted_upstream_but_edited_here_is_never_pruned() {
    // `deleted-upstream` was assigned before the local hash was consulted, so
    // `pull --prune` deleted a file carrying edits that were waiting to be
    // exported — the one operation here that destroys unrecoverable work.
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb0 = seeded(&kb_root, &upstream);
    let before = fs::read_to_string(sb0.local_file("docs/types.md")).unwrap();
    fs::write(
        sb0.local_file("docs/types.md"),
        before + "\nWork we mean to send.\n",
    )
    .unwrap();
    fs::remove_file(upstream.join("docs").join("types.md")).unwrap();
    let (_, sb) = load_sync(&kb_root);
    let rows = sync::status(&sb, Some(&upstream)).unwrap();
    assert_eq!(
        state_of(&rows, "docs/types.md").state,
        SyncState::DeletedUpstreamEdited
    );
    let pruned = sync::pull(&sb, &upstream, "deadbeef", today(), false, true, true).unwrap();
    assert!(
        sb.local_file("docs/types.md").exists(),
        "--prune --theirs together must still not delete it"
    );
    let content = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    assert!(
        content.contains("Work we mean to send."),
        "and the edit is intact"
    );
    assert_eq!(
        pruned
            .refused
            .iter()
            .map(|r| r.path.as_str())
            .collect::<Vec<_>>(),
        vec!["docs/types.md"]
    );
}

#[test]
fn pull_populates_refused_which_is_the_contract_the_exit_code_rests_on() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb0 = seeded(&kb_root, &upstream);
    write_file(
        &upstream.join("docs").join("types.md"),
        "---\ntitle: Types\ndescription: Theirs.\n---\n\n# T\n",
    );
    let before = fs::read_to_string(sb0.local_file("docs/types.md")).unwrap();
    fs::write(sb0.local_file("docs/types.md"), before + "\nOurs.\n").unwrap();
    let (_, sb) = load_sync(&kb_root);
    let again = sync::pull(&sb, &upstream, "deadbeef", today(), false, false, false).unwrap();
    assert_eq!(
        again
            .refused
            .iter()
            .map(|r| r.path.as_str())
            .collect::<Vec<_>>(),
        vec!["docs/types.md"]
    );
    assert_eq!(again.refused[0].state, SyncState::Diverged);
}

// ------------------------------------------------------------------- status

#[test]
fn status_is_clean_straight_after_a_pull_and_local_only_after_an_edit() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb = seeded(&kb_root, &upstream);
    let clean = sync::status(&sb, Some(&upstream)).unwrap();
    assert!(
        clean.iter().all(|r| r.state == SyncState::Clean),
        "got {clean:?}"
    );
    let before = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    fs::write(
        sb.local_file("docs/types.md"),
        before + "\nAn extra line.\n",
    )
    .unwrap();
    let (_, sb2) = load_sync(&kb_root);
    let edited = sync::status(&sb2, Some(&upstream)).unwrap();
    assert_eq!(
        edited
            .iter()
            .filter(|r| r.state == SyncState::LocalOnly)
            .count(),
        1,
        "got {edited:?}"
    );
}

#[test]
fn status_reports_upstream_only_when_upstream_moves_and_we_did_not() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb = seeded(&kb_root, &upstream);
    write_file(
        &upstream.join("docs").join("types.md"),
        "---\ntitle: Types\ndescription: Changed.\n---\n\n# Types\n",
    );
    let rows = sync::status(&sb, Some(&upstream)).unwrap();
    assert_eq!(
        state_of(&rows, "docs/types.md").state,
        SyncState::UpstreamOnly
    );
}

#[test]
fn status_reports_diverged_when_both_sides_moved() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb0 = seeded(&kb_root, &upstream);
    write_file(
        &upstream.join("docs").join("types.md"),
        "---\ntitle: Types\ndescription: Theirs.\n---\n\n# Types\n",
    );
    let before = fs::read_to_string(sb0.local_file("docs/types.md")).unwrap();
    fs::write(sb0.local_file("docs/types.md"), before + "\nOurs.\n").unwrap();
    let (_, sb) = load_sync(&kb_root);
    let rows = sync::status(&sb, Some(&upstream)).unwrap();
    assert_eq!(state_of(&rows, "docs/types.md").state, SyncState::Diverged);
    assert_eq!(sync::strict_violations(&rows), 1);
}

#[test]
fn status_missing_local_and_pull_reimports() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb0 = seeded(&kb_root, &upstream);
    fs::remove_file(sb0.local_file("docs/types.md")).unwrap();
    let (_, sb) = load_sync(&kb_root);
    let rows = sync::status(&sb, Some(&upstream)).unwrap();
    assert_eq!(
        state_of(&rows, "docs/types.md").state,
        SyncState::MissingLocal
    );
    let result = sync::pull(&sb, &upstream, "deadbeef", today(), false, false, false).unwrap();
    assert!(
        result
            .actions
            .iter()
            .any(|a| a.verb == "updated" && a.path == "docs/types.md"),
        "a tracked file is an update, not an add: {:?}",
        result.actions
    );
    assert!(
        sb.local_file("docs/types.md").exists(),
        "and it is restored"
    );
}

#[test]
fn status_deleted_upstream_and_pull_prunes_only_when_asked() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let _sb0 = seeded(&kb_root, &upstream);
    fs::remove_file(upstream.join("schemas").join("thing.yaml")).unwrap();
    let (_, sb) = load_sync(&kb_root);
    let rows = sync::status(&sb, Some(&upstream)).unwrap();
    assert_eq!(
        state_of(&rows, "schemas/thing.yaml").state,
        SyncState::DeletedUpstream
    );
    let kept = sync::pull(&sb, &upstream, "deadbeef", today(), false, false, false).unwrap();
    assert!(
        kept.actions
            .iter()
            .any(|a| a.verb == "gone upstream" && a.path == "schemas/thing.yaml"),
        "got {:?}",
        kept.actions
    );
    assert!(
        kept.lock.get("schemas/thing.yaml").is_some(),
        "the lock entry is kept"
    );
    assert!(sb.local_file("schemas/thing.yaml").exists());
    let pruned = sync::pull(&sb, &upstream, "deadbeef", today(), false, false, true).unwrap();
    assert!(
        pruned
            .actions
            .iter()
            .any(|a| a.verb == "removed" && a.path == "schemas/thing.yaml"),
        "got {:?}",
        pruned.actions
    );
    assert!(
        pruned.lock.get("schemas/thing.yaml").is_none(),
        "--prune drops the entry"
    );
    assert!(
        !sb.local_file("schemas/thing.yaml").exists(),
        "and deletes the file"
    );
}

#[test]
fn status_untracked_and_pull_imports_a_new_upstream_file() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    seeded(&kb_root, &upstream);
    write_file(
        &upstream.join("docs").join("fresh.md"),
        "# Fresh\n\nNew upstream.\n",
    );
    let (_, sb) = load_sync(&kb_root);
    let rows = sync::status(&sb, Some(&upstream)).unwrap();
    assert_eq!(state_of(&rows, "docs/fresh.md").state, SyncState::Untracked);
    let result = sync::pull(&sb, &upstream, "deadbeef", today(), false, false, false).unwrap();
    assert!(
        result
            .actions
            .iter()
            .any(|a| a.verb == "added" && a.path == "docs/fresh.md"),
        "got {:?}",
        result.actions
    );
    // No upstream frontmatter, so the whole block is ours, flagged as such.
    let text = fs::read_to_string(sb.local_file("docs/fresh.md")).unwrap();
    assert!(text.contains("# kb:begin block"), "got: {text}");
    assert_eq!(
        sync::project(&text),
        Ok("# Fresh\n\nNew upstream.\n".to_string())
    );
}

#[test]
fn pull_tracks_an_upstream_path_that_holds_a_comma() {
    // End to end for the truncated lock entry: the phantom `docs/a` read back as
    // deleted-upstream, which `--prune` would have acted on, while the real file
    // stayed untracked and was re-imported by every pull.
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    write_file(
        &upstream.join("docs").join("a,b.md"),
        "---\ntitle: Comma\n---\n\n# Comma\n",
    );
    seeded(&kb_root, &upstream);
    let (_, sb) = load_sync(&kb_root);
    assert!(
        sb.lock.get("docs/a,b.md").is_some(),
        "the lock tracks the whole path: {:?}",
        sb.lock.files
    );
    let rows = sync::status(&sb, Some(&upstream)).unwrap();
    assert_eq!(state_of(&rows, "docs/a,b.md").state, SyncState::Clean);
    assert!(
        !rows.iter().any(|r| r.path == "docs/a"),
        "no phantom truncated row: {rows:?}"
    );
    let again = sync::pull(&sb, &upstream, "deadbeef", today(), false, false, true).unwrap();
    assert!(
        again.actions.is_empty(),
        "a second pull, pruning, has nothing to do: {:?}",
        again.actions
    );
    assert!(
        sb.local_file("docs/a,b.md").exists(),
        "and the file survives"
    );
}

#[test]
fn a_damaged_fence_is_unreadable_and_pull_refuses_unless_theirs() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb0 = seeded(&kb_root, &upstream);
    let before = fs::read_to_string(sb0.local_file("docs/types.md")).unwrap();
    fs::write(
        sb0.local_file("docs/types.md"),
        before.replace("# kb:end\n", ""),
    )
    .unwrap();
    let (_, sb) = load_sync(&kb_root);
    let rows = sync::status(&sb, Some(&upstream)).unwrap();
    let row = state_of(&rows, "docs/types.md");
    assert_eq!(row.state, SyncState::Unreadable);
    assert_eq!(row.detail, "# kb:begin without # kb:end");
    assert!(!row.injection_stale, "damaged is unreadable, not stale");
    assert_eq!(sync::strict_violations(&rows), 1);
    let refused = sync::pull(&sb, &upstream, "deadbeef", today(), false, false, false).unwrap();
    assert_eq!(
        refused
            .refused
            .iter()
            .map(|r| r.path.as_str())
            .collect::<Vec<_>>(),
        vec!["docs/types.md"]
    );
    let taken = sync::pull(&sb, &upstream, "deadbeef", today(), false, true, false).unwrap();
    assert!(
        taken
            .actions
            .iter()
            .any(|a| a.verb == "updated" && a.path == "docs/types.md"),
        "--theirs takes upstream's copy: {:?}",
        taken.actions
    );
    let restored = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    assert!(
        sync::project(&restored).is_ok(),
        "and the fence is whole again"
    );
}

#[test]
fn upstream_hash_equal_to_local_hash_is_clean_and_pull_rebaselines() {
    // What an export leaves behind: both sides differ from the recorded baseline
    // while being identical to each other — nothing to send, nothing to take.
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb0 = seeded(&kb_root, &upstream);
    let new_upstream = "---\ntitle: Types\ndescription: Exported.\n---\n\n# Types\n";
    write_file(&upstream.join("docs").join("types.md"), new_upstream);
    let injected = sync::inject(
        new_upstream,
        &sync::injected_keys(&sb0.manifest, "docs/types.md", new_upstream),
    );
    fs::write(sb0.local_file("docs/types.md"), &injected).unwrap();
    let (_, sb) = load_sync(&kb_root);
    let rows = sync::status(&sb, Some(&upstream)).unwrap();
    assert_eq!(
        state_of(&rows, "docs/types.md").state,
        SyncState::Clean,
        "agreement beats the baseline"
    );
    let result = sync::pull(&sb, &upstream, "deadbeef", today(), false, false, false).unwrap();
    assert!(
        result
            .actions
            .iter()
            .any(|a| a.verb == "rebaselined" && a.path == "docs/types.md"),
        "got {:?}",
        result.actions
    );
    assert_eq!(
        result
            .lock
            .get("docs/types.md")
            .map(|e| e.upstream_sha256.clone()),
        Some(sync::sha256(new_upstream.as_bytes())),
        "the lock catches up to the shared content"
    );
}

// --------------------------------------------------------------------- push

#[test]
fn push_writes_back_only_what_changed_here_and_in_upstream_form() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb0 = seeded(&kb_root, &upstream);
    let before = fs::read_to_string(sb0.local_file("docs/types.md")).unwrap();
    fs::write(
        sb0.local_file("docs/types.md"),
        before + "\nA sentence we added.\n",
    )
    .unwrap();
    let (_, sb) = load_sync(&kb_root);
    let target = TempDir::new().unwrap();
    let out = sync::push(&sb, target.path(), Some(&upstream), false, false).unwrap();
    assert!(out.refused.is_empty(), "got {:?}", out.refused);
    assert_eq!(
        out.actions
            .iter()
            .map(|a| a.path.as_str())
            .collect::<Vec<_>>(),
        vec!["docs/types.md"]
    );
    let exported = fs::read_to_string(target.path().join("docs").join("types.md")).unwrap();
    assert!(
        !exported.contains("kb:begin"),
        "the kb block must never reach upstream"
    );
    assert!(!exported.contains("kb_upstream"), "nor its keys");
    assert_eq!(
        exported,
        "---\ntitle: Types\ndescription: The types.\n---\n\n# Types\n\nA sentence we added.\n"
    );
    assert!(
        !target.path().join("schemas").join("thing.yaml").exists(),
        "unchanged files are not written"
    );
}

#[test]
fn push_holds_back_a_diverged_file_rather_than_overwriting_upstream() {
    // push used to compare against the lockfile alone. A file that had moved on
    // both sides then looked merely local-only, and exporting it silently
    // discarded upstream's edit.
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb0 = seeded(&kb_root, &upstream);
    write_file(
        &upstream.join("docs").join("types.md"),
        "---\ntitle: Types\ndescription: Theirs.\n---\n\n# Types\n",
    );
    let before = fs::read_to_string(sb0.local_file("docs/types.md")).unwrap();
    fs::write(sb0.local_file("docs/types.md"), before + "\nOurs.\n").unwrap();
    let (_, sb) = load_sync(&kb_root);
    let target = TempDir::new().unwrap();
    let held = sync::push(&sb, target.path(), Some(&upstream), false, false).unwrap();
    assert!(
        !target.path().join("docs").join("types.md").exists(),
        "a diverged file must not be exported by default"
    );
    assert!(
        held.actions.iter().any(|a| a.verb == "held back"),
        "and it must be reported: {:?}",
        held.actions
    );
    assert!(
        !held.refused.is_empty(),
        "it counts as refused, so the command exits non-zero"
    );
    let forced = sync::push(&sb, target.path(), Some(&upstream), false, true).unwrap();
    assert!(
        target.path().join("docs").join("types.md").exists(),
        "--include-diverged is the explicit override"
    );
    assert!(
        forced.actions.iter().any(|a| a.verb == "wrote"),
        "got {:?}",
        forced.actions
    );
}

// ------------------------------------------------------------ check findings

#[test]
fn check_flags_a_mirrored_file_the_lockfile_expects_but_the_mirror_lacks() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb0 = seeded(&kb_root, &upstream);
    fs::remove_file(sb0.local_file("docs/types.md")).unwrap();
    let (kb, sb) = load_sync(&kb_root);
    let findings = sync::check_findings(&kb, &sb, None).unwrap();
    assert!(
        findings
            .iter()
            .any(|f| f.check == "sync-lock-drift" && f.severity == Severity::Error),
        "got {:?}",
        findings.iter().map(|f| &f.check).collect::<Vec<_>>()
    );
}

#[test]
fn check_reports_drift_states_against_an_upstream_checkout() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb0 = seeded(&kb_root, &upstream);
    // upstream-only drift on docs/index.md.
    write_file(
        &upstream.join("docs").join("index.md"),
        "---\ntitle: Docs\ndescription: Moved on.\n---\n\n# Docs\n",
    );
    // diverged on docs/types.md.
    write_file(
        &upstream.join("docs").join("types.md"),
        "---\ntitle: Types\ndescription: Theirs.\n---\n\n# Types\n",
    );
    let before = fs::read_to_string(sb0.local_file("docs/types.md")).unwrap();
    fs::write(sb0.local_file("docs/types.md"), before + "\nOurs.\n").unwrap();
    // deleted upstream on schemas/thing.yaml, untracked on docs/fresh.md.
    fs::remove_file(upstream.join("schemas").join("thing.yaml")).unwrap();
    write_file(&upstream.join("docs").join("fresh.md"), "# Fresh\n");
    let (kb, sb) = load_sync(&kb_root);
    let findings = sync::check_findings(&kb, &sb, Some(&upstream)).unwrap();
    let checks: Vec<&str> = findings.iter().map(|f| f.check.as_str()).collect();
    assert!(checks.contains(&"sync-upstream-drift"), "got {checks:?}");
    assert!(checks.contains(&"sync-diverged"), "got {checks:?}");
    assert!(checks.contains(&"sync-deleted-upstream"), "got {checks:?}");
    assert!(checks.contains(&"sync-untracked"), "got {checks:?}");
}

#[test]
fn check_reports_deleted_upstream_edited_and_projection_broken_as_errors() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb0 = seeded(&kb_root, &upstream);
    let before = fs::read_to_string(sb0.local_file("docs/types.md")).unwrap();
    fs::write(sb0.local_file("docs/types.md"), before + "\nEdited.\n").unwrap();
    fs::remove_file(upstream.join("docs").join("types.md")).unwrap();
    let broken = fs::read_to_string(sb0.local_file("docs/index.md")).unwrap();
    fs::write(
        sb0.local_file("docs/index.md"),
        broken.replace("# kb:end\n", ""),
    )
    .unwrap();
    let (kb, sb) = load_sync(&kb_root);
    let findings = sync::check_findings(&kb, &sb, Some(&upstream)).unwrap();
    assert!(
        findings
            .iter()
            .any(|f| f.check == "sync-deleted-upstream-edited" && f.severity == Severity::Error),
        "got {findings:?}"
    );
    assert!(
        findings.iter().any(|f| f.check == "sync-projection-broken"
            && f.severity == Severity::Error
            && f.line == Some(1)),
        "got {findings:?}"
    );
}

#[test]
fn all_sync_findings_reports_an_invalid_manifest_without_a_checkout() {
    let dir = TempDir::new().unwrap();
    let kb_root = dir.path().join("kb");
    let bundle_root = kb_root.join("bundles").join("broken");
    write_file(
        &bundle_root.join("index.md"),
        "---\nokf_version: \"0.2\"\ntitle: Broken\ndescription: A bad manifest.\nsync: true\n---\n\n# Broken\n",
    );
    // A manifest every sync command refuses: it has a repo but no mappings.
    write_file(
        &bundle_root.join("sync.yaml"),
        "upstream:\n  repo: a/b\nroot: sources\n",
    );
    let kb = store::load(&kb_root).unwrap();
    let findings = sync::all_sync_findings(&kb, &dir.path().join(".refs"), true).unwrap();
    let f = findings
        .iter()
        .find(|f| f.check == "sync-manifest-invalid")
        .expect("the load failure becomes a finding");
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(
        f.message,
        "sync.yaml needs at least one entry under `mappings:`"
    );
    assert!(
        f.path.ends_with("bundles/broken/sync.yaml"),
        "got {}",
        f.path
    );
}

// ------------------------------------------------------------ generated index

#[test]
fn generate_index_rewrites_below_the_marker_and_is_stable() {
    let (_dir, kb_root, bundle_root, upstream) = sync_fixture();
    let (_, sb0) = load_sync(&kb_root);
    let result = sync::pull(
        &sb0,
        &upstream,
        "deadbeef00000000",
        today(),
        false,
        false,
        false,
    )
    .unwrap();
    let written = sync::write_lock(&sb0, &result.lock).unwrap();
    // Reload before generating: the bullets have to carry the descriptions of the
    // concepts that were just written, and the in-memory bundle predates them.
    let (_, sb) = load_sync(&kb_root);
    assert!(sync::generate_index(&sb, &written, today()).unwrap());
    let index = fs::read_to_string(bundle_root.join("index.md")).unwrap();
    assert!(index.contains(sync::INDEX_MARKER));
    assert!(
        index.contains(
            "_Generated by `kb sync pull` — do not edit below the marker. Last built 2026-07-28, from acme/spec@deadbeef._"
        ),
        "got: {index}"
    );
    // The bullet text is the concept's own description, verbatim.
    assert!(
        index.contains("* [Types](/sources/docs/types.md) - The types."),
        "got: {index}"
    );
    assert!(index.contains("## Assets (1)"), "got: {index}");
    assert!(
        index.contains("* `sources/schemas/thing.yaml`"),
        "got: {index}"
    );
    // A second pass over unchanged state rewrites nothing.
    let (_, sb2) = load_sync(&kb_root);
    assert!(!sync::generate_index(&sb2, &written, today()).unwrap());
}

// ---------------------------------------------------------------- rendering

#[test]
fn json_field_names_match_the_scala_output() {
    let rows = vec![FileStatus {
        path: "a.md".to_string(),
        kind: SyncKind::Concept,
        state: SyncState::Clean,
        detail: String::new(),
        injection_stale: false,
    }];
    assert_eq!(
        sync::render_status(&rows, true, false),
        "{\n  \"files\": [\n    {\n      \"path\": \"a.md\",\n      \"kind\": \"concept\",\n      \"state\": \"clean\",\n      \"detail\": \"\",\n      \"injectionStale\": false\n    }\n  ],\n  \"summary\": {\n    \"clean\": 1\n  }\n}\n"
    );
    assert_eq!(
        sync::render_actions(&[], &[], true, true),
        "{\n  \"dryRun\": true,\n  \"actions\": [],\n  \"refused\": []\n}\n"
    );
}

#[test]
fn text_rendering_matches_the_scala_shapes() {
    let rows = vec![FileStatus {
        path: "a.md".to_string(),
        kind: SyncKind::Concept,
        state: SyncState::Clean,
        detail: String::new(),
        injection_stale: false,
    }];
    assert_eq!(
        sync::render_status(&rows, false, false),
        "1 file(s), all clean\n"
    );
    let verbose = sync::render_status(&rows, false, true);
    assert!(
        verbose.starts_with("clean             a.md\n"),
        "got: {verbose}"
    );
    assert!(verbose.contains("clean: 1\n"), "got: {verbose}");
    assert_eq!(
        sync::render_actions(&[], &[], false, false),
        "nothing to do\n"
    );
    let acted = sync::render_actions(
        &[sync::SyncAction {
            verb: "added".to_string(),
            path: "docs/a.md".to_string(),
            detail: "concept".to_string(),
        }],
        &[],
        true,
        false,
    );
    assert_eq!(acted, "would added (1)\n  docs/a.md\n");
}

// --------------------------------------------------------------------- diff

#[test]
fn diff_compares_the_upstream_copy_against_the_projected_local_copy() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb = seeded(&kb_root, &upstream);
    let identical = sync::diff(&sb, &upstream, "docs/types.md").unwrap();
    assert_eq!(identical.path, "docs/types.md");
    assert!(identical.identical, "got: {}", identical.diff);
    assert!(identical.diff.trim().is_empty(), "got: {}", identical.diff);
    let before = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    fs::write(sb.local_file("docs/types.md"), before + "\nAn addition.\n").unwrap();
    let changed = sync::diff(&sb, &upstream, "docs/types.md").unwrap();
    assert!(!changed.identical);
    assert!(
        changed.diff.contains("+An addition."),
        "got: {}",
        changed.diff
    );
    assert!(
        !changed.diff.contains("kb:begin"),
        "the diff shows the upstream form, never the fence: {}",
        changed.diff
    );
}

/// The same fixture, diffed twice: once unchanged, once after a local edit.
fn diff_both_ways() -> (TempDir, sync::DiffResult, sync::DiffResult) {
    let (dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb = seeded(&kb_root, &upstream);
    let identical = sync::diff(&sb, &upstream, "docs/types.md").unwrap();
    let before = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    fs::write(sb.local_file("docs/types.md"), before + "\nAn addition.\n").unwrap();
    let changed = sync::diff(&sb, &upstream, "docs/types.md").unwrap();
    (dir, identical, changed)
}

#[test]
fn render_diff_text_form_is_unchanged() {
    let (_dir, identical, changed) = diff_both_ways();
    assert_eq!(
        sync::render_diff_text(&identical),
        "docs/types.md: identical\n",
        "the identical line is byte-for-byte what the CLI printed before"
    );
    assert_eq!(
        sync::render_diff_text(&changed),
        changed.diff,
        "a changed file renders git's output verbatim, as `print!` did"
    );
}

#[test]
fn render_diff_json_is_machine_readable_in_both_cases() {
    let (_dir, identical, changed) = diff_both_ways();

    let text = sync::render_diff_json(&identical);
    assert!(
        text.ends_with('\n'),
        "payloads end in a newline like the rest"
    );
    let v: serde_json::Value = serde_json::from_str(&text).expect("identical case parses");
    assert_eq!(v["path"], "docs/types.md");
    assert_eq!(v["identical"], true);
    assert_eq!(v["diff"], "");
    assert_eq!(v["patch"], "");

    let v: serde_json::Value =
        serde_json::from_str(&sync::render_diff_json(&changed)).expect("changed case parses");
    assert_eq!(v["path"], "docs/types.md");
    assert_eq!(v["identical"], false);
    assert!(
        v["diff"].as_str().unwrap().contains("+An addition."),
        "the unified diff travels in the payload: {v}"
    );
    assert!(
        v["patch"]
            .as_str()
            .unwrap()
            .starts_with("diff --git a/docs/types.md b/docs/types.md\n"),
        "and so does the applicable patch: {v}"
    );
}

#[test]
fn render_diff_raw_emits_git_bytes_and_nothing_else() {
    let (_dir, identical, changed) = diff_both_ways();
    assert_eq!(
        sync::render_diff_raw(&identical),
        "",
        "an identical pair is an empty patch — any decoration would corrupt the pipe"
    );
    let raw = sync::render_diff_raw(&changed);
    assert_eq!(raw, changed.patch, "git's bytes, unwrapped and unpadded");
    assert!(raw.contains("+An addition."));
    assert!(
        !raw.contains("docs/types.md: identical"),
        "the human line never leaks into the raw form"
    );
}

#[test]
fn render_diff_raw_headers_are_relative_to_the_upstream_root() {
    let (_dir, _identical, changed) = diff_both_ways();
    let raw = sync::render_diff_raw(&changed);
    assert!(
        raw.starts_with("diff --git a/docs/types.md b/docs/types.md\n"),
        "the headers name the mirrored path, not a checkout or a temp file: {raw}"
    );
    assert!(raw.contains("\n--- a/docs/types.md\n"), "got: {raw}");
    assert!(raw.contains("\n+++ b/docs/types.md\n"), "got: {raw}");
    assert!(
        !raw.contains("kb-sync-"),
        "no scratch path survives into the patch: {raw}"
    );
    assert!(
        sync::render_diff_text(&changed).contains("kb-sync-"),
        "while the human form keeps the real paths it always showed"
    );
}

#[test]
fn render_diff_raw_produces_a_patch_that_git_apply_lands_upstream() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb = seeded(&kb_root, &upstream);
    let before = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    fs::write(sb.local_file("docs/types.md"), before + "\nAn addition.\n").unwrap();
    let changed = sync::diff(&sb, &upstream, "docs/types.md").unwrap();

    // A scratch checkout holding upstream's copy at the mirrored path — the
    // only place the patch claims to fit.
    let repo = TempDir::new().unwrap();
    run_git(repo.path(), &["init", "-q"]);
    let target = repo.path().join("docs").join("types.md");
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::copy(upstream.join("docs").join("types.md"), &target).unwrap();
    run_git(repo.path(), &["add", "-A"]);

    let patch_file = repo.path().join("kb.patch");
    fs::write(&patch_file, sync::render_diff_raw(&changed)).unwrap();
    let checked = std::process::Command::new("git")
        .current_dir(repo.path())
        .args(GIT_PIN)
        .args(["apply", "--check", "kb.patch"])
        .output()
        .unwrap();
    assert!(
        checked.status.success(),
        "git apply --check refused the patch: {}",
        String::from_utf8_lossy(&checked.stderr)
    );
    run_git(repo.path(), &["apply", "kb.patch"]);

    // And what it lands is our projected form, byte for byte.
    let projected =
        sync::project(&fs::read_to_string(sb.local_file("docs/types.md")).unwrap()).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), projected);
}

/// Git config these tests must not inherit from the developer's global config.
///
/// `core.autocrlf=true` is the default on a Windows git install, and it rewrites
/// line endings on both `add` and `apply`. The assertions below compare mirrored
/// files byte for byte, so an inherited setting would decide whether they pass.
const GIT_PIN: [&str; 4] = ["-c", "core.autocrlf=false", "-c", "core.eol=lf"];

/// Runs git in `dir` and insists it succeeded.
fn run_git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(GIT_PIN)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ------------------------------------------------- diff: containment and gaps

/// A bundle whose mirror root is two directories deep.
///
/// The depth is the point. `mirror_file` judges containment against the bundle
/// root, which sits that far above the resolved path, so it absorbs two `..`
/// segments and calls the result contained. The diff's own scratch tree is only
/// one directory above `scratch/a` and `scratch/b`, so the very same `rel`
/// climbs out of it — the two containment roots cannot be made to agree.
fn deep_mirror_fixture() -> (TempDir, PathBuf, PathBuf) {
    let dir = TempDir::new().unwrap();
    let kb_root = dir.path().join("kb");
    let bundle_root = kb_root.join("bundles").join("vendored");
    write_file(
        &bundle_root.join("index.md"),
        "---\nokf_version: \"0.2\"\ntitle: Vendored\ndescription: Mirrored upstream material.\nsync: true\n---\n\n# Vendored\n\nMirrored upstream material.\n\n## Orientation\n",
    );
    write_file(
        &bundle_root.join("sync.yaml"),
        "upstream:\n  repo: acme/spec\n  refs_path: acme/spec\nroot: mirror/sources\nmappings:\n  - \"docs/**\"\n",
    );
    // Real directories, so the `..` segments below are resolvable by the OS
    // rather than failing before anything is written.
    fs::create_dir_all(bundle_root.join("mirror").join("sources")).unwrap();
    (dir, kb_root, bundle_root)
}

#[test]
fn diff_refuses_a_rel_that_climbs_out_of_the_scratch_directory() {
    let (dir, kb_root, bundle_root) = deep_mirror_fixture();

    // The scratch directory lives directly under the system temp directory, and
    // so does the fixture, so two `..` segments from `scratch/a` land on a
    // sentinel that has nothing to do with either.
    let victim = std::env::temp_dir().join(format!("kb-diff-victim-{}", std::process::id()));
    let _ = fs::remove_dir_all(&victim);
    fs::create_dir_all(&victim).unwrap();
    let sentinel = victim.join("evil.md");
    fs::write(&sentinel, "SENTINEL\n").unwrap();
    let name = victim.file_name().unwrap().to_str().unwrap().to_string();
    let rel = format!("../../{name}/evil.md");

    // Both sides of the comparison exist, so nothing short of the guard stops
    // the staging writes: the mirrored file where the rel really lands inside
    // the bundle, and an upstream copy where it lands from the checkout root.
    write_file(
        &bundle_root.join(&name).join("evil.md"),
        "---\ntitle: Evil\n---\n\n# Evil\n",
    );
    let upstream = dir.path().join("nested").join("upstream");
    fs::create_dir_all(&upstream).unwrap();
    write_file(
        &dir.path().join(&name).join("evil.md"),
        "---\ntitle: Upstream\n---\n\n# Upstream\n",
    );

    let (_, sb) = load_sync(&kb_root);
    let outcome = sync::diff(&sb, &upstream, &rel);
    let survived = fs::read_to_string(&sentinel).unwrap();
    let _ = fs::remove_dir_all(&victim);
    assert_eq!(
        survived, "SENTINEL\n",
        "a diff writes inside its own scratch directory and nowhere else"
    );
    let err = outcome.unwrap_err();
    assert!(
        err.to_string().contains(&rel),
        "the refusal names the path it refused: {err}"
    );
}

/// A scratch checkout of `upstream`, minus `absent`, as a git repository — the
/// only place a patch from `diff` claims to fit.
fn checkout_of(upstream: &Path, absent: Option<&str>) -> TempDir {
    let repo = TempDir::new().unwrap();
    run_git(repo.path(), &["init", "-q"]);
    for rel in sync::relative_files_under(upstream).unwrap() {
        if Some(rel.as_str()) == absent {
            continue;
        }
        let target = sync::resolve(repo.path(), &rel);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(upstream.join(&rel), &target).unwrap();
    }
    run_git(repo.path(), &["add", "-A"]);
    repo
}

/// `git apply` in `repo`, insisting the patch both checks and lands.
fn apply_patch(repo: &Path, patch: &str) {
    let patch_file = repo.join("kb.patch");
    fs::write(&patch_file, patch).unwrap();
    let checked = std::process::Command::new("git")
        .current_dir(repo)
        .args(GIT_PIN)
        .args(["apply", "--check", "kb.patch"])
        .output()
        .unwrap();
    assert!(
        checked.status.success(),
        "git apply --check refused the patch:\n{patch}\n{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    run_git(repo, &["apply", "kb.patch"]);
}

#[test]
fn diff_of_a_file_deleted_upstream_but_edited_here_is_not_identical() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb = seeded(&kb_root, &upstream);
    let before = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    fs::write(sb.local_file("docs/types.md"), before + "\nA local edit.\n").unwrap();
    fs::remove_file(upstream.join("docs").join("types.md")).unwrap();

    let d = sync::diff(&sb, &upstream, "docs/types.md").unwrap();
    assert!(
        !d.identical,
        "a file that exists here and is gone upstream is not identical: {d:?}"
    );
    assert!(
        d.diff.contains("A local edit."),
        "the human diff shows what would go upstream: {}",
        d.diff
    );
    assert!(
        d.patch
            .starts_with("diff --git a/docs/types.md b/docs/types.md\n"),
        "the patch still names the mirrored path: {}",
        d.patch
    );
    assert_eq!(sync::render_diff_raw(&d), d.patch);
    assert_ne!(sync::render_diff_text(&d), "docs/types.md: identical\n");

    // And it lands in a checkout where the file is gone, which is the only
    // state the upstream repository can be in for this to have happened.
    let repo = checkout_of(&upstream, Some("docs/types.md"));
    apply_patch(repo.path(), &d.patch);
    let projected =
        sync::project(&fs::read_to_string(sb.local_file("docs/types.md")).unwrap()).unwrap();
    assert_eq!(
        fs::read_to_string(repo.path().join("docs").join("types.md")).unwrap(),
        projected
    );
}

#[test]
fn diff_of_a_file_missing_locally_is_a_deletion_rather_than_an_unattributed_io_error() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb = seeded(&kb_root, &upstream);
    fs::remove_file(sb.local_file("docs/types.md")).unwrap();

    let d = sync::diff(&sb, &upstream, "docs/types.md").unwrap();
    assert!(!d.identical, "got: {d:?}");
    assert!(
        d.patch
            .starts_with("diff --git a/docs/types.md b/docs/types.md\n"),
        "got: {}",
        d.patch
    );

    let repo = checkout_of(&upstream, None);
    apply_patch(repo.path(), &d.patch);
    assert!(
        !repo.path().join("docs").join("types.md").exists(),
        "the patch takes the file away, which is what the mirror now says"
    );
}

#[test]
fn diff_of_a_path_on_neither_side_names_the_path_and_the_situation() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb = seeded(&kb_root, &upstream);
    let err = sync::diff(&sb, &upstream, "docs/never-existed.md").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("docs/never-existed.md"), "got: {msg}");
    assert!(
        !msg.contains("os error"),
        "a bare IO error attributes nothing: {msg}"
    );
}

// ------------------------------------------------------- diff: many at once

/// The seeded fixture with `docs/types.md` and `schemas/thing.yaml` edited here,
/// leaving `docs/index.md` agreeing with upstream.
/// A fixture whose lockfile lists a path that neither side still holds — the
/// ghost the sweep passes over.
fn with_a_ghost() -> (TempDir, sync::SyncBundle, PathBuf) {
    let (dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let _ = seeded(&kb_root, &upstream);
    fs::remove_file(upstream.join("docs").join("types.md")).unwrap();
    let (_, sb) = load_sync(&kb_root);
    fs::remove_file(sb.local_file("docs/types.md")).unwrap();
    let (_, sb) = load_sync(&kb_root);
    (dir, sb, upstream)
}

#[test]
fn a_ghost_in_the_lockfile_is_counted_absent_not_compared() {
    let (_dir, sb, upstream) = with_a_ghost();
    let set = sync::diff_many(&sb, &upstream, &[]).unwrap();
    assert!(set.absent >= 1, "the ghost is counted");
    assert_eq!(set.compared(), set.matched - set.absent);
    let sel = sync::DiffSelection::Many(set);
    let v: serde_json::Value = serde_json::from_str(&sync::render_diffs_json(&sel)).unwrap();
    assert_eq!(v["summary"]["absent"], 1);
    // The tally never claims the ghost was compared, and it points the reader
    // at the tool whose job the ghost is.
    let text = sync::render_diffs_text(&sel);
    assert!(
        text.contains("listed in the lockfile absent on both sides — see `kb sync status`"),
        "got: {text}"
    );
}

#[test]
fn a_selection_reaching_only_ghosts_says_so_rather_than_claiming_agreement() {
    let (_dir, sb, upstream) = with_a_ghost();
    let set = sync::diff_many(&sb, &upstream, &select(&["docs/types.md"])).unwrap();
    assert_eq!((set.compared(), set.absent), (0, 1));
    let sel = sync::DiffSelection::Many(set);
    assert_eq!(
        sync::render_diffs_text(&sel),
        "1 path(s) matched, none present on either side — see `kb sync status`\n"
    );
    assert_eq!(sync::render_diffs_raw(&sel), "", "still nothing to apply");
}

fn two_of_three_differ() -> (TempDir, sync::SyncBundle, PathBuf) {
    let (dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb = seeded(&kb_root, &upstream);
    let before = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    fs::write(sb.local_file("docs/types.md"), before + "\nAn addition.\n").unwrap();
    let before = fs::read_to_string(sb.local_file("schemas/thing.yaml")).unwrap();
    fs::write(
        sb.local_file("schemas/thing.yaml"),
        before + "extra: true\n",
    )
    .unwrap();
    (dir, sb, upstream)
}

/// A selection list, as the CLI will hand one over.
fn select(patterns: &[&str]) -> Vec<String> {
    patterns.iter().map(|p| p.to_string()).collect()
}

fn paths_of(files: &[sync::DiffResult]) -> Vec<&str> {
    files.iter().map(|d| d.path.as_str()).collect()
}

#[test]
fn diff_without_a_pattern_covers_every_differing_file_and_omits_the_rest() {
    let (_dir, sb, upstream) = two_of_three_differ();
    let set = sync::diff_many(&sb, &upstream, &[]).unwrap();
    assert_eq!(
        paths_of(&set.files),
        vec!["docs/types.md", "schemas/thing.yaml"],
        "docs/index.md agrees with upstream, so it is not part of the diff"
    );
    assert_eq!(set.matched, 3, "all three mirrored files were considered");
    assert!(
        set.files.iter().all(|d| !d.identical),
        "an identical file never reaches the output: {:?}",
        paths_of(&set.files)
    );
}

#[test]
fn diff_with_a_glob_covers_the_subset_it_matches() {
    let (_dir, sb, upstream) = two_of_three_differ();
    let set = sync::diff_many(&sb, &upstream, &select(&["docs/**"])).unwrap();
    assert_eq!(paths_of(&set.files), vec!["docs/types.md"]);
    assert_eq!(set.matched, 2, "both files under docs/ were considered");
}

#[test]
fn diff_glob_uses_the_manifest_glob_dialect() {
    let (_dir, sb, upstream) = two_of_three_differ();
    // `**/` also matches zero directories, exactly as a `sync.yaml` mapping does.
    let set = sync::diff_many(&sb, &upstream, &select(&["**/*.yaml"])).unwrap();
    assert_eq!(paths_of(&set.files), vec!["schemas/thing.yaml"]);
    // `?` is one character and never a separator.
    let set = sync::diff_many(&sb, &upstream, &select(&["docs/type?.md"])).unwrap();
    assert_eq!(paths_of(&set.files), vec!["docs/types.md"]);
}

#[test]
fn several_patterns_are_a_union_taken_once_and_in_path_order() {
    let (_dir, sb, upstream) = two_of_three_differ();
    // Overlapping, and in the wrong order: `docs/types.md` is selected twice.
    let set = sync::diff_many(
        &sb,
        &upstream,
        &select(&["schemas/**", "docs/types.md", "docs/**"]),
    )
    .unwrap();
    assert_eq!(
        paths_of(&set.files),
        vec!["docs/types.md", "schemas/thing.yaml"],
        "a file selected twice appears once, in mirrored-path order"
    );
    assert_eq!(set.matched, 3);
    // The same union, however the patterns arrive.
    let same = sync::diff_many(
        &sb,
        &upstream,
        &select(&["docs/**", "docs/types.md", "schemas/**"]),
    )
    .unwrap();
    assert_eq!(paths_of(&same.files), paths_of(&set.files));
    assert_eq!(
        sync::render_diffs_raw(&sync::DiffSelection::Many(same)),
        sync::render_diffs_raw(&sync::DiffSelection::Many(set)),
        "so the patch is byte-reproducible whatever order the pipe delivered"
    );
}

#[test]
fn a_glob_matching_one_file_reports_what_the_literal_path_reports() {
    let (_dir, sb, upstream) = two_of_three_differ();
    let literal = sync::diff(&sb, &upstream, "docs/types.md").unwrap();
    let set = sync::diff_many(&sb, &upstream, &select(&["docs/typ*.md"])).unwrap();
    assert_eq!(set.files.len(), 1);
    let matched = &set.files[0];
    assert_eq!(matched.path, literal.path);
    assert_eq!(matched.identical, literal.identical);
    // The patch is the byte-for-byte comparison; the human diff names a scratch
    // file whose name is unique per call, so only the patch can be compared.
    assert_eq!(matched.patch, literal.patch);
    assert_eq!(
        sync::render_diffs_raw(&sync::DiffSelection::Many(set)),
        sync::render_diff_raw(&literal),
        "one differing file makes the same patch either way"
    );
}

#[test]
fn a_pattern_matching_nothing_is_refused_by_name() {
    let (_dir, sb, upstream) = two_of_three_differ();
    for pattern in ["docs/nowhere/*.md", "docs/never-existed.md"] {
        let err = sync::diff_many(&sb, &upstream, &select(&[pattern]))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(pattern),
            "the refusal names the pattern it refused: {err}"
        );
        assert!(
            err.contains("kb sync status"),
            "and says where the known paths are listed: {err}"
        );
    }
}

#[test]
fn every_pattern_that_matched_nothing_is_named_in_one_refusal() {
    let (_dir, sb, upstream) = two_of_three_differ();
    let err = sync::diff_many(
        &sb,
        &upstream,
        &select(&["docs/**", "docs/nowhere/*.md", "schemas/nope.yaml"]),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("docs/nowhere/*.md"), "got: {err}");
    assert!(
        err.contains("schemas/nope.yaml"),
        "the second failure is named too, rather than waiting for another run: {err}"
    );
    assert!(
        !err.contains("`docs/**`"),
        "and the pattern that did match is not accused: {err}"
    );
}

#[test]
fn a_mirror_that_agrees_with_upstream_everywhere_is_not_an_error() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb = seeded(&kb_root, &upstream);
    let set = sync::diff_many(&sb, &upstream, &[]).unwrap();
    assert!(set.files.is_empty(), "got: {:?}", paths_of(&set.files));
    assert_eq!(set.matched, 3);
    let sel = sync::DiffSelection::Many(set);
    assert_eq!(
        sync::render_diffs_raw(&sel),
        "",
        "nothing to apply, so nothing is printed"
    );
    assert_eq!(
        sync::render_diffs_text(&sel),
        "3 file(s) compared, no differences\n"
    );
    let v: serde_json::Value = serde_json::from_str(&sync::render_diffs_json(&sel)).unwrap();
    assert_eq!(v["files"].as_array().unwrap().len(), 0);
    assert_eq!(v["summary"]["differing"], 0);
    assert_eq!(v["summary"]["compared"], 3);
    assert_eq!(v["summary"]["absent"], 0);
}

#[test]
fn a_binary_asset_that_matches_upstream_is_not_reported_as_differing() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    // Bytes that are not valid UTF-8: decoding them would substitute U+FFFD and
    // make a freshly pulled file look locally modified.
    write_bin(
        &upstream.join("schemas").join("logo.png"),
        &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0xff, 0xfe, 0x0a],
    );
    let sb = seeded(&kb_root, &upstream);
    let set = sync::diff_many(&sb, &upstream, &[]).unwrap();
    assert!(
        set.files.is_empty(),
        "a byte-identical asset differs from nothing: {:?}",
        paths_of(&set.files)
    );
    assert_eq!(set.matched, 4);
}

#[test]
fn the_order_of_a_multi_file_diff_is_stable_across_runs() {
    let (_dir, sb, upstream) = two_of_three_differ();
    let first = sync::diff_many(&sb, &upstream, &[]).unwrap();
    let second = sync::diff_many(&sb, &upstream, &[]).unwrap();
    assert_eq!(paths_of(&first.files), paths_of(&second.files));
    let mut sorted = paths_of(&first.files);
    sorted.sort_unstable();
    assert_eq!(
        paths_of(&first.files),
        sorted,
        "mirrored path order, so the patch is byte-reproducible"
    );
    assert_eq!(
        sync::render_diffs_raw(&sync::DiffSelection::Many(first)),
        sync::render_diffs_raw(&sync::DiffSelection::Many(second))
    );
}

#[test]
fn the_multi_file_raw_patch_applies_as_one_patch() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    let sb = seeded(&kb_root, &upstream);
    let before = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    fs::write(sb.local_file("docs/types.md"), before + "\nAn addition.\n").unwrap();
    let before = fs::read_to_string(sb.local_file("schemas/thing.yaml")).unwrap();
    fs::write(
        sb.local_file("schemas/thing.yaml"),
        before + "extra: true\n",
    )
    .unwrap();
    // A modification, an edited asset, and a deletion in one patch.
    fs::remove_file(sb.local_file("docs/index.md")).unwrap();

    let set = sync::diff_many(&sb, &upstream, &[]).unwrap();
    assert_eq!(
        paths_of(&set.files),
        vec!["docs/index.md", "docs/types.md", "schemas/thing.yaml"]
    );
    let raw = sync::render_diffs_raw(&sync::DiffSelection::Many(set));

    let repo = checkout_of(&upstream, None);
    apply_patch(repo.path(), &raw);

    for rel in ["docs/types.md", "schemas/thing.yaml"] {
        let projected = sync::project(&fs::read_to_string(sb.local_file(rel)).unwrap()).unwrap();
        assert_eq!(
            fs::read_to_string(sync::resolve(repo.path(), rel)).unwrap(),
            projected,
            "{rel} lands as the mirror's own form"
        );
    }
    assert!(
        !repo.path().join("docs").join("index.md").exists(),
        "and the deletion lands too"
    );
}

#[test]
fn a_glob_cannot_smuggle_a_parent_segment_past_the_guard() {
    let (dir, kb_root, bundle_root) = deep_mirror_fixture();
    let victim = std::env::temp_dir().join(format!("kb-glob-victim-{}", std::process::id()));
    let _ = fs::remove_dir_all(&victim);
    fs::create_dir_all(&victim).unwrap();
    let sentinel = victim.join("evil.md");
    fs::write(&sentinel, "SENTINEL\n").unwrap();
    let name = victim.file_name().unwrap().to_str().unwrap().to_string();
    write_file(
        &bundle_root.join(&name).join("evil.md"),
        "---\ntitle: Evil\n---\n\n# Evil\n",
    );
    let upstream = dir.path().join("nested").join("upstream");
    fs::create_dir_all(&upstream).unwrap();
    write_file(
        &dir.path().join(&name).join("evil.md"),
        "---\ntitle: Upstream\n---\n\n# Upstream\n",
    );

    let (_, sb) = load_sync(&kb_root);
    let pattern = format!("../../{name}/*.md");
    // Refused whether it arrives alone or hidden among patterns that are fine.
    for patterns in [
        select(&[&pattern]),
        select(&["docs/**", &pattern, "schemas/**"]),
    ] {
        let outcome = sync::diff_many(&sb, &upstream, &patterns);
        let survived = fs::read_to_string(&sentinel).unwrap();
        assert_eq!(
            survived, "SENTINEL\n",
            "a glob writes inside the scratch directory and nowhere else"
        );
        let err = outcome.unwrap_err().to_string();
        assert!(
            err.contains(&pattern),
            "the refusal names the pattern it refused: {err}"
        );
        assert!(
            err.contains("the mirror"),
            "and refuses it on containment, not on having matched nothing: {err}"
        );
    }
    let _ = fs::remove_dir_all(&victim);

    // And on a mirror that does hold files, so what refuses the pattern is the
    // guard rather than its having matched nothing.
    let (_dir, sb, upstream) = two_of_three_differ();
    let err = sync::diff_many(&sb, &upstream, &select(&["docs/**", "../*/*.md"]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("`../*/*.md` leaves the mirror"), "got: {err}");
}

#[test]
fn a_mirror_holding_nothing_says_so_rather_than_matching_nothing() {
    let (dir, kb_root, _bundle_root) = deep_mirror_fixture();
    let upstream = dir.path().join("empty-upstream");
    fs::create_dir_all(&upstream).unwrap();
    let (_, sb) = load_sync(&kb_root);
    let err = sync::diff_many(&sb, &upstream, &[])
        .unwrap_err()
        .to_string();
    assert!(err.contains("kb sync pull"), "got: {err}");
}

#[test]
fn the_diff_file_set_is_the_one_status_reports_on() {
    let (_dir, sb, upstream) = two_of_three_differ();
    let from_status: Vec<String> = sync::status(&sb, Some(&upstream))
        .unwrap()
        .into_iter()
        .map(|r| r.path)
        .collect();
    assert_eq!(
        sync::known_paths(&sb, Some(&upstream)).unwrap(),
        from_status
    );
    assert_eq!(
        sync::diff_many(&sb, &upstream, &[]).unwrap().matched,
        from_status.len()
    );
}

// --------------------------------------------- diff: selecting and rendering

#[test]
fn one_literal_path_is_a_single_diff_and_everything_else_is_not() {
    let (_dir, sb, upstream) = two_of_three_differ();
    let single = sync::diff_selected(&sb, &upstream, &select(&["docs/types.md"])).unwrap();
    assert!(matches!(single, sync::DiffSelection::Single(_)));
    for patterns in [
        select(&[]),
        select(&["docs/**"]),
        select(&["docs/type?.md"]),
        // Two literals are still a set of files, not the single-file case.
        select(&["docs/types.md", "schemas/thing.yaml"]),
    ] {
        let many = sync::diff_selected(&sb, &upstream, &patterns).unwrap();
        assert!(
            matches!(many, sync::DiffSelection::Many(_)),
            "got a single diff for {patterns:?}"
        );
    }
    assert!(sync::is_glob("docs/**"));
    assert!(sync::is_glob("docs/type?.md"));
    assert!(!sync::is_glob("docs/types.md"));
}

#[test]
fn a_single_file_selection_renders_exactly_as_it_did_before() {
    let (_dir, sb, upstream) = two_of_three_differ();
    let one = sync::diff(&sb, &upstream, "docs/types.md").unwrap();
    let sel = sync::DiffSelection::Single(one.clone());
    assert_eq!(sync::render_diffs_text(&sel), sync::render_diff_text(&one));
    assert_eq!(sync::render_diffs_json(&sel), sync::render_diff_json(&one));
    assert_eq!(sync::render_diffs_raw(&sel), sync::render_diff_raw(&one));
    // Including the identical case, whose one line the CLI has always printed.
    let same = sync::diff(&sb, &upstream, "docs/index.md").unwrap();
    assert_eq!(
        sync::render_diffs_text(&sync::DiffSelection::Single(same)),
        "docs/index.md: identical\n"
    );
}

#[test]
fn multi_file_text_names_every_file_it_shows() {
    let (_dir, sb, upstream) = two_of_three_differ();
    let sel = sync::DiffSelection::Many(sync::diff_many(&sb, &upstream, &[]).unwrap());
    let text = sync::render_diffs_text(&sel);
    assert!(text.contains("=== docs/types.md ===\n"), "got: {text}");
    assert!(text.contains("=== schemas/thing.yaml ===\n"), "got: {text}");
    assert!(
        !text.contains("=== docs/index.md ==="),
        "an identical file has no section: {text}"
    );
    assert!(text.contains("+An addition."), "got: {text}");
    assert!(
        text.ends_with("\n2 of 3 file(s) differ\n"),
        "and it closes with the tally: {text}"
    );
}

#[test]
fn multi_file_json_is_an_object_carrying_the_records_and_a_count() {
    let (_dir, sb, upstream) = two_of_three_differ();
    let sel = sync::DiffSelection::Many(sync::diff_many(&sb, &upstream, &[]).unwrap());
    let text = sync::render_diffs_json(&sel);
    assert!(
        text.ends_with('\n'),
        "payloads end in a newline like the rest"
    );
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let files = v["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    // Each record is the shape the single-file payload has, so one parser reads both.
    assert_eq!(files[0]["path"], "docs/types.md");
    assert_eq!(files[0]["identical"], false);
    assert!(
        files[0]["patch"]
            .as_str()
            .unwrap()
            .starts_with("diff --git a/docs/types.md b/docs/types.md\n"),
        "got: {v}"
    );
    assert_eq!(files[1]["path"], "schemas/thing.yaml");
    assert_eq!(v["summary"]["differing"], 2);
    assert_eq!(v["summary"]["compared"], 3);
    assert_eq!(v["summary"]["absent"], 0);
}

#[test]
fn multi_file_raw_is_the_patches_in_path_order_and_nothing_else() {
    let (_dir, sb, upstream) = two_of_three_differ();
    let set = sync::diff_many(&sb, &upstream, &[]).unwrap();
    let joined: String = set.files.iter().map(|d| d.patch.clone()).collect();
    let raw = sync::render_diffs_raw(&sync::DiffSelection::Many(set));
    assert_eq!(raw, joined, "git's bytes, concatenated, undecorated");
    assert!(!raw.contains("==="), "no section headers in the raw form");
    assert!(!raw.contains("kb-sync-"), "no scratch path survives: {raw}");
    assert_eq!(
        raw.matches("diff --git ").count(),
        2,
        "one file header per differing file"
    );
}

#[test]
fn a_changed_binary_asset_makes_a_patch_that_lands_like_any_other() {
    let (_dir, kb_root, _bundle_root, upstream) = sync_fixture();
    write_bin(
        &upstream.join("schemas").join("logo.png"),
        &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0xff, 0xfe, 0x0a],
    );
    let sb = seeded(&kb_root, &upstream);
    write_bin(
        &sb.local_file("schemas/logo.png"),
        &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x00, 0x01, 0x0a],
    );
    // A file the mirror changed sits in the same patch as everything else, so a
    // patch git refuses to apply would take the whole multi-file patch with it.
    let before = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    fs::write(sb.local_file("docs/types.md"), before + "\nAn addition.\n").unwrap();
    let set = sync::diff_many(&sb, &upstream, &[]).unwrap();
    assert_eq!(
        paths_of(&set.files),
        vec!["docs/types.md", "schemas/logo.png"]
    );

    let repo = checkout_of(&upstream, None);
    apply_patch(
        repo.path(),
        &sync::render_diffs_raw(&sync::DiffSelection::Many(set)),
    );
    assert_eq!(
        fs::read(repo.path().join("schemas").join("logo.png")).unwrap(),
        fs::read(sb.local_file("schemas/logo.png")).unwrap(),
        "the bytes the mirror holds are the bytes that land"
    );
}
