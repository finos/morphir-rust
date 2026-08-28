//! A SQLite index over the knowledge base. Ported from `KbIndex.scala`.
//!
//! The index is **derived state**: everything in it is recomputed from the
//! markdown, so it lives under `.dev/` and is never committed. Deleting it
//! costs a rebuild, nothing more.
//!
//! It exists to make three things cheap that are otherwise a full re-parse of
//! every file: full-text search (FTS5), link-graph queries in both directions,
//! and grouping by frontmatter facets — type, tag, status, source repository.
//!
//! Unlike the Scala original, which reads `Instant.now()` inside the build,
//! `built_at` flows in as a parameter so the library stays deterministic; the
//! CLI supplies the clock.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

use chrono::{DateTime, SecondsFormat, Utc};
use regex::Regex;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, OpenFlags, params, params_from_iter};

use morphir_okf::markdown::extract_headings;
use morphir_okf::model::{DocKind, Kb, LinkRef};
use morphir_okf::paths;

use crate::error::{Error, Result};

/// Bumped whenever the schema changes; a mismatch forces a rebuild rather than
/// querying stale shapes.
pub const SCHEMA_VERSION: i32 = 2;

/// Counts reported after a build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexStats {
    pub bundles: usize,
    pub docs: usize,
    pub concepts: usize,
    pub links: usize,
    pub headings: usize,
    pub sources: usize,
    pub tags: usize,
}

/// Result of `kb index --status`: when the index was built, how many documents
/// the kb holds *now*, and which files changed since the build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexStatus {
    pub built_at: String,
    pub docs: usize,
    pub stale: Vec<String>,
}

/// Column-ordered query results; a `None` cell is a SQL NULL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Option<String>>>,
}

/// Must run outside a transaction — SQLite refuses a journal-mode change from
/// within one.
const PRAGMAS: &str = "PRAGMA journal_mode=WAL;\nPRAGMA synchronous=NORMAL;";

const DDL: &[&str] = &[
    "CREATE TABLE meta (
       key   TEXT PRIMARY KEY,
       value TEXT NOT NULL
     )",
    "CREATE TABLE bundle (
       id          INTEGER PRIMARY KEY,
       label       TEXT NOT NULL UNIQUE,
       name        TEXT NOT NULL,
       grp         TEXT,
       okf_version TEXT,
       title       TEXT,
       description TEXT,
       root_path   TEXT NOT NULL
     )",
    "CREATE TABLE doc (
       id                INTEGER PRIMARY KEY,
       bundle_id         INTEGER NOT NULL REFERENCES bundle(id),
       bundle_path       TEXT NOT NULL,
       rel_path          TEXT NOT NULL,
       file_path         TEXT NOT NULL,
       kind              TEXT NOT NULL,
       type              TEXT,
       title             TEXT,
       description       TEXT,
       status            TEXT,
       stale_after       TEXT,
       resource          TEXT,
       has_frontmatter   INTEGER NOT NULL,
       frontmatter_error TEXT,
       body_lines        INTEGER NOT NULL,
       body_chars        INTEGER NOT NULL,
       UNIQUE (bundle_id, rel_path)
     )",
    // Generic key/value over every top-level frontmatter field. Lets other
    // tooling (intent, and whatever comes next) query its own facets without
    // this schema having to learn about them.
    "CREATE TABLE frontmatter (
       doc_id INTEGER NOT NULL REFERENCES doc(id),
       key    TEXT NOT NULL,
       value  TEXT,
       PRIMARY KEY (doc_id, key)
     )",
    "CREATE TABLE tag (
       doc_id INTEGER NOT NULL REFERENCES doc(id),
       tag    TEXT NOT NULL,
       PRIMARY KEY (doc_id, tag)
     )",
    "CREATE TABLE source (
       id         INTEGER PRIMARY KEY,
       doc_id     INTEGER NOT NULL REFERENCES doc(id),
       source_id  TEXT,
       resource   TEXT NOT NULL,
       title      TEXT,
       org        TEXT,
       repo       TEXT,
       commit_sha TEXT,
       src_path   TEXT
     )",
    "CREATE TABLE link (
       id            INTEGER PRIMARY KEY,
       doc_id        INTEGER NOT NULL REFERENCES doc(id),
       dest          TEXT NOT NULL,
       text          TEXT,
       line          INTEGER,
       kind          TEXT NOT NULL,
       target_doc_id INTEGER REFERENCES doc(id)
     )",
    "CREATE TABLE heading (
       id     INTEGER PRIMARY KEY,
       doc_id INTEGER NOT NULL REFERENCES doc(id),
       level  INTEGER NOT NULL,
       text   TEXT NOT NULL,
       slug   TEXT NOT NULL,
       line   INTEGER NOT NULL
     )",
    "CREATE INDEX idx_doc_type ON doc(type)",
    "CREATE INDEX idx_doc_status ON doc(status)",
    "CREATE INDEX idx_doc_kind ON doc(kind)",
    "CREATE INDEX idx_tag_tag ON tag(tag)",
    "CREATE INDEX idx_frontmatter_key ON frontmatter(key, value)",
    "CREATE INDEX idx_link_target ON link(target_doc_id)",
    "CREATE INDEX idx_link_kind ON link(kind)",
    "CREATE INDEX idx_source_repo ON source(org, repo)",
    "CREATE INDEX idx_heading_doc ON heading(doc_id)",
    // Detached content: the KB is small and keeping the text here means FTS
    // answers without touching the doc table.
    "CREATE VIRTUAL TABLE doc_fts USING fts5(bundle_path, title, description, body, tokenize='porter unicode61')",
    "CREATE VIEW v_concept AS
       SELECT d.id, b.label AS bundle, d.bundle_path, d.type, d.title, d.description, d.status, d.stale_after
       FROM doc d JOIN bundle b ON b.id = d.bundle_id
       WHERE d.kind = 'Concept'",
    "CREATE VIEW v_backlink AS
       SELECT l.target_doc_id AS doc_id,
              src.bundle_path AS from_path,
              b.label         AS from_bundle,
              l.line          AS line
       FROM link l
       JOIN doc src ON src.id = l.doc_id
       JOIN bundle b ON b.id = src.bundle_id
       WHERE l.target_doc_id IS NOT NULL",
    // Intent facets pivoted into columns. Depends only on the generic
    // frontmatter table, so kb knows nothing about intent beyond the fact
    // that `type: Intent` documents exist.
    "CREATE VIEW v_intent AS
       SELECT d.id,
              b.label       AS bundle,
              d.bundle_path,
              d.title,
              d.description,
              MAX(CASE WHEN f.key = 'state'         THEN f.value END) AS state,
              MAX(CASE WHEN f.key = 'kind'          THEN f.value END) AS kind,
              MAX(CASE WHEN f.key = 'breaking'      THEN f.value END) AS breaking,
              MAX(CASE WHEN f.key = 'created'       THEN f.value END) AS created,
              MAX(CASE WHEN f.key = 'state_since'   THEN f.value END) AS state_since,
              MAX(CASE WHEN f.key = 'issue'         THEN f.value END) AS issue,
              MAX(CASE WHEN f.key = 'capability'    THEN f.value END) AS capability,
              MAX(CASE WHEN f.key = 'superseded_by' THEN f.value END) AS superseded_by,
              MAX(CASE WHEN f.key = 'artifacts'     THEN f.value END) AS artifacts
       FROM doc d
       JOIN bundle b ON b.id = d.bundle_id
       LEFT JOIN frontmatter f ON f.doc_id = d.id
       WHERE d.type = 'Intent'
       GROUP BY d.id",
    "CREATE VIEW v_orphan AS
       SELECT d.id, b.label AS bundle, d.bundle_path, d.title
       FROM doc d
       JOIN bundle b ON b.id = d.bundle_id
       WHERE d.kind = 'Concept'
         AND NOT EXISTS (SELECT 1 FROM link l WHERE l.target_doc_id = d.id)",
];

static GITHUB_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https://github\.com/([^/]+)/([^/]+)/(?:blob|tree)/([0-9a-f]{7,40})/(.*)$")
        .expect("valid regex")
});

fn link_kind(l: &LinkRef) -> &'static str {
    if l.is_external() {
        "external"
    } else if l.is_anchor_only() {
        "anchor"
    } else if l.is_bundle_relative() {
        "bundle"
    } else {
        "relative"
    }
}

fn kind_str(k: DocKind) -> &'static str {
    match k {
        DocKind::RootIndex => "RootIndex",
        DocKind::SubIndex => "SubIndex",
        DocKind::Log => "Log",
        DocKind::Concept => "Concept",
    }
}

// ------------------------------------------------------------------- build

/// The database file plus the WAL siblings SQLite leaves next to it.
fn siblings(db: &Path) -> Vec<PathBuf> {
    let mut out = vec![db.to_path_buf()];
    for sfx in ["-wal", "-shm"] {
        let mut name = db.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(sfx);
        out.push(db.with_file_name(name));
    }
    out
}

/// Rebuilds the index from scratch. The database file is replaced, so a stale
/// schema can never linger. `built_at` is recorded in `meta` and later drives
/// the staleness comparison in [`status`].
pub fn build(kb: &Kb, db: &Path, built_at: DateTime<Utc>) -> Result<IndexStats> {
    if let Some(parent) = db.parent() {
        fs::create_dir_all(parent)?;
    }
    // WAL leaves siblings behind; a rebuild that kept them would resurrect
    // pages from the old database.
    for p in siblings(db) {
        if p.exists() {
            fs::remove_file(&p)?;
        }
    }
    write_all(kb, db, built_at)
}

fn write_all(kb: &Kb, db: &Path, built_at: DateTime<Utc>) -> Result<IndexStats> {
    let mut conn = Connection::open(db)?;
    conn.execute_batch(PRAGMAS)?;

    let tx = conn.transaction()?;
    for ddl in DDL {
        tx.execute_batch(ddl)?;
    }

    let mut doc_id: i64 = 0;
    let mut links = 0usize;
    let mut heads = 0usize;
    let mut sources = 0usize;
    let mut tags = 0usize;

    {
        let mut meta_st = tx.prepare("INSERT INTO meta(key, value) VALUES (?, ?)")?;
        meta_st.execute(params!["schema_version", SCHEMA_VERSION.to_string()])?;
        meta_st.execute(params!["kb_root", paths::render(&kb.root)])?;
        meta_st.execute(params![
            "built_at",
            built_at.to_rfc3339_opts(SecondsFormat::AutoSi, true)
        ])?;

        let mut bundle_st = tx.prepare(
            "INSERT INTO bundle(id, label, name, grp, okf_version, title, description, root_path) VALUES (?,?,?,?,?,?,?,?)",
        )?;
        let mut doc_st = tx.prepare(
            "INSERT INTO doc(id, bundle_id, bundle_path, rel_path, file_path, kind, type, title, description,
                             status, stale_after, resource, has_frontmatter, frontmatter_error, body_lines, body_chars)
             VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )?;
        let mut fts_st = tx.prepare(
            "INSERT INTO doc_fts(rowid, bundle_path, title, description, body) VALUES (?,?,?,?,?)",
        )?;
        let mut tag_st = tx.prepare("INSERT OR IGNORE INTO tag(doc_id, tag) VALUES (?,?)")?;
        let mut fm_st =
            tx.prepare("INSERT OR IGNORE INTO frontmatter(doc_id, key, value) VALUES (?,?,?)")?;
        let mut src_st = tx.prepare(
            "INSERT INTO source(doc_id, source_id, resource, title, org, repo, commit_sha, src_path) VALUES (?,?,?,?,?,?,?,?)",
        )?;
        let mut head_st =
            tx.prepare("INSERT INTO heading(doc_id, level, text, slug, line) VALUES (?,?,?,?,?)")?;
        let mut link_st =
            tx.prepare("INSERT INTO link(doc_id, dest, text, line, kind) VALUES (?,?,?,?,?)")?;

        for (b_idx, b) in kb.bundles.iter().enumerate() {
            let bundle_id = (b_idx + 1) as i64;
            bundle_st.execute(params![
                bundle_id,
                b.label(),
                b.name,
                b.group,
                b.okf_version(),
                b.index.fm().title(),
                b.index.fm().description(),
                paths::render(&b.root),
            ])?;

            for d in b.all_docs() {
                doc_id += 1;
                let id = doc_id;
                let fm = d.fm();

                doc_st.execute(params![
                    id,
                    bundle_id,
                    d.bundle_path(),
                    d.rel.join("/"),
                    paths::render(&d.file),
                    kind_str(d.kind),
                    fm.doc_type(),
                    d.display_title(),
                    fm.description(),
                    fm.status(),
                    fm.stale_after(),
                    fm.resource(),
                    d.has_frontmatter_block as i64,
                    d.frontmatter_error,
                    d.body.lines().count() as i64,
                    d.body.chars().count() as i64,
                ])?;

                fts_st.execute(params![
                    id,
                    d.bundle_path(),
                    d.display_title(),
                    fm.description().unwrap_or_default(),
                    d.body,
                ])?;

                for t in fm.tags() {
                    tag_st.execute(params![id, t])?;
                    tags += 1;
                }

                for k in fm.keys() {
                    if let Some(text) = fm.get(k).and_then(flatten) {
                        fm_st.execute(params![id, k, text])?;
                    }
                }

                for s in fm.sources() {
                    match GITHUB_URL.captures(&s.resource) {
                        Some(c) => src_st.execute(params![
                            id, s.id, s.resource, s.title, &c[1], &c[2], &c[3], &c[4],
                        ])?,
                        None => src_st.execute(params![
                            id,
                            s.id,
                            s.resource,
                            s.title,
                            Option::<String>::None,
                            Option::<String>::None,
                            Option::<String>::None,
                            Option::<String>::None,
                        ])?,
                    };
                    sources += 1;
                }

                for h in extract_headings(&d.body) {
                    head_st.execute(params![id, h.level as i64, h.text, h.slug, h.line as i64])?;
                    heads += 1;
                }

                for l in &d.links {
                    link_st.execute(params![id, l.dest, l.text, l.line as i64, link_kind(l)])?;
                    links += 1;
                }
            }
        }

        // Second pass: resolve bundle-relative links to their target doc.
        // Only same-bundle destinations resolve, which is exactly what a
        // bundle-relative path means.
        tx.execute(
            "UPDATE link
               SET target_doc_id = (
                 SELECT d.id FROM doc d
                 WHERE d.bundle_id = (SELECT bundle_id FROM doc s WHERE s.id = link.doc_id)
                   AND d.bundle_path = CASE
                     WHEN instr(link.dest, '#') > 0 THEN substr(link.dest, 1, instr(link.dest, '#') - 1)
                     ELSE link.dest
                   END
               )
             WHERE kind = 'bundle'",
            [],
        )?;
    }

    tx.commit()?;
    conn.execute_batch("ANALYZE")?;

    Ok(IndexStats {
        bundles: kb.bundles.len(),
        docs: doc_id as usize,
        concepts: kb.concepts().len(),
        links,
        headings: heads,
        sources,
        tags,
    })
}

/// Renders a frontmatter value as text. Nested mappings are skipped — they
/// have no single sensible scalar form, and the structures that matter
/// (`sources`) already have dedicated tables.
fn flatten(v: &serde_yaml::Value) -> Option<String> {
    use serde_yaml::Value;
    match v {
        Value::Null => None,
        Value::Mapping(_) | Value::Tagged(_) => None,
        Value::Sequence(items) => {
            // As in Scala: a list keeps only its string and number elements.
            let parts: Vec<String> = items
                .iter()
                .filter_map(|x| match x {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    _ => None,
                })
                .collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(", "))
            }
        }
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
    }
}

// ------------------------------------------------------------------- query

/// Runs a read-only query. Anything that is not a SELECT/WITH/PRAGMA/EXPLAIN
/// is refused — this is a query surface, not a way to mutate derived state
/// behind the builder's back. Every failure arm is an [`Error::Msg`] carrying
/// the exact user-facing message the Scala CLI prints.
pub fn query(db: &Path, sql: &str) -> Result<Rows> {
    let trimmed = sql.trim();
    let trimmed = trimmed.strip_suffix(';').unwrap_or(trimmed).trim();
    let head: String = trimmed
        .chars()
        .take_while(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    if !matches!(head.as_str(), "select" | "with" | "pragma" | "explain") {
        return Err(Error::msg(format!(
            "refusing to run `{head}`: kb query is read-only (SELECT, WITH, PRAGMA, EXPLAIN)"
        )));
    }
    present(db)?;
    run_query(db, trimmed, Vec::new()).map_err(|e| Error::msg(e.to_string()))
}

/// The one message every read path owes the caller when there is no database
/// yet — pulled out so [`search`], which builds its own statement, cannot
/// drift from [`query`].
fn present(db: &Path) -> Result<()> {
    if db.exists() {
        Ok(())
    } else {
        Err(Error::msg(format!(
            "no index at {} — run `kb index` first",
            paths::render(db)
        )))
    }
}

fn run_query(db: &Path, sql: &str, binds: Vec<Value>) -> rusqlite::Result<Rows> {
    // SQLite enforces read-only, not the token guard above: a `PRAGMA` that
    // writes, or a CTE prefixing a DELETE/UPDATE, sails straight past a first
    // token of `pragma` or `with`.
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let mut stmt = conn.prepare(sql)?;
    let columns: Vec<String> = stmt.column_names().into_iter().map(String::from).collect();
    let ncols = columns.len();
    let mut out = Vec::new();
    let mut rows = stmt.query(params_from_iter(binds))?;
    while let Some(row) = rows.next()? {
        let mut cells = Vec::with_capacity(ncols);
        for i in 0..ncols {
            cells.push(cell_to_string(row.get_ref(i)?));
        }
        out.push(cells);
    }
    Ok(Rows { columns, rows: out })
}

/// How a SQL value reads as text — the JDBC `getString` semantics the Scala
/// reader relies on: NULL is `None`, everything else its text form.
fn cell_to_string(v: ValueRef<'_>) -> Option<String> {
    match v {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
        ValueRef::Text(t) | ValueRef::Blob(t) => Some(String::from_utf8_lossy(t).into_owned()),
    }
}

/// The facet filters an indexed search narrows by — the same set the scanning
/// search in [`crate::render::search`] accepts, so `--type`, `--tag`,
/// `--status` and `--bundle` mean one thing whichever path serves them.
///
/// The Scala CLI applies these only when scanning; asking the index for
/// `--bundle private` there quietly returns every bundle. Silently dropping a
/// filter the user asked for is worse than not offering it, so this port
/// honours them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SearchFilters<'a> {
    /// Frontmatter `type`, compared without regard to ASCII case.
    pub doc_type: Option<&'a str>,
    /// Every tag listed must be present on the document.
    pub tags: &'a [String],
    /// Frontmatter `status`, compared without regard to ASCII case.
    pub status: Option<&'a str>,
    /// A bundle label (`group/name`) or its bare name, as `Kb::bundle` resolves.
    pub bundle: Option<&'a str>,
}

/// Full-text search over titles, descriptions and bodies, ranked by FTS5's
/// bm25, narrowed by `filters`.
///
/// The column set — `bundle`, `bundle_path`, `type`, `status`, `title`,
/// `description`, `snippet` — and its order are part of the JSON contract
/// [`render_rows`] emits, so filtering may only remove rows, never reshape
/// them.
///
/// Search everything the index holds by passing the default filters:
///
/// ```no_run
/// use morphir_kb::index::{self, SearchFilters};
/// use std::path::Path;
///
/// # fn main() -> morphir_kb::Result<()> {
/// let db = Path::new(".dev/kb/index.db");
/// let hits = index::search(db, "naming", 20, &SearchFilters::default())?;
/// println!("{} row(s)", hits.rows.len());
/// # Ok(())
/// # }
/// ```
///
/// Narrow it by naming the facets. Every tag listed must be present, so this
/// finds stable reference documents tagged both `naming` and `v4`, within one
/// bundle:
///
/// ```no_run
/// use morphir_kb::index::{self, SearchFilters};
/// use std::path::Path;
///
/// # fn main() -> morphir_kb::Result<()> {
/// let tags = vec!["naming".to_string(), "v4".to_string()];
/// let filters = SearchFilters {
///     doc_type: Some("Reference"),
///     tags: &tags,
///     status: Some("stable"),
///     bundle: Some("morphir/morphir-cli"),
/// };
/// let hits = index::search(Path::new(".dev/kb/index.db"), "naming", 20, &filters)?;
/// # let _ = hits;
/// # Ok(())
/// # }
/// ```
pub fn search(db: &Path, needle: &str, limit: usize, filters: &SearchFilters<'_>) -> Result<Rows> {
    present(db)?;
    let (sql, binds) = search_sql(needle, limit, filters);
    // Deliberately not routed through `query`: that entry point takes a bare
    // SQL string and binds nothing, so carrying user-supplied facet values
    // through it would mean interpolating them — an injection surface, and
    // one that would also mangle any value holding a quote or a wildcard.
    // Filters are bound instead, which is why this private path exists.
    run_query(db, &sql, binds).map_err(|e| Error::msg(e.to_string()))
}

/// Builds the statement and its bindings together, so a predicate can never
/// be added without the value it consumes.
fn search_sql(needle: &str, limit: usize, filters: &SearchFilters<'_>) -> (String, Vec<Value>) {
    let mut binds = vec![Value::Text(needle.to_string())];
    let mut wheres = vec!["doc_fts MATCH ?".to_string()];

    // NOCASE is SQLite's ASCII-only case folding, which is precisely what the
    // scanning search's `eq_ignore_ascii_case` does.
    if let Some(t) = filters.doc_type {
        wheres.push("d.type = ? COLLATE NOCASE".to_string());
        binds.push(Value::Text(t.to_string()));
    }
    if let Some(s) = filters.status {
        wheres.push("d.status = ? COLLATE NOCASE".to_string());
        binds.push(Value::Text(s.to_string()));
    }
    // `Kb::bundle` accepts either the full label or the bare name, and matches
    // both exactly; the index stores each in its own column. The subquery is
    // what keeps the answer to one bundle. `label = ? OR name = ?` is
    // set-valued, so a bare name shared by `public/foo` and `private/foo`
    // matched both, while the scanning search — which asks `Kb::bundle` and
    // scopes to the single bundle it returns — matched one: the indexed search
    // handed back documents from a bundle the scan had excluded, which in a
    // public/private split is a disclosure. Bundle ids are handed out in
    // `kb.bundles` order, so the lowest id is the bundle `Kb::bundle` finds
    // first, and the two searches now scope alike.
    //
    // The underlying rule is itself poor: silently picking one of two bundles
    // that both answer to `foo` is not a good answer to an ambiguous name, and
    // an error would be. But that means changing `Kb::bundle`, which both
    // search paths and several other callers go through, so it is a behaviour
    // change of its own and belongs in its own change rather than smuggled in
    // behind a leak fix.
    if let Some(b) = filters.bundle {
        wheres.push(
            "b.id = (SELECT id FROM bundle WHERE label = ? OR name = ? ORDER BY id LIMIT 1)"
                .to_string(),
        );
        binds.push(Value::Text(b.to_string()));
        binds.push(Value::Text(b.to_string()));
    }
    // One EXISTS per tag rather than an IN list: every tag supplied has to be
    // present, and an IN list would settle for any one of them.
    for t in filters.tags {
        wheres.push(
            "EXISTS (SELECT 1 FROM tag WHERE tag.doc_id = d.id AND tag.tag = ? COLLATE NOCASE)"
                .to_string(),
        );
        binds.push(Value::Text(t.clone()));
    }

    binds.push(Value::Integer(limit as i64));
    let predicate = wheres.join("\n               AND ");
    (
        format!(
            "SELECT b.label AS bundle, d.bundle_path, d.type, d.status, d.title, d.description,
                    snippet(doc_fts, 3, '[', ']', '…', 12) AS snippet
             FROM doc_fts
             JOIN doc d ON d.id = doc_fts.rowid
             JOIN bundle b ON b.id = d.bundle_id
             WHERE {predicate}
             ORDER BY bm25(doc_fts, 1.0, 8.0, 4.0, 1.0)
             LIMIT ?"
        ),
        binds,
    )
}

// ------------------------------------------------------------------ status

/// When the index was built, and whether any markdown file has changed since.
///
/// Modification times only speak for files that still exist, so the document
/// count is compared as well: a deleted document leaves no mtime, and without
/// the count check the index would keep serving it as though it were still
/// there.
pub fn status(db: &Path, kb: &Kb) -> Result<IndexStatus> {
    if !db.exists() {
        return Err(Error::msg(format!("no index at {}", paths::render(db))));
    }
    let (built, indexed) = match Connection::open(db) {
        Ok(conn) => {
            let built: Option<String> = conn
                .query_row("SELECT value FROM meta WHERE key = 'built_at'", [], |r| {
                    r.get(0)
                })
                .ok();
            let indexed: Option<i64> = conn
                .query_row("SELECT count(*) FROM doc", [], |r| r.get(0))
                .ok();
            (built, indexed)
        }
        Err(_) => (None, None),
    };

    let built_instant = match &built {
        None => None,
        Some(b) => Some(DateTime::parse_from_rfc3339(b).map_err(|e| Error::msg(e.to_string()))?),
    };
    let mut stale = stale_files(kb, built_instant)?;

    let current: usize = kb.bundles.iter().map(|b| b.all_docs().len()).sum();
    if let Some(n) = indexed {
        let n = n as usize;
        if n != current {
            let verb = if n > current {
                format!("{} document(s) removed since", n - current)
            } else {
                format!("{} document(s) added since", current - n)
            };
            stale.push(format!("({verb} the build)"));
        }
    }

    match built {
        None => Err(Error::msg("index has no built_at — rebuild it")),
        Some(b) => Ok(IndexStatus {
            built_at: b,
            docs: current,
            stale,
        }),
    }
}

fn stale_files(kb: &Kb, built_at: Option<DateTime<chrono::FixedOffset>>) -> Result<Vec<String>> {
    let Some(at) = built_at else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for b in &kb.bundles {
        for d in b.all_docs() {
            let modified = fs::metadata(&d.file)?.modified()?;
            // Millisecond mtimes, as in the Scala original's lastModifiedMs.
            let ms = modified
                .duration_since(UNIX_EPOCH)
                .map(|dur| dur.as_millis() as i64)
                .unwrap_or(0);
            let mtime = DateTime::from_timestamp_millis(ms).unwrap_or_default();
            if mtime > at {
                out.push(kb.rel(&d.file));
            }
        }
    }
    out.sort();
    Ok(out)
}

// --------------------------------------------------------------- rendering

/// JSON string encoding shared by the renderers; matches ujson's default
/// (no unicode escaping).
fn json_str(s: &str) -> String {
    serde_json::to_string(s).expect("a string always serializes")
}

/// An indented JSON array over pre-rendered items, in ujson's indent=2 style:
/// one item per line, `[]` when empty.
fn json_array(items: &[String], indent: usize) -> String {
    if items.is_empty() {
        return "[]".to_string();
    }
    let inner = " ".repeat(indent + 2);
    let outer = " ".repeat(indent);
    let body = items
        .iter()
        .map(|i| format!("{inner}{i}"))
        .collect::<Vec<_>>()
        .join(",\n");
    format!("[\n{body}\n{outer}]")
}

/// A row as a column-name-keyed JSON object, in ujson's indent=2 style.
fn json_row_obj(columns: &[String], row: &[Option<String>], indent: usize) -> String {
    if columns.is_empty() {
        return "{}".to_string();
    }
    let inner = " ".repeat(indent + 2);
    let outer = " ".repeat(indent);
    let body = columns
        .iter()
        .zip(row)
        .map(|(c, v)| {
            let val = v.as_deref().map(json_str).unwrap_or_else(|| "null".into());
            format!("{inner}{}: {val}", json_str(c))
        })
        .collect::<Vec<_>>()
        .join(",\n");
    format!("{{\n{body}\n{outer}}}")
}

/// Renders query results as the Scala CLI does: a `{columns, rowCount, rows}`
/// JSON object, or an aligned text table.
pub fn render_rows(r: &Rows, json: bool) -> String {
    if json {
        let cols: Vec<String> = r.columns.iter().map(|c| json_str(c)).collect();
        let row_objs: Vec<String> = r
            .rows
            .iter()
            .map(|row| json_row_obj(&r.columns, row, 4))
            .collect();
        format!(
            "{{\n  \"columns\": {},\n  \"rowCount\": {},\n  \"rows\": {}\n}}\n",
            json_array(&cols, 2),
            r.rows.len(),
            json_array(&row_objs, 2),
        )
    } else if r.rows.is_empty() {
        "no rows\n".to_string()
    } else {
        let widths: Vec<usize> = (0..r.columns.len())
            .map(|i| {
                let col = r.columns[i].chars().count();
                let cells = r
                    .rows
                    .iter()
                    .map(|row| row[i].as_deref().unwrap_or("").chars().count());
                col.max(cells.max().unwrap_or(0)).min(70)
            })
            .collect();
        let fmt = |cells: &[String]| -> String {
            cells
                .iter()
                .zip(&widths)
                .map(|(c, &w)| {
                    let len = c.chars().count();
                    let cell = if len > w {
                        let head: String = c.chars().take(w - 1).collect();
                        format!("{head}…")
                    } else {
                        c.clone()
                    };
                    let pad = w.saturating_sub(cell.chars().count());
                    format!("{cell}{}", " ".repeat(pad))
                })
                .collect::<Vec<_>>()
                .join("  ")
                .trim_end()
                .to_string()
        };
        let mut sb = String::new();
        sb.push_str(&fmt(&r.columns));
        sb.push('\n');
        sb.push_str(
            &widths
                .iter()
                .map(|w| "-".repeat(*w))
                .collect::<Vec<_>>()
                .join("  "),
        );
        sb.push('\n');
        for row in &r.rows {
            let cells: Vec<String> = row.iter().map(|c| c.clone().unwrap_or_default()).collect();
            sb.push_str(&fmt(&cells));
            sb.push('\n');
        }
        sb.push_str(&format!("\n{} row(s)\n", r.rows.len()));
        sb
    }
}

/// Renders build stats as the Scala CLI does.
pub fn render_stats(s: &IndexStats, db: &Path, json: bool) -> String {
    if json {
        format!(
            "{{\n  \"db\": {},\n  \"schemaVersion\": {SCHEMA_VERSION},\n  \"bundles\": {},\n  \"docs\": {},\n  \"concepts\": {},\n  \"links\": {},\n  \"headings\": {},\n  \"sources\": {},\n  \"tags\": {}\n}}\n",
            json_str(&paths::render(db)),
            s.bundles,
            s.docs,
            s.concepts,
            s.links,
            s.headings,
            s.sources,
            s.tags,
        )
    } else {
        format!(
            "built {} (schema v{SCHEMA_VERSION})\n  {} bundles, {} docs ({} concepts)\n  {} links, {} headings, {} sources, {} tags\n",
            paths::render(db),
            s.bundles,
            s.docs,
            s.concepts,
            s.links,
            s.headings,
            s.sources,
            s.tags,
        )
    }
}

/// Renders `kb index --status` output as the Scala CLI does:
/// `{db, builtAt, docs, staleCount, stale}` in JSON, or the text summary.
pub fn render_status(st: &IndexStatus, db: &Path, json: bool) -> String {
    if json {
        let stale: Vec<String> = st.stale.iter().map(|s| json_str(s)).collect();
        format!(
            "{{\n  \"db\": {},\n  \"builtAt\": {},\n  \"docs\": {},\n  \"staleCount\": {},\n  \"stale\": {}\n}}\n",
            json_str(&paths::render(db)),
            json_str(&st.built_at),
            st.docs,
            st.stale.len(),
            json_array(&stale, 2),
        )
    } else {
        let head = format!(
            "index {}\n  built {} over {} doc(s)\n",
            paths::render(db),
            st.built_at,
            st.docs
        );
        let tail = if st.stale.is_empty() {
            "  up to date\n".to_string()
        } else {
            let mut t = format!(
                "  {} file(s) changed since — rerun `kb index`\n",
                st.stale.len()
            );
            for s in &st.stale {
                t.push_str(&format!("    {s}\n"));
            }
            t
        };
        format!("{head}{tail}")
    }
}
