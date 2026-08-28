//! Intent management — features, enhancements and bugs recorded as prose with
//! a lifecycle. Ported from `KbIntent.scala` and `KbIntentEdit.scala`.
//!
//! The distinction this rests on: an **Intent** is future-tense and has a
//! lifecycle; a **Capability** is present-tense and is simply either true or
//! stale. Releasing is where they meet, which is why marking an Intent
//! Released demands a link to the Capability it produced.
//!
//! Obligations are checked wherever a record currently sits, never against the
//! path it took to get there — work genuinely jumps stages, and a tool that
//! fights that gets worked around.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::NaiveDate;
use morphir_okf::frontmatter::{Frontmatter, split_frontmatter};
use morphir_okf::model::{Bundle, Doc, Finding, Kb, Severity};
use morphir_okf::paths;
use regex::Regex;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::util::{slugify, yaml_str};

/// Everything below this marker in the intent bundle's index is regenerated.
pub const MARKER: &str = "<!-- intent:index -->";

// -------------------------------------------------------------------- states

/// Lifecycle state of an intent record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum IntentState {
    Backlog,
    Refinement,
    InProgress,
    Released,
    Cancelled,
    Superseded,
}

impl IntentState {
    /// Declaration order, as the Scala enum's `values`.
    pub const ALL: [IntentState; 6] = [
        IntentState::Backlog,
        IntentState::Refinement,
        IntentState::InProgress,
        IntentState::Released,
        IntentState::Cancelled,
        IntentState::Superseded,
    ];

    /// Display order: live work first, then terminal records.
    pub const DISPLAY_ORDER: [IntentState; 6] = [
        IntentState::InProgress,
        IntentState::Refinement,
        IntentState::Backlog,
        IntentState::Released,
        IntentState::Superseded,
        IntentState::Cancelled,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            IntentState::Backlog => "Backlog",
            IntentState::Refinement => "Refinement",
            IntentState::InProgress => "InProgress",
            IntentState::Released => "Released",
            IntentState::Cancelled => "Cancelled",
            IntentState::Superseded => "Superseded",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            IntentState::Released | IntentState::Cancelled | IntentState::Superseded
        )
    }

    /// States where nothing moving is a real signal. Backlog is excluded — a
    /// backlog is *meant* to sit.
    pub fn is_active(&self) -> bool {
        matches!(self, IntentState::Refinement | IntentState::InProgress)
    }

    /// Case-insensitive, hyphens stripped — `in-progress` parses.
    pub fn parse(s: &str) -> Option<IntentState> {
        let needle = s.trim().replace('-', "");
        Self::ALL
            .into_iter()
            .find(|st| st.as_str().eq_ignore_ascii_case(&needle))
    }

    /// The state names joined `", "`, for guard hints.
    pub fn names() -> String {
        Self::ALL
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl fmt::Display for IntentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// --------------------------------------------------------------------- kinds

/// What sort of work an intent records. The user-visible tier decides whether
/// releasing owes a capability link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IntentKind {
    Feature,
    Bug,
    Performance,
    Security,
    Deprecation,
    Removal,
    Refactor,
    Docs,
    Test,
    Build,
    Spike,
}

impl IntentKind {
    pub const ALL: [IntentKind; 11] = [
        IntentKind::Feature,
        IntentKind::Bug,
        IntentKind::Performance,
        IntentKind::Security,
        IntentKind::Deprecation,
        IntentKind::Removal,
        IntentKind::Refactor,
        IntentKind::Docs,
        IntentKind::Test,
        IntentKind::Build,
        IntentKind::Spike,
    ];

    pub fn user_visible(&self) -> bool {
        matches!(
            self,
            IntentKind::Feature
                | IntentKind::Bug
                | IntentKind::Performance
                | IntentKind::Security
                | IntentKind::Deprecation
                | IntentKind::Removal
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            IntentKind::Feature => "feature",
            IntentKind::Bug => "bug",
            IntentKind::Performance => "performance",
            IntentKind::Security => "security",
            IntentKind::Deprecation => "deprecation",
            IntentKind::Removal => "removal",
            IntentKind::Refactor => "refactor",
            IntentKind::Docs => "docs",
            IntentKind::Test => "test",
            IntentKind::Build => "build",
            IntentKind::Spike => "spike",
        }
    }

    pub fn parse(s: &str) -> Option<IntentKind> {
        let needle = s.trim().to_lowercase();
        Self::ALL.into_iter().find(|k| k.label() == needle)
    }

    /// The kind labels joined `", "`, for help text and guard hints.
    pub fn names() -> String {
        Self::ALL
            .iter()
            .map(|k| k.label())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

// -------------------------------------------------------------------- DocRef

static DOC_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^:\s]+):(/[^\s]*)$").expect("valid regex"));

/// A reference to a document elsewhere in the knowledge base:
/// `bundle-label:/path.md`.
///
/// Deliberately not a Package URL — purl identifies registry-backed
/// artifacts, and a Capability is a markdown document no registry knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocRef {
    pub bundle: String,
    pub path: String,
}

impl DocRef {
    pub fn render(&self) -> String {
        format!("{}:{}", self.bundle, self.path)
    }

    pub fn parse(s: &str) -> Option<DocRef> {
        DOC_REF.captures(s.trim()).map(|caps| DocRef {
            bundle: caps[1].to_string(),
            path: caps[2].to_string(),
        })
    }
}

// -------------------------------------------------------------------- config

/// Settings declared in the intent bundle's own index frontmatter, so the
/// tooling is portable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentConfig {
    pub system: Option<String>,
    pub capability_bundle: Option<String>,
    pub stale_after_days: i64,
}

impl IntentConfig {
    pub const DEFAULT_STALE_AFTER_DAYS: i64 = 60;

    pub fn from(fm: &Frontmatter) -> IntentConfig {
        IntentConfig {
            system: fm.str_at("system"),
            capability_bundle: fm.str_at("capability_bundle"),
            stale_after_days: fm
                .str_at("stale_after_days")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(Self::DEFAULT_STALE_AFTER_DAYS),
        }
    }
}

// -------------------------------------------------------------------- record

/// A view over a concept document with `type: Intent`.
#[derive(Debug, Clone, Copy)]
pub struct Intent<'a> {
    pub doc: &'a Doc,
}

impl<'a> Intent<'a> {
    fn fm(&self) -> &Frontmatter {
        self.doc.fm()
    }

    /// Leading digits of the filename, e.g. `0004` from `0004-thing.md`.
    pub fn id(&self) -> String {
        self.doc
            .name()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect()
    }

    pub fn slug(&self) -> String {
        self.doc
            .name()
            .strip_suffix(".md")
            .unwrap_or(self.doc.name())
            .to_string()
    }

    pub fn title(&self) -> String {
        self.doc.display_title()
    }

    pub fn description(&self) -> Option<String> {
        self.fm().description()
    }

    pub fn state(&self) -> Option<IntentState> {
        self.fm()
            .str_at("state")
            .and_then(|s| IntentState::parse(&s))
    }

    pub fn kind(&self) -> Option<IntentKind> {
        self.fm().str_at("kind").and_then(|s| IntentKind::parse(&s))
    }

    pub fn breaking(&self) -> bool {
        self.fm().bool_at("breaking").unwrap_or(false)
    }

    pub fn created(&self) -> Option<NaiveDate> {
        self.date("created")
    }

    pub fn state_since(&self) -> Option<NaiveDate> {
        self.date("state_since")
    }

    pub fn issue(&self) -> Option<String> {
        self.fm().str_at("issue")
    }

    pub fn capability(&self) -> Option<String> {
        self.fm().str_at("capability")
    }

    pub fn superseded_by(&self) -> Option<String> {
        self.fm().str_at("superseded_by")
    }

    pub fn reason(&self) -> Option<String> {
        self.fm().str_at("reason")
    }

    pub fn artifacts(&self) -> Vec<String> {
        self.fm().list_at("artifacts")
    }

    fn date(&self, key: &str) -> Option<NaiveDate> {
        self.fm()
            .str_at(key)
            .and_then(|s| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok())
    }

    pub fn days_since_state_change(&self, today: NaiveDate) -> Option<i64> {
        self.state_since().map(|d| (today - d).num_days())
    }
}

// ----------------------------------------------------------------- discovery

/// The intent bundle is the one whose index frontmatter carries
/// `intent: true`. Nothing is hardcoded, so the tooling works in any
/// repository whatever the bundle is called.
pub fn find_bundle(kb: &Kb) -> Option<&Bundle> {
    kb.bundles
        .iter()
        .find(|b| b.index.fm().bool_at("intent").unwrap_or(false))
}

pub fn config(b: &Bundle) -> IntentConfig {
    IntentConfig::from(b.index.fm())
}

/// Every intent record in the bundle, sorted by slug.
pub fn intents(b: &Bundle) -> Vec<Intent<'_>> {
    let mut out: Vec<Intent<'_>> = b
        .concepts
        .iter()
        .filter(|d| {
            d.fm()
                .doc_type()
                .is_some_and(|t| t.eq_ignore_ascii_case("Intent"))
        })
        .map(|doc| Intent { doc })
        .collect();
    out.sort_by_key(|i| i.slug());
    out
}

/// The next free id: `max(existing numeric ids) + 1`, zero-padded to 4.
pub fn next_id(b: &Bundle) -> String {
    let max = intents(b)
        .iter()
        .filter_map(|i| i.id().parse::<i64>().ok())
        .max()
        .unwrap_or(0);
    format!("{:04}", max + 1)
}

/// Finds an intent by id (`0007` or bare `7`) or slug.
pub fn find<'a>(b: &'a Bundle, id_or_slug: &str) -> Option<Intent<'a>> {
    let needle = id_or_slug.trim();
    let padded = needle.parse::<i64>().ok().map(|n| format!("{n:04}"));
    intents(b).into_iter().find(|i| {
        let id = i.id();
        id == needle || i.slug() == needle || padded.as_deref() == Some(id.as_str())
    })
}

/// `None` when the reference resolves; `Some(message)` when it does not.
pub fn resolve_ref(kb: &Kb, r: &DocRef) -> Option<String> {
    match kb.bundle(&r.bundle) {
        None => Some(format!(
            "no bundle `{}` (known: {})",
            r.bundle,
            kb.bundles
                .iter()
                .map(|b| b.label())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Some(target) => {
            if target.concept_at(&r.path).is_none() {
                Some(format!(
                    "`{}` names no concept in {}",
                    r.render(),
                    target.label()
                ))
            } else {
                None
            }
        }
    }
}

// -------------------------------------------------------------------- check

/// Obligations owed by each state, plus staleness and duplicate ids. Reuses
/// the kb [`Finding`] type so `kb check` and `kb intent check` render
/// identically.
pub fn check(kb: &Kb, b: &Bundle, today: NaiveDate) -> Vec<Finding> {
    let cfg = config(b);
    let all = intents(b);
    let ids: HashSet<String> = all.iter().map(|i| i.id()).collect();
    let mut out = Vec::new();
    for i in &all {
        out.extend(check_one(kb, &cfg, &ids, i, today));
    }
    out.extend(check_duplicates(kb, b, &all));
    out.extend(check_bundle(kb, b, &cfg));
    out
}

fn check_bundle(kb: &Kb, b: &Bundle, cfg: &IntentConfig) -> Vec<Finding> {
    let where_ = kb.rel(&b.index.file);
    let mut out = Vec::new();
    if cfg.system.is_none() {
        out.push(Finding {
            severity: Severity::Warn,
            check: "intent-no-system".to_string(),
            path: where_.clone(),
            line: Some(1),
            message: "intent bundle declares no `system`".to_string(),
            hint: Some(
                "add a Package URL, e.g. system: pkg:maven/org.finos.morphir/morphir-core"
                    .to_string(),
            ),
        });
    }
    if cfg.capability_bundle.is_none() {
        out.push(Finding {
            severity: Severity::Warn,
            check: "intent-no-capability-bundle".to_string(),
            path: where_,
            line: Some(1),
            message: "intent bundle declares no `capability_bundle`".to_string(),
            hint: Some(
                "releasing cannot be checked against a target bundle without it".to_string(),
            ),
        });
    }
    out
}

/// The addition over the Scala tool: two intent files sharing a numeric id
/// prefix is an error (beads morphir-df9b) — `find` by id would silently pick
/// one of them.
fn check_duplicates(kb: &Kb, b: &Bundle, all: &[Intent<'_>]) -> Vec<Finding> {
    let mut by_id: BTreeMap<String, Vec<&Intent<'_>>> = BTreeMap::new();
    for i in all {
        let id = i.id();
        if !id.is_empty() {
            by_id.entry(id).or_default().push(i);
        }
    }
    by_id
        .into_iter()
        .filter(|(_, group)| group.len() > 1)
        .flat_map(|(id, group)| {
            let n = group.len();
            group
                .into_iter()
                .map(|i| Finding {
                    severity: Severity::Error,
                    check: "intent-duplicate-id".to_string(),
                    path: kb.rel(&i.doc.file),
                    line: Some(1),
                    message: format!("intent id `{id}` is used by {n} records in {}", b.label()),
                    hint: Some(
                        "ids come from the filename prefix; renumber one of them".to_string(),
                    ),
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn check_one(
    kb: &Kb,
    cfg: &IntentConfig,
    ids: &HashSet<String>,
    i: &Intent<'_>,
    today: NaiveDate,
) -> Vec<Finding> {
    let where_ = kb.rel(&i.doc.file);
    let err = |check: &str, msg: String, hint: Option<String>| Finding {
        severity: Severity::Error,
        check: check.to_string(),
        path: where_.clone(),
        line: Some(1),
        message: msg,
        hint,
    };
    let warn = |check: &str, msg: String, hint: Option<String>| Finding {
        severity: Severity::Warn,
        check: check.to_string(),
        path: where_.clone(),
        line: Some(1),
        message: msg,
        hint,
    };

    let mut out = Vec::new();

    // ----------------------------------------------------------- shape
    if i.state().is_none() {
        let msg = match i.fm().str_at("state") {
            None => "intent has no `state`".to_string(),
            Some(s) => format!("`state: {s}` is not a known state"),
        };
        out.push(err(
            "intent-state-missing",
            msg,
            Some(format!("one of {}", IntentState::names())),
        ));
    }
    if i.kind().is_none() {
        let msg = match i.fm().str_at("kind") {
            None => "intent has no `kind`".to_string(),
            Some(s) => format!("`kind: {s}` is not a known kind"),
        };
        out.push(err(
            "intent-kind-missing",
            msg,
            Some(format!("one of {}", IntentKind::names())),
        ));
    }
    if i.created().is_none() {
        out.push(err(
            "intent-created-missing",
            "intent has no valid `created` date (YYYY-MM-DD)".to_string(),
            None,
        ));
    }
    if i.state_since().is_none() {
        out.push(err(
            "intent-state-since-missing",
            "intent has no valid `state_since` date (YYYY-MM-DD)".to_string(),
            Some("it is what staleness is measured from".to_string()),
        ));
    }

    // ----------------------------------------------------- obligations
    match i.state() {
        Some(IntentState::Released) => {
            let target = if i.kind() == Some(IntentKind::Spike) {
                "Design Note"
            } else {
                "Capability"
            };
            match i.capability() {
                None if i.kind().is_none_or(|k| k.user_visible()) => {
                    // User-visible work must teach the knowledge base what
                    // changed. Internal work often has nothing to teach it,
                    // and inventing a document for "added three labels" is
                    // the noise this design avoids.
                    out.push(err(
                        "intent-released-no-capability",
                        format!("Released intent does not link to the {target} it produced"),
                        Some("kb intent release <id> --capability bundle:/path.md".to_string()),
                    ));
                }
                None => {
                    out.push(warn(
                        "intent-released-no-capability-internal",
                        format!(
                            "Released {} intent links to no {target}",
                            i.kind().map(|k| k.label()).unwrap_or("")
                        ),
                        Some(
                            "fine when there is nothing for the knowledge base to learn; link one if there is"
                                .to_string(),
                        ),
                    ));
                }
                Some(raw) => match DocRef::parse(&raw) {
                    None => out.push(err(
                        "intent-capability-malformed",
                        format!("`capability: {raw}` is not `bundle-label:/path.md`"),
                        None,
                    )),
                    Some(r) => {
                        if let Some(msg) = resolve_ref(kb, &r) {
                            out.push(err("intent-capability-unresolved", msg, None));
                        }
                    }
                },
            }
        }
        Some(IntentState::Cancelled) => {
            if i.reason().is_none_or(|r| r.trim().is_empty()) {
                out.push(err(
                    "intent-cancelled-no-reason",
                    "Cancelled intent has no `reason`".to_string(),
                    Some("a cancellation without a reason is worthless six months on".to_string()),
                ));
            }
        }
        Some(IntentState::Superseded) => match i.superseded_by() {
            None => out.push(err(
                "intent-superseded-no-successor",
                "Superseded intent has no `superseded_by`".to_string(),
                None,
            )),
            Some(succ) => {
                let trimmed = succ.trim();
                let padded = trimmed.parse::<i64>().ok().map(|n| format!("{n:04}"));
                let known =
                    ids.contains(trimmed) || padded.as_deref().is_some_and(|p| ids.contains(p));
                if !known {
                    out.push(err(
                        "intent-superseded-unknown",
                        format!("`superseded_by: {succ}` names no intent in this bundle"),
                        None,
                    ));
                }
            }
        },
        _ => {}
    }

    // ------------------------------------------------------- staleness
    if let Some(st) = i.state()
        && st.is_active()
        && let Some(days) = i.days_since_state_change(today)
        && days > cfg.stale_after_days
    {
        out.push(warn(
            "intent-stale",
            format!(
                "in {st} for {days} days (threshold {})",
                cfg.stale_after_days
            ),
            Some("move it on, or move it back to Backlog and say so".to_string()),
        ));
    }

    // ------------------------------------------------------- artifacts
    for a in i.artifacts() {
        if !a.trim().starts_with("pkg:") {
            out.push(warn(
                "intent-artifact-not-purl",
                format!("artifact `{a}` is not a Package URL"),
                Some("expected pkg:type/namespace/name@version".to_string()),
            ));
        }
    }

    out
}

// ------------------------------------------------------- frontmatter edits

/// Sets or removes top-level scalar frontmatter keys, leaving everything else
/// byte-for-byte alone.
///
/// Only column-zero `key:` lines are considered, so nested structures
/// (`generated:`, `sources:`) are never touched. A key being added is
/// appended at the very end of the frontmatter lines: anchoring on "the last
/// top-level line" looked tidier but was wrong — when frontmatter ends in a
/// block such as `sources:`, that line *is* the block header, and the new key
/// landed between it and its indented children, corrupting the YAML and the
/// provenance with it.
pub fn set_keys(file: &Path, updates: &[(String, Option<String>)]) -> Result<()> {
    let text = fs::read_to_string(file)?;
    let (raw_fm, body) = split_frontmatter(&text);
    let Some(fm) = raw_fm else {
        return Err(Error::msg(format!(
            "{} has no frontmatter to edit",
            paths::render(file)
        )));
    };
    let mut out: Vec<String> = fm.split('\n').map(str::to_string).collect();
    for (key, value) in updates {
        let prefix = format!("{key}:");
        let idx = out.iter().position(|l| l.starts_with(&prefix));
        match (idx, value) {
            (None, Some(v)) => {
                let at = out
                    .iter()
                    .rposition(|l| !l.trim().is_empty())
                    .map(|i| i + 1)
                    .unwrap_or(0);
                out.insert(at, format!("{key}: {v}"));
            }
            (None, None) => {}
            (Some(i), Some(v)) => out[i] = format!("{key}: {v}"),
            (Some(i), None) => {
                // Drop the key and any indented continuation beneath it.
                let end = out[i + 1..]
                    .iter()
                    .position(|l| !l.is_empty() && !l.starts_with(' '))
                    .map(|n| i + 1 + n)
                    .unwrap_or(out.len());
                out.drain(i..end);
            }
        }
    }
    fs::write(
        file,
        format!("---\n{}\n---\n{body}", out.join("\n").trim_end()),
    )?;
    Ok(())
}

// ------------------------------------------------------------------ creating

/// Creates a new intent record in Backlog, allocating the next id.
#[allow(clippy::too_many_arguments)]
pub fn create(
    b: &Bundle,
    title: &str,
    description: &str,
    kind: IntentKind,
    breaking: bool,
    issue: Option<&str>,
    tags: &[String],
    today: NaiveDate,
) -> Result<PathBuf> {
    let id = next_id(b);
    let slug = slugify(title);
    let file = b.root.join(format!("{id}-{slug}.md"));
    if file.exists() {
        return Err(Error::msg(format!(
            "{} already exists",
            paths::render(&file)
        )));
    }
    let mut sb = String::new();
    sb.push_str("---\n");
    sb.push_str("type: Intent\n");
    sb.push_str(&format!("title: {}\n", yaml_str(title)));
    sb.push_str(&format!("description: {}\n", yaml_str(description)));
    sb.push_str(&format!("state: {}\n", IntentState::Backlog));
    sb.push_str(&format!("kind: {}\n", kind.label()));
    sb.push_str(&format!("breaking: {breaking}\n"));
    sb.push_str(&format!("created: {today}\n"));
    sb.push_str(&format!("state_since: {today}\n"));
    if let Some(x) = issue {
        sb.push_str(&format!("issue: {}\n", x.strip_prefix('#').unwrap_or(x)));
    }
    if !tags.is_empty() {
        let slugged: Vec<String> = tags.iter().map(|t| slugify(t)).collect();
        sb.push_str(&format!("tags: [{}]\n", slugged.join(", ")));
    }
    sb.push_str("---\n\n");
    sb.push_str(&format!("# {id} — {title}\n\n"));
    sb.push_str(&format!("{description}\n\n"));
    sb.push_str(
        "## Problem\n\n<!-- TODO: what problem is this solving? Resist describing a solution here. -->\n\n",
    );
    sb.push_str(
        "## Approach\n\n<!-- TODO: fill in during Refinement. Delete if it stays trivial. -->\n",
    );
    fs::write(&file, sb)?;
    Ok(file)
}

// --------------------------------------------------------------- transitions

/// A requested state change, with the material its obligations may demand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub to: IntentState,
    pub capability: Option<String>,
    pub artifacts: Vec<String>,
    pub reason: Option<String>,
    pub superseded_by: Option<String>,
}

impl Transition {
    /// A bare transition to `to`, with no obligation material.
    pub fn to(to: IntentState) -> Transition {
        Transition {
            to,
            capability: None,
            artifacts: Vec::new(),
            reason: None,
            superseded_by: None,
        }
    }
}

/// Applies a transition, refusing up front when the target state's obligation
/// is unmet.
pub fn transition(
    kb: &Kb,
    b: &Bundle,
    i: &Intent<'_>,
    t: &Transition,
    today: NaiveDate,
) -> Result<PathBuf> {
    let missing = match t.to {
        // Only user-visible kinds owe a capability; internal work (build,
        // refactor, test) often changes nothing a reader of the knowledge
        // base needs to know.
        IntentState::Released
            if t.capability.is_none()
                && i.capability().is_none()
                && i.kind().is_none_or(|k| k.user_visible()) =>
        {
            let target = if i.kind() == Some(IntentKind::Spike) {
                "design note"
            } else {
                "capability"
            };
            Some(format!(
                "releasing needs --capability bundle:/path.md (the {target} this produced)"
            ))
        }
        IntentState::Cancelled if t.reason.is_none() && i.reason().is_none() => {
            Some("cancelling needs --reason".to_string())
        }
        IntentState::Superseded if t.superseded_by.is_none() && i.superseded_by().is_none() => {
            Some("superseding needs --by <intent-id>".to_string())
        }
        _ => None,
    };

    let bad_ref = t
        .capability
        .as_ref()
        .and_then(|raw| match DocRef::parse(raw) {
            None => Some(format!("`{raw}` is not `bundle-label:/path.md`")),
            Some(r) => resolve_ref(kb, &r),
        });

    let bad_successor = t.superseded_by.as_ref().and_then(|s| {
        if find(b, s).is_none() {
            Some(format!("no intent `{s}` in {}", b.label()))
        } else {
            None
        }
    });

    if let Some(msg) = missing.or(bad_ref).or(bad_successor) {
        return Err(Error::msg(msg));
    }

    let mut updates: Vec<(String, Option<String>)> = vec![
        ("state".to_string(), Some(t.to.to_string())),
        ("state_since".to_string(), Some(today.to_string())),
    ];
    if let Some(c) = &t.capability {
        updates.push(("capability".to_string(), Some(c.clone())));
    }
    if let Some(r) = &t.reason {
        updates.push(("reason".to_string(), Some(yaml_str(r))));
    }
    if let Some(s) = &t.superseded_by {
        updates.push(("superseded_by".to_string(), Some(s.clone())));
    }
    if !t.artifacts.is_empty() {
        updates.push((
            "artifacts".to_string(),
            Some(format!("[{}]", t.artifacts.join(", "))),
        ));
    }
    set_keys(&i.doc.file, &updates)?;
    Ok(i.doc.file.clone())
}

// ----------------------------------------------------------- index generation

/// Rewrites the intent bundle's index below the marker, grouped by state.
/// Returns whether anything changed.
///
/// Generated rather than hand-maintained, which is what stops it rotting —
/// and it is the human-readable answer to "what is pending, in flight, and no
/// longer valid".
pub fn generate_index(b: &Bundle, today: NaiveDate) -> Result<bool> {
    let text = fs::read_to_string(&b.index.file)?;
    let normalized = text.replace("\r\n", "\n");
    let preamble = match normalized.find(MARKER) {
        Some(at) => normalized[..at + MARKER.len()].to_string(),
        None => format!("{}\n\n{MARKER}", normalized.trim_end()),
    };
    let generated = render_sections(&intents(b), today);
    let updated = format!("{preamble}\n\n{generated}");
    if updated == normalized {
        Ok(false)
    } else {
        fs::write(&b.index.file, updated)?;
        Ok(true)
    }
}

fn render_sections(items: &[Intent<'_>], today: NaiveDate) -> String {
    let mut sb = String::new();
    sb.push_str(&format!(
        "_Generated by `kb refresh` — do not edit below the marker. Last built {today}._\n"
    ));
    for st in IntentState::DISPLAY_ORDER {
        let mut group: Vec<&Intent<'_>> = items.iter().filter(|i| i.state() == Some(st)).collect();
        group.sort_by_key(|i| i.id());
        if !group.is_empty() {
            sb.push_str(&format!("\n## {} ({})\n\n", heading(st), group.len()));
            // Flags live in the link text, never after the description: the
            // bullet's trailing text must stay verbatim equal to the
            // concept's `description`, which is what `kb check` enforces.
            for i in group {
                sb.push_str(&bullet(i));
            }
        }
    }
    let stateless: Vec<&Intent<'_>> = items.iter().filter(|i| i.state().is_none()).collect();
    if !stateless.is_empty() {
        sb.push_str(&format!("\n## Without a state ({})\n\n", stateless.len()));
        for i in stateless {
            sb.push_str(&bullet(i));
        }
    }
    if items.is_empty() {
        sb.push_str(
            "\nNo intent recorded yet. `kb intent new --title … --description … --kind feature`\n",
        );
    }
    sb
}

fn bullet(i: &Intent<'_>) -> String {
    format!(
        "* [{}]({}) - {}\n",
        link_text(i),
        i.doc.bundle_path(),
        i.description().unwrap_or_default().trim()
    )
}

fn link_text(i: &Intent<'_>) -> String {
    let mut flags: Vec<&str> = Vec::new();
    if i.breaking() {
        flags.push("breaking");
    }
    if let Some(k) = i.kind() {
        flags.push(k.label());
    }
    if flags.is_empty() {
        format!("{} {}", i.id(), i.title())
    } else {
        format!("{} {} — {}", i.id(), i.title(), flags.join(", "))
    }
}

fn heading(st: IntentState) -> &'static str {
    match st {
        IntentState::InProgress => "In progress",
        IntentState::Backlog => "Backlog",
        IntentState::Refinement => "In refinement",
        IntentState::Released => "Released",
        IntentState::Cancelled => "Cancelled",
        IntentState::Superseded => "Superseded",
    }
}

// ---------------------------------------------------------------------- init

/// Scaffolds an intent bundle in a knowledge base that has none. Returns the
/// created files (`index.md`, `log.md`).
pub fn init_bundle(
    kb_root: &Path,
    name: &str,
    system: Option<&str>,
    capability_bundle: Option<&str>,
    stale_after_days: i64,
    today: NaiveDate,
) -> Result<Vec<PathBuf>> {
    let dir = kb_root.join("bundles").join(slugify(name));
    if dir.exists() {
        return Err(Error::msg(format!(
            "{} already exists",
            paths::render(&dir)
        )));
    }
    let index = dir.join("index.md");
    let log = dir.join("log.md");
    let mut sb = String::new();
    sb.push_str("---\n");
    sb.push_str("okf_version: \"0.2\"\n");
    sb.push_str("title: Intent\n");
    sb.push_str(
        "description: \"Work this project means to do, is doing, or has done — with the reasoning behind it.\"\n",
    );
    sb.push_str("intent: true\n");
    if let Some(s) = system {
        sb.push_str(&format!("system: {s}\n"));
    }
    if let Some(c) = capability_bundle {
        sb.push_str(&format!("capability_bundle: {c}\n"));
    }
    sb.push_str(&format!("stale_after_days: {stale_after_days}\n"));
    sb.push_str("---\n\n");
    sb.push_str("# Intent\n\n");
    sb.push_str(
        "Work this project means to do, is doing, or has done — with the reasoning behind it.\n\n",
    );
    sb.push_str(
        "Each entry is an Intent: future-tense, with a lifecycle. What the system actually *does* today lives\n",
    );
    sb.push_str(
        "in the capability bundle, in the present tense. Releasing an Intent requires linking the Capability it\n",
    );
    sb.push_str("produced, which is what keeps the two in step.\n\n");
    sb.push_str(&format!("{MARKER}\n"));
    fs::create_dir_all(&dir)?;
    fs::write(&index, sb)?;
    fs::write(
        &log,
        format!("# Log\n\n## {today}\n\n* **Creation**: Intent bundle created.\n"),
    )?;
    Ok(vec![index, log])
}

// ----------------------------------------------------------------- rendering

/// JSON shape of one intent, with field names matching the Scala CLI output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntentJson {
    pub id: String,
    pub slug: String,
    pub path: String,
    pub title: String,
    pub description: Option<String>,
    pub state: Option<IntentState>,
    pub kind: Option<IntentKind>,
    #[serde(rename = "userVisible")]
    pub user_visible: Option<bool>,
    pub breaking: bool,
    pub created: Option<String>,
    #[serde(rename = "stateSince")]
    pub state_since: Option<String>,
    pub issue: Option<String>,
    pub capability: Option<String>,
    #[serde(rename = "supersededBy")]
    pub superseded_by: Option<String>,
    pub artifacts: Vec<String>,
}

/// JSON shape of `kb intent list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntentListJson {
    pub bundle: String,
    pub count: usize,
    pub intents: Vec<IntentJson>,
}

pub fn intent_json(i: &Intent<'_>) -> IntentJson {
    IntentJson {
        id: i.id(),
        slug: i.slug(),
        path: i.doc.bundle_path(),
        title: i.title(),
        description: i.description(),
        state: i.state(),
        kind: i.kind(),
        user_visible: i.kind().map(|k| k.user_visible()),
        breaking: i.breaking(),
        created: i.created().map(|d| d.to_string()),
        state_since: i.state_since().map(|d| d.to_string()),
        issue: i.issue(),
        capability: i.capability(),
        superseded_by: i.superseded_by(),
        artifacts: i.artifacts(),
    }
}

fn to_pretty_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("serializable") + "\n"
}

/// Renders `kb intent list`, grouped by state in display order.
pub fn render_list(b: &Bundle, items: &[Intent<'_>], json: bool) -> String {
    if json {
        return to_pretty_json(&IntentListJson {
            bundle: b.label(),
            count: items.len(),
            intents: items.iter().map(intent_json).collect(),
        });
    }
    if items.is_empty() {
        return "no matching intent\n".to_string();
    }
    let mut sb = String::new();
    for st in IntentState::DISPLAY_ORDER {
        let group: Vec<&Intent<'_>> = items.iter().filter(|i| i.state() == Some(st)).collect();
        if !group.is_empty() {
            sb.push_str(&format!(
                "\n{} ({})\n",
                st.as_str().to_uppercase(),
                group.len()
            ));
            for i in group {
                let mut flags: Vec<&str> = Vec::new();
                if i.breaking() {
                    flags.push("breaking");
                }
                if let Some(k) = i.kind() {
                    flags.push(k.label());
                }
                sb.push_str(&format!(
                    "  {:<6} {:<48} {}\n",
                    i.id(),
                    i.title(),
                    flags.join(", ")
                ));
            }
        }
    }
    let orphan: Vec<&Intent<'_>> = items.iter().filter(|i| i.state().is_none()).collect();
    if !orphan.is_empty() {
        sb.push_str(&format!("\nNO STATE ({})\n", orphan.len()));
        for i in orphan {
            sb.push_str(&format!("  {}   {}\n", i.id(), i.title()));
        }
    }
    sb.push_str(&format!("\n{} intent\n", items.len()));
    sb
}

/// Renders `kb intent show`.
pub fn render_show(kb: &Kb, i: &Intent<'_>, json: bool) -> String {
    if json {
        return to_pretty_json(&intent_json(i));
    }
    let mut sb = String::new();
    sb.push_str(&format!("intent {} — {}\n", i.id(), i.title()));
    if let Some(d) = i.description() {
        sb.push_str(&format!("{d}\n"));
    }
    sb.push_str(&format!(
        "\nstate        {}",
        i.state()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "(missing)".to_string())
    ));
    if let Some(d) = i.state_since() {
        sb.push_str(&format!("  since {d}"));
    }
    sb.push('\n');
    sb.push_str(&format!(
        "kind         {}",
        i.kind().map(|k| k.label()).unwrap_or("(missing)")
    ));
    if i.breaking() {
        sb.push_str("  BREAKING");
    }
    sb.push('\n');
    if let Some(d) = i.created() {
        sb.push_str(&format!("created      {d}\n"));
    }
    if let Some(x) = i.issue() {
        sb.push_str(&format!("issue        #{x}\n"));
    }
    if let Some(c) = i.capability() {
        sb.push_str(&format!("capability   {c}\n"));
    }
    if let Some(x) = i.superseded_by() {
        sb.push_str(&format!("superseded   by {x}\n"));
    }
    if let Some(r) = i.reason() {
        sb.push_str(&format!("reason       {r}\n"));
    }
    let artifacts = i.artifacts();
    if !artifacts.is_empty() {
        sb.push_str(&format!(
            "artifacts    {}\n",
            artifacts.join("\n             ")
        ));
    }
    sb.push_str(&format!("file         {}\n", kb.rel(&i.doc.file)));
    sb
}
