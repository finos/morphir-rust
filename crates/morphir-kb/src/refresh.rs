//! Bringing derived state back in line with the markdown. Ported from
//! `KbRefresh.scala`, plus the orchestration from `KbCli.runRefresh` in
//! `kb.scala`.
//!
//! Two kinds of derived state exist in this knowledge base:
//!
//!   - the **markdown indexes**, whose bullets mirror each concept's
//!     `description`
//!   - the **SQLite index**, which is recomputed from the files
//!
//! `refresh` reconciles both. It only makes changes that are mechanical —
//! rewriting a bullet to match the description it is supposed to mirror, or
//! rebuilding a stale database. Adding a missing entry means choosing which
//! section it belongs under, which is a judgement call, so that is opt-in.
//!
//! Unlike the Scala original, which reads the clock inside the intent-index
//! regeneration and the DB build, `today` and `built_at` flow in as
//! parameters so the library stays deterministic; the CLI supplies the clock.

use std::fs;
use std::path::Path;

use chrono::{DateTime, NaiveDate, Utc};

use morphir_okf::model::{Bundle, Doc, Kb, parse_index_entry};
use morphir_okf::paths;
use morphir_okf::store;

use crate::error::{Error, Result};
use crate::render::JVal;
use crate::{index, intent, scaffold};

/// What a refresh action did (or would do, under `--dry-run`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefreshKind {
    DescriptionFixed,
    EntryAdded,
    EntryMissing,
    IntentIndexRebuilt,
    IndexRebuilt,
    IndexFresh,
}

impl RefreshKind {
    /// The enum-variant name, as the Scala `kind.toString` renders in JSON.
    pub fn as_str(&self) -> &'static str {
        match self {
            RefreshKind::DescriptionFixed => "DescriptionFixed",
            RefreshKind::EntryAdded => "EntryAdded",
            RefreshKind::EntryMissing => "EntryMissing",
            RefreshKind::IntentIndexRebuilt => "IntentIndexRebuilt",
            RefreshKind::IndexRebuilt => "IndexRebuilt",
            RefreshKind::IndexFresh => "IndexFresh",
        }
    }
}

/// One reported action: what happened, to which file, and the detail line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshAction {
    pub kind: RefreshKind,
    pub file: String,
    pub detail: String,
}

fn normalize(s: &str) -> String {
    let trimmed = s.trim();
    let stripped = trimmed.strip_suffix('.').unwrap_or(trimmed);
    stripped
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

// ---------------------------------------------------------------- markdown

/// Rewrites index bullets whose text has drifted from the concept's
/// `description`, reports (and with `add_missing` appends) unindexed
/// concepts, and regenerates the intent bundle's generated index.
///
/// Only the trailing description of a bullet is touched; the link itself is
/// preserved verbatim, so a hand-written link title survives a refresh.
pub fn refresh_markdown(
    kb: &Kb,
    add_missing: bool,
    section: &str,
    dry_run: bool,
    today: NaiveDate,
) -> Result<Vec<RefreshAction>> {
    let mut out = Vec::new();
    for b in &kb.bundles {
        out.extend(refresh_bundle(kb, b, add_missing, section, dry_run)?);
    }
    out.extend(refresh_intent_index(kb, dry_run, today)?);
    Ok(out)
}

/// The intent bundle's index is generated, not hand-written — that is what
/// stops it drifting from the records.
fn refresh_intent_index(kb: &Kb, dry_run: bool, today: NaiveDate) -> Result<Vec<RefreshAction>> {
    let Some(b) = intent::find_bundle(kb) else {
        return Ok(Vec::new());
    };
    let action = RefreshAction {
        kind: RefreshKind::IntentIndexRebuilt,
        file: kb.rel(&b.index.file),
        detail: format!("{} intent grouped by state", intent::intents(b).len()),
    };
    if dry_run {
        return Ok(vec![action]);
    }
    if intent::generate_index(b, today)? {
        Ok(vec![action])
    } else {
        Ok(Vec::new())
    }
}

fn refresh_bundle(
    kb: &Kb,
    b: &Bundle,
    add_missing: bool,
    section: &str,
    dry_run: bool,
) -> Result<Vec<RefreshAction>> {
    // The intent bundle's index is regenerated wholesale elsewhere, so
    // per-entry repair and coverage reporting would be both redundant and
    // wrong here — they run against the pre-regeneration state.
    if intent::find_bundle(kb).is_some_and(|ib| ib.label() == b.label()) {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for idx in b.all_indexes() {
        out.extend(fix_index(kb, b, idx, dry_run)?);
    }
    out.extend(add_missing_entries(kb, b, add_missing, section, dry_run)?);
    Ok(out)
}

fn fix_index(kb: &Kb, b: &Bundle, idx: &Doc, dry_run: bool) -> Result<Vec<RefreshAction>> {
    let text = fs::read_to_string(&idx.file)?.replace("\r\n", "\n");
    let mut actions = Vec::new();
    let updated: Vec<String> = text
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let Some(entry) = parse_index_entry(line) else {
                return line.to_string();
            };
            if !entry.dest.starts_with('/') {
                return line.to_string();
            }
            let current = entry
                .description
                .as_deref()
                .unwrap_or("")
                .trim()
                .to_string();
            let want = b
                .concept_at(entry.dest.split('#').next().unwrap_or(""))
                .and_then(|c| c.fm().description())
                .map(|d| d.trim().to_string());
            match want {
                Some(want) if normalize(&current) != normalize(&want) => {
                    let was = if current.is_empty() {
                        "(no description)".to_string()
                    } else {
                        format!("\"{current}\"")
                    };
                    actions.push(RefreshAction {
                        kind: RefreshKind::DescriptionFixed,
                        file: format!("{}:{}", kb.rel(&idx.file), i + 1),
                        detail: format!("{} — was {was}", entry.dest),
                    });
                    format!("{} - {want}", entry.link)
                }
                _ => line.to_string(),
            }
        })
        .collect();
    if !actions.is_empty() && !dry_run {
        fs::write(&idx.file, format!("{}\n", updated.join("\n").trim_end()))?;
    }
    Ok(actions)
}

/// Concepts no index links to. Reported always; appended only when
/// `add_missing` is set.
fn add_missing_entries(
    kb: &Kb,
    b: &Bundle,
    add_missing: bool,
    section: &str,
    dry_run: bool,
) -> Result<Vec<RefreshAction>> {
    let linked: std::collections::HashSet<String> = b
        .all_indexes()
        .iter()
        .flat_map(|idx| {
            idx.links
                .iter()
                .filter(|l| l.is_bundle_relative())
                .map(|l| l.dest.split('#').next().unwrap_or("").to_string())
        })
        .collect();
    let missing: Vec<&Doc> = b
        .concepts
        .iter()
        .filter(|c| !linked.contains(&c.bundle_path()))
        .collect();
    if missing.is_empty() {
        return Ok(Vec::new());
    }
    if !add_missing {
        return Ok(missing
            .into_iter()
            .map(|c| RefreshAction {
                kind: RefreshKind::EntryMissing,
                file: kb.rel(&c.file),
                detail: format!(
                    "not linked from any index in {} — pass --add-missing to append it",
                    b.label()
                ),
            })
            .collect());
    }
    let mut out = Vec::new();
    for c in missing {
        // Subdirectory concepts belong in that subdirectory's index when one
        // exists.
        let idx_doc = b
            .sub_indexes
            .iter()
            .find(|i| c.rel.len() > 1 && i.rel[..i.rel.len() - 1] == c.rel[..c.rel.len() - 1])
            .unwrap_or(&b.index);
        let action = RefreshAction {
            kind: RefreshKind::EntryAdded,
            file: kb.rel(&idx_doc.file),
            detail: format!("{} under \"{section}\"", c.bundle_path()),
        };
        if !dry_run {
            scaffold::insert_index_entry(
                &idx_doc.file,
                section,
                &c.display_title(),
                &c.bundle_path(),
                &c.fm().description().unwrap_or_default(),
            )?;
        }
        out.push(action);
    }
    Ok(out)
}

// ---------------------------------------------------------------------- db

/// Rebuilds the SQLite index when it is missing, stale, or `force` is set.
pub fn refresh_db(
    kb: &Kb,
    db: &Path,
    force: bool,
    dry_run: bool,
    built_at: DateTime<Utc>,
) -> Result<Vec<RefreshAction>> {
    let rendered = paths::render(db);
    match index::status(db, kb) {
        // No index yet, or one without usable metadata: build it.
        Err(Error::Msg(_)) => {
            if dry_run {
                Ok(vec![RefreshAction {
                    kind: RefreshKind::IndexRebuilt,
                    file: rendered,
                    detail: "absent".to_string(),
                }])
            } else {
                let s = index::build(kb, db, built_at)?;
                Ok(vec![RefreshAction {
                    kind: RefreshKind::IndexRebuilt,
                    file: rendered,
                    detail: format!("built {} docs", s.docs),
                }])
            }
        }
        Err(e) => Err(e),
        Ok(st) => {
            if st.stale.is_empty() && !force {
                return Ok(vec![RefreshAction {
                    kind: RefreshKind::IndexFresh,
                    file: rendered,
                    detail: format!("up to date (built {})", st.built_at),
                }]);
            }
            let why = if force {
                "forced".to_string()
            } else {
                format!("{} file(s) changed", st.stale.len())
            };
            if dry_run {
                Ok(vec![RefreshAction {
                    kind: RefreshKind::IndexRebuilt,
                    file: rendered,
                    detail: why,
                }])
            } else {
                let s = index::build(kb, db, built_at)?;
                Ok(vec![RefreshAction {
                    kind: RefreshKind::IndexRebuilt,
                    file: rendered,
                    detail: format!("rebuilt {} docs ({why})", s.docs),
                }])
            }
        }
    }
}

// ------------------------------------------------------------ orchestration

/// The one refresh implementation behind `kb refresh`, `kb refresh markdown`
/// and `kb refresh db` — they differ only in which halves they enable.
///
/// Loads the kb from `kb_root`, runs the markdown pass, then **reloads from
/// disk** before the DB pass, so the database is built from what is now on
/// disk rather than from the pre-rewrite state.
#[allow(clippy::too_many_arguments)]
pub fn refresh(
    kb_root: &Path,
    markdown: bool,
    database: bool,
    dry_run: bool,
    force: bool,
    add_missing: bool,
    section: &str,
    db: &Path,
    today: NaiveDate,
    built_at: DateTime<Utc>,
) -> Result<Vec<RefreshAction>> {
    let kb = store::load(kb_root)?;
    let md = if markdown {
        refresh_markdown(&kb, add_missing, section, dry_run, today)?
    } else {
        Vec::new()
    };
    // Reload after rewriting the markdown so the database is built from what
    // is now on disk.
    let db_actions = if database {
        let reloaded = if md.is_empty() || dry_run {
            kb
        } else {
            store::load(kb_root)?
        };
        refresh_db(&reloaded, db, force, dry_run, built_at)?
    } else {
        Vec::new()
    };
    let mut out = md;
    out.extend(db_actions);
    Ok(out)
}

// -------------------------------------------------------------- rendering

/// Renders refresh actions as the Scala CLI does: verb-per-kind text, or the
/// `{dryRun, changed, actions}` JSON shape.
pub fn render(actions: &[RefreshAction], dry_run: bool, json: bool) -> String {
    if json {
        let items: Vec<JVal> = actions
            .iter()
            .map(|a| {
                JVal::Obj(vec![
                    ("kind".to_string(), JVal::str(a.kind.as_str())),
                    ("file".to_string(), JVal::str(&a.file)),
                    ("detail".to_string(), JVal::str(&a.detail)),
                ])
            })
            .collect();
        let changed = actions
            .iter()
            .filter(|a| a.kind != RefreshKind::IndexFresh && a.kind != RefreshKind::EntryMissing)
            .count();
        return JVal::Obj(vec![
            ("dryRun".to_string(), JVal::bool(dry_run)),
            ("changed".to_string(), JVal::num(changed)),
            ("actions".to_string(), JVal::Arr(items)),
        ])
        .document();
    }
    let mut sb = String::new();
    for a in actions {
        let verb = match a.kind {
            RefreshKind::DescriptionFixed => {
                if dry_run {
                    "would fix    "
                } else {
                    "fixed        "
                }
            }
            RefreshKind::EntryAdded => {
                if dry_run {
                    "would add    "
                } else {
                    "added        "
                }
            }
            RefreshKind::EntryMissing => "missing      ",
            RefreshKind::IntentIndexRebuilt => {
                if dry_run {
                    "would regen  "
                } else {
                    "regenerated  "
                }
            }
            RefreshKind::IndexRebuilt => {
                if dry_run {
                    "would rebuild"
                } else {
                    "rebuilt      "
                }
            }
            RefreshKind::IndexFresh => "fresh        ",
        };
        sb.push_str(&format!("{verb} {}\n              {}\n", a.file, a.detail));
    }
    let fixed = actions
        .iter()
        .filter(|a| a.kind == RefreshKind::DescriptionFixed)
        .count();
    let added = actions
        .iter()
        .filter(|a| a.kind == RefreshKind::EntryAdded)
        .count();
    let missing = actions
        .iter()
        .filter(|a| a.kind == RefreshKind::EntryMissing)
        .count();
    if actions.is_empty() {
        sb.push_str("nothing to refresh\n");
    }
    sb.push_str(&format!(
        "\n{fixed} description(s) {}",
        if dry_run { "to fix" } else { "fixed" }
    ));
    if added > 0 {
        sb.push_str(&format!(
            ", {added} entr{} {}",
            if added == 1 { "y" } else { "ies" },
            if dry_run { "to add" } else { "added" }
        ));
    }
    if missing > 0 {
        sb.push_str(&format!(", {missing} unindexed concept(s)"));
    }
    sb.push('\n');
    sb
}
