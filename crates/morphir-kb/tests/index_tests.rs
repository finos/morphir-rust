//! Tests for the SQLite index, ported from `KbIndexSpec` in `KbTests.scala`
//! and extended per the port spec: schema and views, FTS ranking and
//! snippets, link kinds and target resolution, headings, source URL
//! splitting, the read-only query guard, staleness, and rebuild semantics.
//!
//! Fixtures are written to disk by hand (the scaffold module is not used
//! here) and loaded through `morphir_okf::store`, exactly as the CLI does.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, TimeZone, Utc};
use morphir_kb::index::{self, IndexStats, Rows};
use morphir_okf::model::Kb;
use morphir_okf::store;
use tempfile::TempDir;

const DEMO_INDEX: &str = "---\nokf_version: \"0.2\"\ntitle: Demo\ndescription: A scratch bundle.\n---\n\n# Demo\n\nA scratch bundle.\n\n## Orientation\n\n* [Alpha](/alpha.md) - First concept.\n* [Beta](/beta.md) - Second concept.\n";

const ALPHA: &str = "---\ntype: Concept\ntitle: Caching strategy\ndescription: How caching works.\ntags: [cache, perf]\nstatus: draft\nsources:\n  - id: gh\n    resource: https://github.com/acme/widgets/blob/0123456789abcdef0123456789abcdef01234567/docs/cache.md\n    title: Upstream cache doc\n  - resource: https://example.com/other\n---\n\n# Caching strategy\n\nCaching appears here. See [Beta](/beta.md), [gone](/missing.md),\n[sibling](beta.md), [ext](https://example.com/x), [anchor](#eviction-policy).\n\n```bash\n# not a heading\n```\n\n## Eviction policy\n\nBody text mentions eviction and caching.\n";

const BETA: &str = "---\ntype: Concept\ntitle: Beta\ndescription: Beta doc.\n---\n\n# Beta\n\nThe word caching appears once in this body only.\n";

const INTENT_INDEX: &str = "---\nokf_version: \"0.2\"\ntitle: Intent\ndescription: The intent register.\nintent: true\nsystem: pkg:pypi/demo\ncapability_bundle: demo\n---\n\n# Intent\n\nThe intent register.\n";

const INTENT_0001: &str = "---\ntype: Intent\ntitle: Indexed thing\ndescription: Something.\nstate: Backlog\nkind: feature\nbreaking: false\ncreated: 2026-07-28\nstate_since: 2026-07-28\ntags: [a, b]\n---\n\n# 0001 \u{2014} Indexed thing\n\nSomething.\n";

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A minimal knowledge base: one ordinary bundle, optionally an intent
/// bundle alongside it — the shape of the Scala fixture.
fn fixture(root: &Path, with_intent: bool) -> PathBuf {
    let kb_root = root.join("kb");
    let demo = kb_root.join("bundles").join("demo");
    write(&demo.join("index.md"), DEMO_INDEX);
    write(&demo.join("alpha.md"), ALPHA);
    write(&demo.join("beta.md"), BETA);
    if with_intent {
        let intent = kb_root.join("bundles").join("intent");
        write(&intent.join("index.md"), INTENT_INDEX);
        write(&intent.join("0001-indexed-thing.md"), INTENT_0001);
    }
    kb_root
}

fn load(kb_root: &Path) -> Kb {
    store::load(kb_root).unwrap()
}

fn db_path(kb_root: &Path) -> PathBuf {
    // Nested under a directory that does not exist yet, so the build's
    // mkdirs is exercised on every test.
    kb_root
        .parent()
        .unwrap()
        .join(".dev")
        .join("kb")
        .join("index.db")
}

/// Builds the index and returns everything a test needs.
fn built(root: &Path, with_intent: bool) -> (Kb, PathBuf, IndexStats) {
    let kb_root = fixture(root, with_intent);
    let kb = load(&kb_root);
    let db = db_path(&kb_root);
    let stats = index::build(&kb, &db, Utc::now()).unwrap();
    (kb, db, stats)
}

fn single_column(rows: &Rows) -> Vec<Option<String>> {
    rows.rows.iter().map(|r| r[0].clone()).collect()
}

// ---------------------------------------------------------------- building

#[test]
fn build_records_documents_links_and_frontmatter_facets() {
    let tmp = TempDir::new().unwrap();
    let (_kb, db, stats) = built(tmp.path(), true);
    assert!(stats.docs > 0, "documents indexed");
    let rows = index::query(&db, "SELECT state, kind FROM v_intent").unwrap();
    assert_eq!(rows.rows.len(), 1, "one intent row");
    assert_eq!(rows.rows[0][0].as_deref(), Some("Backlog"), "state pivoted");
    assert_eq!(rows.rows[0][1].as_deref(), Some("feature"), "kind pivoted");
}

#[test]
fn build_reports_stats_and_meta() {
    let tmp = TempDir::new().unwrap();
    let (_kb, db, stats) = built(tmp.path(), true);
    assert_eq!(stats.bundles, 2);
    assert_eq!(stats.docs, 5, "index + alpha + beta + intent index + 0001");
    assert_eq!(stats.concepts, 3, "alpha, beta, 0001");
    assert_eq!(stats.tags, 4, "cache, perf, a, b");
    assert_eq!(stats.sources, 2);
    let version = index::query(&db, "SELECT value FROM meta WHERE key = 'schema_version'").unwrap();
    assert_eq!(single_column(&version), vec![Some("2".to_string())]);
    let built_at = index::query(&db, "SELECT value FROM meta WHERE key = 'built_at'").unwrap();
    assert_eq!(built_at.rows.len(), 1, "built_at recorded");
}

#[test]
fn views_are_queryable() {
    let tmp = TempDir::new().unwrap();
    let (_kb, db, _) = built(tmp.path(), true);
    let concepts = index::query(
        &db,
        "SELECT bundle, bundle_path FROM v_concept ORDER BY bundle_path",
    )
    .unwrap();
    assert_eq!(concepts.rows.len(), 3);
    let backlinks = index::query(
        &db,
        "SELECT from_path FROM v_backlink JOIN doc d ON d.id = v_backlink.doc_id WHERE d.bundle_path = '/beta.md'",
    )
    .unwrap();
    let froms = single_column(&backlinks);
    assert!(
        froms.contains(&Some("/index.md".to_string()))
            && froms.contains(&Some("/alpha.md".to_string())),
        "beta has backlinks from the bundle index and alpha, got {froms:?}"
    );
    let orphans = index::query(&db, "SELECT bundle_path FROM v_orphan").unwrap();
    assert_eq!(
        single_column(&orphans),
        vec![Some("/0001-indexed-thing.md".to_string())],
        "only the unlinked intent record is an orphan"
    );
}

// ------------------------------------------------------------------ search

#[test]
fn fts_search_ranks_title_matches_first_and_snippets() {
    let tmp = TempDir::new().unwrap();
    let (_kb, db, _) = built(tmp.path(), false);
    let rows = index::search(&db, "caching", 20).unwrap();
    assert_eq!(
        rows.columns,
        vec![
            "bundle",
            "bundle_path",
            "type",
            "status",
            "title",
            "description",
            "snippet"
        ]
    );
    assert_eq!(rows.rows.len(), 2, "alpha and beta match");
    assert_eq!(
        rows.rows[0][1].as_deref(),
        Some("/alpha.md"),
        "the title match outranks the body-only match"
    );
    assert_eq!(rows.rows[1][1].as_deref(), Some("/beta.md"));
    let snippet = rows.rows[0][6].as_deref().unwrap();
    assert!(
        snippet.contains('[') && snippet.contains(']'),
        "snippet highlights the match, got {snippet:?}"
    );
}

#[test]
fn search_limit_caps_rows() {
    let tmp = TempDir::new().unwrap();
    let (_kb, db, _) = built(tmp.path(), false);
    let rows = index::search(&db, "caching", 1).unwrap();
    assert_eq!(rows.rows.len(), 1);
}

// ------------------------------------------------------------------- links

#[test]
fn link_kinds_and_target_resolution() {
    let tmp = TempDir::new().unwrap();
    let (_kb, db, _) = built(tmp.path(), false);
    let rows = index::query(
        &db,
        "SELECT l.dest, l.kind, l.target_doc_id FROM link l JOIN doc d ON d.id = l.doc_id WHERE d.bundle_path = '/alpha.md' ORDER BY l.id",
    )
    .unwrap();
    let got: Vec<(Option<String>, Option<String>, bool)> = rows
        .rows
        .iter()
        .map(|r| (r[0].clone(), r[1].clone(), r[2].is_some()))
        .collect();
    let expect = |dest: &str, kind: &str, resolved: bool| {
        assert!(
            got.contains(&(Some(dest.to_string()), Some(kind.to_string()), resolved)),
            "expected ({dest}, {kind}, resolved={resolved}) in {got:?}"
        );
    };
    expect("/beta.md", "bundle", true);
    // A null target on a `bundle` link is a broken link.
    expect("/missing.md", "bundle", false);
    expect("beta.md", "relative", false);
    expect("https://example.com/x", "external", false);
    expect("#eviction-policy", "anchor", false);
}

// ---------------------------------------------------------------- headings

#[test]
fn headings_skip_fenced_code_blocks() {
    let tmp = TempDir::new().unwrap();
    let (_kb, db, _) = built(tmp.path(), false);
    let rows = index::query(
        &db,
        "SELECT h.level, h.text, h.slug FROM heading h JOIN doc d ON d.id = h.doc_id WHERE d.bundle_path = '/alpha.md' ORDER BY h.line",
    )
    .unwrap();
    let texts: Vec<Option<String>> = rows.rows.iter().map(|r| r[1].clone()).collect();
    assert_eq!(
        texts,
        vec![
            Some("Caching strategy".to_string()),
            Some("Eviction policy".to_string())
        ],
        "the shell comment inside the fence is not a heading"
    );
    assert_eq!(rows.rows[1][0].as_deref(), Some("2"));
    assert_eq!(rows.rows[1][2].as_deref(), Some("eviction-policy"));
}

// ----------------------------------------------------------------- sources

#[test]
fn source_github_urls_split_into_provenance_columns() {
    let tmp = TempDir::new().unwrap();
    let (_kb, db, _) = built(tmp.path(), false);
    let rows = index::query(
        &db,
        "SELECT s.source_id, s.org, s.repo, s.commit_sha, s.src_path FROM source s JOIN doc d ON d.id = s.doc_id WHERE d.bundle_path = '/alpha.md' ORDER BY s.id",
    )
    .unwrap();
    assert_eq!(rows.rows.len(), 2);
    let gh = &rows.rows[0];
    assert_eq!(gh[0].as_deref(), Some("gh"));
    assert_eq!(gh[1].as_deref(), Some("acme"));
    assert_eq!(gh[2].as_deref(), Some("widgets"));
    assert_eq!(
        gh[3].as_deref(),
        Some("0123456789abcdef0123456789abcdef01234567")
    );
    assert_eq!(gh[4].as_deref(), Some("docs/cache.md"));
    let other = &rows.rows[1];
    assert!(
        other[1].is_none() && other[2].is_none() && other[3].is_none() && other[4].is_none(),
        "a non-GitHub resource leaves the provenance columns null"
    );
}

// ------------------------------------------------------------------- query

#[test]
fn query_refuses_anything_not_read_only() {
    let tmp = TempDir::new().unwrap();
    let (_kb, db, _) = built(tmp.path(), false);
    let err = index::query(&db, "DELETE FROM doc").unwrap_err();
    assert_eq!(
        err.to_string(),
        "refusing to run `delete`: kb query is read-only (SELECT, WITH, PRAGMA, EXPLAIN)"
    );
    let err = index::query(&db, "  UPDATE doc SET title = 'x';  ").unwrap_err();
    assert!(err.to_string().contains("refusing to run `update`"));
    // The doc table survived both refusals.
    let count = index::query(&db, "SELECT count(*) FROM doc").unwrap();
    assert_eq!(single_column(&count), vec![Some("3".to_string())]);
}

#[test]
fn query_accepts_the_read_only_heads() {
    let tmp = TempDir::new().unwrap();
    let (_kb, db, _) = built(tmp.path(), false);
    for sql in [
        "  SELECT 1;  ",
        "WITH t AS (SELECT 1 AS x) SELECT x FROM t",
        "PRAGMA table_info(doc)",
        "EXPLAIN SELECT 1",
    ] {
        let rows = index::query(&db, sql);
        assert!(rows.is_ok(), "{sql:?} must run, got {rows:?}");
        assert!(!rows.unwrap().rows.is_empty(), "{sql:?} returns rows");
    }
}

#[test]
fn query_rows_are_keyed_by_column_name_with_text_values() {
    let tmp = TempDir::new().unwrap();
    let (_kb, db, _) = built(tmp.path(), false);
    let rows = index::query(&db, "SELECT 1 AS n, 'x' AS s, NULL AS missing").unwrap();
    assert_eq!(rows.columns, vec!["n", "s", "missing"]);
    assert_eq!(
        rows.rows,
        vec![vec![Some("1".to_string()), Some("x".to_string()), None]]
    );
}

#[test]
fn query_without_an_index_names_the_missing_db() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("nowhere.db");
    let err = index::query(&db, "SELECT 1").unwrap_err().to_string();
    assert!(
        err.starts_with("no index at ") && err.ends_with("\u{2014} run `kb index` first"),
        "got {err:?}"
    );
}

// ---------------------------------------------------------------- staleness

#[test]
fn status_is_fresh_immediately_after_building() {
    let tmp = TempDir::new().unwrap();
    let kb_root = fixture(tmp.path(), false);
    let kb = load(&kb_root);
    let db = db_path(&kb_root);
    index::build(&kb, &db, Utc::now()).unwrap();
    let st = index::status(&db, &kb).unwrap();
    assert!(
        st.stale.is_empty(),
        "fresh immediately after building, got {:?}",
        st.stale
    );
    assert_eq!(st.docs, 3);
}

#[test]
fn status_lists_files_modified_since_the_build() {
    let tmp = TempDir::new().unwrap();
    let kb_root = fixture(tmp.path(), false);
    let kb = load(&kb_root);
    let db = db_path(&kb_root);
    // A build stamped in the distant past: every file's mtime is newer.
    let past: DateTime<Utc> = Utc.with_ymd_and_hms(2000, 1, 1, 0, 0, 0).unwrap();
    index::build(&kb, &db, past).unwrap();
    let st = index::status(&db, &kb).unwrap();
    assert_eq!(
        st.stale,
        vec![
            "kb/bundles/demo/alpha.md".to_string(),
            "kb/bundles/demo/beta.md".to_string(),
            "kb/bundles/demo/index.md".to_string(),
        ],
        "every file is stale, sorted"
    );
}

#[test]
fn status_notices_a_document_deleted_since_the_build() {
    // Regression ported from Scala: staleness compared modification times,
    // and a deleted file has none — so the index kept serving a document
    // that no longer existed.
    let tmp = TempDir::new().unwrap();
    let kb_root = fixture(tmp.path(), false);
    let kb = load(&kb_root);
    let db = db_path(&kb_root);
    index::build(&kb, &db, Utc::now()).unwrap();
    fs::remove_file(kb_root.join("bundles").join("demo").join("beta.md")).unwrap();
    let kb2 = load(&kb_root);
    let st = index::status(&db, &kb2).unwrap();
    assert!(
        st.stale.iter().any(|s| s.contains("removed since")),
        "got {:?}",
        st.stale
    );
    assert_eq!(st.docs, 2, "docs reports the current count");
}

#[test]
fn status_notices_a_document_added_since_the_build() {
    let tmp = TempDir::new().unwrap();
    let kb_root = fixture(tmp.path(), false);
    let kb = load(&kb_root);
    let db = db_path(&kb_root);
    // A future built_at keeps the new file's mtime out of the stale list, so
    // the count check alone must notice it.
    index::build(&kb, &db, Utc::now() + Duration::hours(1)).unwrap();
    write(
        &kb_root.join("bundles").join("demo").join("gamma.md"),
        "---\ntype: Concept\ntitle: Gamma\ndescription: New.\n---\n\n# Gamma\n",
    );
    let kb2 = load(&kb_root);
    let st = index::status(&db, &kb2).unwrap();
    assert_eq!(
        st.stale,
        vec!["(1 document(s) added since the build)".to_string()],
        "count-based staleness only"
    );
}

#[test]
fn status_without_built_at_asks_for_a_rebuild() {
    let tmp = TempDir::new().unwrap();
    let (kb, db, _) = built(tmp.path(), false);
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute("DELETE FROM meta WHERE key = 'built_at'", [])
        .unwrap();
    let err = index::status(&db, &kb).unwrap_err();
    assert_eq!(err.to_string(), "index has no built_at \u{2014} rebuild it");
}

#[test]
fn status_without_an_index_names_the_missing_db() {
    let tmp = TempDir::new().unwrap();
    let kb_root = fixture(tmp.path(), false);
    let kb = load(&kb_root);
    let err = index::status(&db_path(&kb_root), &kb)
        .unwrap_err()
        .to_string();
    assert!(err.starts_with("no index at "), "got {err:?}");
}

// ------------------------------------------------------------------ rebuild

#[test]
fn rebuild_replaces_the_previous_database_and_wal_siblings() {
    let tmp = TempDir::new().unwrap();
    let (_kb, db, _) = built(tmp.path(), false);
    let count = index::query(&db, "SELECT count(*) FROM doc").unwrap();
    assert_eq!(single_column(&count), vec![Some("3".to_string())]);

    // Garbage WAL siblings from a previous life must not poison the rebuild.
    fs::write(db.with_file_name("index.db-wal"), b"garbage").unwrap();
    fs::write(db.with_file_name("index.db-shm"), b"garbage").unwrap();

    let kb_root = tmp.path().join("kb");
    fs::remove_file(kb_root.join("bundles").join("demo").join("beta.md")).unwrap();
    let kb2 = load(&kb_root);
    let stats = index::build(&kb2, &db, Utc::now()).unwrap();
    assert_eq!(stats.docs, 2);
    let count = index::query(&db, "SELECT count(*) FROM doc").unwrap();
    assert_eq!(single_column(&count), vec![Some("2".to_string())]);
    let beta = index::query(&db, "SELECT id FROM doc WHERE bundle_path = '/beta.md'").unwrap();
    assert!(beta.rows.is_empty(), "the old row is gone, not resurrected");
}

// ---------------------------------------------------------------- rendering

#[test]
fn render_rows_json_matches_the_scala_shape() {
    let rows = Rows {
        columns: vec!["a".to_string(), "b".to_string()],
        rows: vec![vec![Some("x".to_string()), None]],
    };
    let expected = "{\n  \"columns\": [\n    \"a\",\n    \"b\"\n  ],\n  \"rowCount\": 1,\n  \"rows\": [\n    {\n      \"a\": \"x\",\n      \"b\": null\n    }\n  ]\n}\n";
    assert_eq!(index::render_rows(&rows, true), expected);
    let empty = Rows {
        columns: vec!["a".to_string()],
        rows: vec![],
    };
    let expected_empty =
        "{\n  \"columns\": [\n    \"a\"\n  ],\n  \"rowCount\": 0,\n  \"rows\": []\n}\n";
    assert_eq!(index::render_rows(&empty, true), expected_empty);
}

#[test]
fn render_rows_text_aligns_and_counts() {
    let rows = Rows {
        columns: vec!["name".to_string(), "v".to_string()],
        rows: vec![
            vec![Some("x".to_string()), Some("1".to_string())],
            vec![Some("longer".to_string()), None],
        ],
    };
    assert_eq!(
        index::render_rows(&rows, false),
        "name    v\n------  -\nx       1\nlonger\n\n2 row(s)\n"
    );
    let empty = Rows {
        columns: vec!["a".to_string()],
        rows: vec![],
    };
    assert_eq!(index::render_rows(&empty, false), "no rows\n");
}

#[test]
fn query_cannot_write_through_a_pragma_the_token_guard_admits() {
    // The first-token guard lets every PRAGMA past, so the connection itself
    // has to be read-only.
    let tmp = TempDir::new().unwrap();
    let (_kb, db, _) = built(tmp.path(), false);
    let before = single_column(&index::query(&db, "PRAGMA user_version").unwrap());
    let journal = single_column(&index::query(&db, "PRAGMA journal_mode").unwrap());
    for sql in [
        "PRAGMA user_version = 7",
        "PRAGMA application_id = 1234",
        "PRAGMA journal_mode = DELETE",
    ] {
        // Refused or inert, but never effective.
        let _ = index::query(&db, sql);
    }
    assert_eq!(
        single_column(&index::query(&db, "PRAGMA user_version").unwrap()),
        before,
        "user_version must not move"
    );
    assert_eq!(
        single_column(&index::query(&db, "PRAGMA application_id").unwrap()),
        vec![Some("0".to_string())],
        "application_id must not move"
    );
    assert_eq!(
        single_column(&index::query(&db, "PRAGMA journal_mode").unwrap()),
        journal,
        "journal_mode must not move"
    );
}

#[test]
fn query_cannot_mutate_through_a_with_prefixed_statement() {
    // `WITH` is admitted by the token guard, but a CTE may prefix a DELETE or
    // an UPDATE.
    let tmp = TempDir::new().unwrap();
    let (_kb, db, _) = built(tmp.path(), false);
    let links = single_column(&index::query(&db, "SELECT count(*) FROM link").unwrap());
    let titles = single_column(&index::query(&db, "SELECT count(*) FROM doc").unwrap());
    assert_ne!(links, vec![Some("0".to_string())], "fixture has links");

    let err = index::query(
        &db,
        "WITH victims AS (SELECT id FROM link) DELETE FROM link WHERE id IN (SELECT id FROM victims) RETURNING id",
    )
    .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("readonly"),
        "got: {err}"
    );
    let err = index::query(
        &db,
        "WITH x AS (SELECT 1) UPDATE doc SET title = 'pwned' RETURNING title",
    )
    .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("readonly"),
        "got: {err}"
    );

    assert_eq!(
        single_column(&index::query(&db, "SELECT count(*) FROM link").unwrap()),
        links,
        "no link was deleted"
    );
    assert_eq!(
        single_column(&index::query(&db, "SELECT count(*) FROM doc").unwrap()),
        titles
    );
    assert_eq!(
        single_column(
            &index::query(&db, "SELECT count(*) FROM doc WHERE title = 'pwned'").unwrap()
        ),
        vec![Some("0".to_string())],
        "no title was rewritten"
    );
}
