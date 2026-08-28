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
    assert!(
        resolved
            .iter()
            .any(|p| p.exists() && p.to_string_lossy().ends_with("/sources/docs/types.md")),
        "got {resolved:?}"
    );
    assert!(
        resolved
            .iter()
            .any(|p| !p.exists() && p.to_string_lossy().ends_with("missing.md")),
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
    assert!(identical.trim().is_empty(), "got: {identical}");
    let before = fs::read_to_string(sb.local_file("docs/types.md")).unwrap();
    fs::write(sb.local_file("docs/types.md"), before + "\nAn addition.\n").unwrap();
    let changed = sync::diff(&sb, &upstream, "docs/types.md").unwrap();
    assert!(changed.contains("+An addition."), "got: {changed}");
    assert!(
        !changed.contains("kb:begin"),
        "the diff shows the upstream form, never the fence: {changed}"
    );
}
