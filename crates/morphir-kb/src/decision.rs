//! Decision Records — architectural decisions recorded as prose, with
//! supersession rather than revision. Ported from `KbDecision.scala`.
//!
//! The distinction this rests on, and why it is a third register alongside
//! the two the knowledge base already had:
//!
//!   - An **Intent** is future-tense and has a lifecycle. It answers *should
//!     we do this*.
//!   - A **Capability** is present-tense and has no lifecycle. It answers
//!     *what does the system do*.
//!   - A **Decision Record** is past-tense and immutable. It answers *why is
//!     it shaped this way* — including which alternatives were rejected and
//!     under what condition the decision should be revisited.
//!
//! The immutability is the point: once made, a record is superseded by a
//! later one rather than edited, so the reasoning available at the time
//! survives even after the conclusion changes. That is why
//! `supersedes`/`superseded_by` are modelled here and checked for mutual
//! consistency.
//!
//! Records are found by `type: Decision Record` wherever they sit — no bundle
//! marker, no configuration. Ids come from the filename prefix, matching how
//! intent ids work, so the id and the file can never disagree.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::LazyLock;

use chrono::NaiveDate;
use morphir_okf::frontmatter::Frontmatter;
use morphir_okf::model::{Doc, Finding, Kb, Severity};
use morphir_okf::profile::OkfProfile;
use serde::Serialize;

// -------------------------------------------------------------------- states

/// Lifecycle state of a decision record. `status` is OKF maturity and is
/// unrelated to this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum DecisionState {
    Proposed,
    Accepted,
    Superseded,
    Withdrawn,
}

impl DecisionState {
    pub const ALL: [DecisionState; 4] = [
        DecisionState::Proposed,
        DecisionState::Accepted,
        DecisionState::Superseded,
        DecisionState::Withdrawn,
    ];

    /// Display order: what governs now, then what used to.
    pub const DISPLAY_ORDER: [DecisionState; 4] = [
        DecisionState::Accepted,
        DecisionState::Proposed,
        DecisionState::Superseded,
        DecisionState::Withdrawn,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            DecisionState::Proposed => "Proposed",
            DecisionState::Accepted => "Accepted",
            DecisionState::Superseded => "Superseded",
            DecisionState::Withdrawn => "Withdrawn",
        }
    }

    /// A decision that no longer governs. Readers should follow it to
    /// whatever replaced it.
    pub fn is_retired(&self) -> bool {
        matches!(self, DecisionState::Superseded | DecisionState::Withdrawn)
    }

    pub fn parse(s: &str) -> Option<DecisionState> {
        let needle = s.trim();
        Self::ALL
            .into_iter()
            .find(|st| st.as_str().eq_ignore_ascii_case(needle))
    }

    /// The state names joined `", "`, for hints.
    pub fn names() -> String {
        Self::ALL
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl fmt::Display for DecisionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// -------------------------------------------------------------------- record

/// The `type:` value that marks a concept as a decision record. Ownership of
/// the string lives in [`OkfProfile::register_owned_types`], because the
/// vendoring engine has to know the same set to keep a manifest from
/// injecting it into somebody else's document.
pub const DECISION_TYPE: &str = "Decision Record";

static PROFILE: LazyLock<OkfProfile> = LazyLock::new(OkfProfile::default);

/// A view over a concept document with `type: Decision Record`.
#[derive(Debug, Clone)]
pub struct Decision<'a> {
    pub doc: &'a Doc,
    /// Label of the bundle the record sits in.
    pub bundle: String,
}

impl<'a> Decision<'a> {
    fn fm(&self) -> &Frontmatter {
        self.doc.fm()
    }

    /// Leading digits of the filename, e.g. `0004` from
    /// `0004-bridge-nothing.md`. Empty when there are none.
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

    pub fn state(&self) -> Option<DecisionState> {
        self.fm()
            .str_at("state")
            .and_then(|s| DecisionState::parse(&s))
    }

    pub fn decided(&self) -> Option<NaiveDate> {
        self.fm()
            .str_at("decided")
            .and_then(|s| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok())
    }

    pub fn supersedes(&self) -> Vec<String> {
        self.fm()
            .list_at("supersedes")
            .iter()
            .map(|s| normalize_id(s))
            .collect()
    }

    pub fn superseded_by(&self) -> Option<String> {
        self.fm().str_at("superseded_by").map(|s| normalize_id(&s))
    }

    pub fn reason(&self) -> Option<String> {
        self.fm().str_at("reason")
    }

    pub fn tags(&self) -> Vec<String> {
        self.fm().tags()
    }
}

/// Accepts `4`, `0004` and `0004-some-slug` alike, so a human writing
/// `supersedes: [2]` is not punished. A numeric prefix too large for an id
/// falls back to the trimmed input rather than panicking (the Scala tool
/// would throw).
fn normalize_id(raw: &str) -> String {
    let trimmed = raw.trim();
    let head: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    if head.is_empty() {
        trimmed.to_string()
    } else {
        head.parse::<i64>()
            .map(|n| format!("{n:04}"))
            .unwrap_or_else(|_| trimmed.to_string())
    }
}

// ----------------------------------------------------------------- discovery

/// Every decision record in the knowledge base, sorted by
/// (bundle, id, slug). Vendored records are included — they are decision
/// records — but the checks leave them alone.
pub fn decisions(kb: &Kb) -> Vec<Decision<'_>> {
    let mut out: Vec<Decision<'_>> = kb
        .concepts()
        .into_iter()
        .filter(|(_, d)| d.fm().doc_type().is_some_and(|t| PROFILE.owns_type(&t)))
        .map(|(b, d)| Decision {
            doc: d,
            bundle: b.label(),
        })
        .collect();
    out.sort_by_key(|d| (d.bundle.clone(), d.id(), d.slug()));
    out
}

/// The records within one bundle label.
pub fn decisions_in<'a>(kb: &'a Kb, bundle: &str) -> Vec<Decision<'a>> {
    decisions(kb)
        .into_iter()
        .filter(|d| d.bundle == bundle)
        .collect()
}

/// Every record an id or slug could mean, optionally narrowed to one bundle.
///
/// Plural rather than `Option` on purpose. Ids are unique per bundle, not
/// across the knowledge base, so `0001` genuinely names several records once
/// there is more than one bundle with decisions in it. Returning the first in
/// sort order would show an unrelated decision with no hint that a choice had
/// been made; callers are expected to reject an ambiguous answer instead.
pub fn find_all<'a>(kb: &'a Kb, id: &str, bundle: Option<&str>) -> Vec<Decision<'a>> {
    let wanted = id.trim();
    let padded: Option<String> = wanted
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse::<i64>()
        .ok()
        .map(|n| format!("{n:04}"));
    decisions(kb)
        .into_iter()
        .filter(|d| bundle.is_none_or(|b| d.bundle == b || d.bundle.ends_with(&format!("/{b}"))))
        .filter(|d| {
            d.slug() == wanted || d.id() == wanted || padded.as_deref() == Some(d.id().as_str())
        })
        .collect()
}

/// The unambiguous match, or `None` when there is none — or when there is
/// more than one.
pub fn find<'a>(kb: &'a Kb, id: &str, bundle: Option<&str>) -> Option<Decision<'a>> {
    let mut matches = find_all(kb, id, bundle);
    if matches.len() == 1 {
        matches.pop()
    } else {
        None
    }
}

/// The CLI error for an id that names a record in more than one bundle,
/// listing `<bundle>  <slug>` candidates. Picking one would be silent and
/// wrong; say which bundles have it and let the caller name one.
pub fn ambiguous_message(id: &str, matches: &[Decision<'_>]) -> String {
    let candidates: Vec<String> = matches
        .iter()
        .map(|d| format!("  {}  {}", d.bundle, d.slug()))
        .collect();
    format!(
        "`{id}` names a decision record in {} bundles — pass --bundle to choose:\n{}",
        matches.len(),
        candidates.join("\n")
    )
}

// -------------------------------------------------------------------- checks

/// Every finding a decision record can produce. Grouped per bundle, because
/// ids are unique within a bundle rather than across the whole knowledge
/// base — two bundles may each number their decisions from 0001.
pub fn findings(kb: &Kb) -> Vec<Finding> {
    // Mirrored records are upstream's, and so is their schema: an ADR
    // imported from finos/morphir carries its status and date in the body
    // under upstream's own conventions, and its filename is `ADR-0001-…`
    // rather than `0001-…`. Demanding this register's shape of it would
    // report four errors per file that nobody here can fix. They stay
    // listable — `decisions` still finds them — because they are decision
    // records; they are just not ours to check.
    let all: Vec<Decision<'_>> = decisions(kb)
        .into_iter()
        .filter(|d| !d.doc.vendored)
        .collect();
    let mut by_bundle: BTreeMap<String, Vec<&Decision<'_>>> = BTreeMap::new();
    for d in &all {
        by_bundle.entry(d.bundle.clone()).or_default().push(d);
    }
    let mut out = Vec::new();
    for in_bundle in by_bundle.values() {
        let mut by_id: BTreeMap<String, Vec<&Decision<'_>>> = BTreeMap::new();
        for d in in_bundle {
            let id = d.id();
            if !id.is_empty() {
                by_id.entry(id).or_default().push(d);
            }
        }
        for d in in_bundle {
            out.extend(one(kb, d, &by_id));
        }
        out.extend(duplicates(kb, &by_id));
    }
    out
}

fn duplicates(kb: &Kb, by_id: &BTreeMap<String, Vec<&Decision<'_>>>) -> Vec<Finding> {
    by_id
        .iter()
        .filter(|(_, ds)| ds.len() > 1)
        .flat_map(|(id, ds)| {
            ds.iter().map(move |d| {
                err(
                    kb,
                    d,
                    "decision-duplicate-id",
                    format!(
                        "decision id `{id}` is used by {} records in {}",
                        ds.len(),
                        d.bundle
                    ),
                    Some("ids come from the filename prefix; renumber one of them".to_string()),
                )
            })
        })
        .collect()
}

fn one(kb: &Kb, d: &Decision<'_>, by_id: &BTreeMap<String, Vec<&Decision<'_>>>) -> Vec<Finding> {
    let mut out = Vec::new();

    if d.id().is_empty() {
        out.push(err(
            kb,
            d,
            "decision-no-id",
            "decision record filename does not start with a numeric id".to_string(),
            Some(
                "name it NNNN-slug.md, e.g. 0004-bridge-nothing-between-zio-and-kyo.md".to_string(),
            ),
        ));
    }

    if d.state().is_none() {
        let msg = match d.fm().str_at("state") {
            None => "decision record has no `state`".to_string(),
            Some(s) => format!("`state: {s}` is not a known state"),
        };
        out.push(err(
            kb,
            d,
            "decision-state-unknown",
            msg,
            Some(format!("one of {}", DecisionState::names())),
        ));
    }

    if d.decided().is_none() {
        out.push(warn(
            kb,
            d,
            "decision-decided-missing",
            "decision record has no valid `decided` date (YYYY-MM-DD)".to_string(),
            Some(
                "a decision without a date cannot be read in sequence with the others".to_string(),
            ),
        ));
    }

    match d.state() {
        Some(DecisionState::Superseded) => match d.superseded_by() {
            None => out.push(err(
                kb,
                d,
                "decision-superseded-no-successor",
                "Superseded decision has no `superseded_by`".to_string(),
                Some(
                    "a superseded record must say what replaced it, or a reader has nowhere to go"
                        .to_string(),
                ),
            )),
            Some(succ) if !by_id.contains_key(&succ) => out.push(err(
                kb,
                d,
                "decision-superseded-unknown",
                format!(
                    "`superseded_by: {succ}` names no decision record in {}",
                    d.bundle
                ),
                None,
            )),
            // The other direction of the mutuality below. Checking only the
            // forward one leaves a chain that is one-way from the *retired*
            // end unreported: this record points at its successor, the
            // successor says nothing, and `kb check` stays silent because
            // there is no `supersedes` entry anywhere to inspect.
            Some(succ) if !d.id().is_empty() => {
                for s in by_id.get(&succ).map(Vec::as_slice).unwrap_or_default() {
                    if !s.supersedes().contains(&d.id()) {
                        out.push(warn(
                            kb,
                            d,
                            "decision-supersede-not-mutual",
                            format!(
                                "this record names {} in `superseded_by` but {} does not list {} in `supersedes`",
                                s.id(),
                                s.id(),
                                d.id()
                            ),
                            Some(format!("add `supersedes: [\"{}\"]` to {}.md", d.id(), s.slug())),
                        ));
                    }
                }
            }
            _ => {}
        },
        Some(DecisionState::Withdrawn) if d.reason().is_none_or(|r| r.trim().is_empty()) => {
            out.push(err(
                kb,
                d,
                "decision-withdrawn-no-reason",
                "Withdrawn decision has no `reason`".to_string(),
                Some("a withdrawal without a reason is worthless six months on".to_string()),
            ));
        }
        _ => {}
    }

    // `supersedes` must name real records, and the record it names should
    // point back. One-way supersession is how a chain silently breaks: the
    // old record still reads as current to anyone who lands on it directly.
    for target in d.supersedes() {
        match by_id.get(&target) {
            None => out.push(err(
                kb,
                d,
                "decision-supersedes-unknown",
                format!(
                    "`supersedes: {target}` names no decision record in {}",
                    d.bundle
                ),
                None,
            )),
            Some(targets) => {
                for t in targets {
                    if t.superseded_by().as_deref() != Some(d.id().as_str()) {
                        out.push(warn(
                            kb,
                            d,
                            "decision-supersede-not-mutual",
                            format!(
                                "this record supersedes {} but {} does not name it in `superseded_by`",
                                t.id(),
                                t.id()
                            ),
                            Some(format!(
                                "add `superseded_by: \"{}\"` and `state: Superseded` to {}.md",
                                d.id(),
                                t.slug()
                            )),
                        ));
                    }
                }
            }
        }
    }

    out
}

fn err(kb: &Kb, d: &Decision<'_>, check: &str, msg: String, hint: Option<String>) -> Finding {
    Finding {
        severity: Severity::Error,
        check: check.to_string(),
        path: kb.rel(&d.doc.file),
        line: Some(1),
        message: msg,
        hint,
    }
}

fn warn(kb: &Kb, d: &Decision<'_>, check: &str, msg: String, hint: Option<String>) -> Finding {
    Finding {
        severity: Severity::Warn,
        check: check.to_string(),
        path: kb.rel(&d.doc.file),
        line: Some(1),
        message: msg,
        hint,
    }
}

// ----------------------------------------------------------------- rendering

/// JSON shape of one decision record, with field names matching the Scala
/// CLI output (`superseded_by` stays snake_case there, unlike intent's
/// `supersededBy`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecisionJson {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub bundle: String,
    pub path: String,
    pub state: Option<DecisionState>,
    pub decided: Option<String>,
    pub supersedes: Vec<String>,
    pub superseded_by: Option<String>,
    pub reason: Option<String>,
    pub tags: Vec<String>,
}

pub fn decision_json(d: &Decision<'_>) -> DecisionJson {
    DecisionJson {
        id: d.id(),
        slug: d.slug(),
        title: d.title(),
        description: d.description(),
        bundle: d.bundle.clone(),
        path: d.doc.bundle_path(),
        state: d.state(),
        decided: d.decided().map(|x| x.to_string()),
        supersedes: d.supersedes(),
        superseded_by: d.superseded_by(),
        reason: d.reason(),
        tags: d.tags(),
    }
}

fn to_pretty_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("serializable") + "\n"
}

/// Renders `kb decision list`, grouped by state in display order.
pub fn render_list(ds: &[Decision<'_>], json: bool) -> String {
    if json {
        let items: Vec<DecisionJson> = ds.iter().map(decision_json).collect();
        return to_pretty_json(&items);
    }
    if ds.is_empty() {
        return "no decision records\n".to_string();
    }
    let mut sb = String::new();
    for st in DecisionState::DISPLAY_ORDER {
        let group: Vec<&Decision<'_>> = ds.iter().filter(|d| d.state() == Some(st)).collect();
        if !group.is_empty() {
            sb.push_str(&format!("\n{st} ({})\n", group.len()));
            for d in group {
                sb.push_str(&format!("  {:<6} {}\n", d.id(), d.title()));
                if let Some(x) = d.description() {
                    sb.push_str(&format!("         {x}\n"));
                }
                if let Some(x) = d.superseded_by() {
                    sb.push_str(&format!("         superseded by {x}\n"));
                }
            }
        }
    }
    let unstated: Vec<&Decision<'_>> = ds.iter().filter(|d| d.state().is_none()).collect();
    if !unstated.is_empty() {
        sb.push_str(&format!("\nNo state ({})\n", unstated.len()));
        for d in unstated {
            sb.push_str(&format!("  {:<6} {}\n", d.id(), d.title()));
        }
    }
    sb.push_str(&format!("\n{} decision record(s)\n", ds.len()));
    sb
}

/// Renders `kb decision show`.
pub fn render_show(d: &Decision<'_>, body: bool, json: bool) -> String {
    if json {
        return to_pretty_json(&decision_json(d));
    }
    let mut sb = String::new();
    sb.push_str(&format!("{} — {}\n", d.id(), d.title()));
    if let Some(x) = d.description() {
        sb.push_str(&format!("{x}\n"));
    }
    sb.push_str(&format!("\nbundle:     {}\n", d.bundle));
    sb.push_str(&format!(
        "state:      {}\n",
        d.state()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "(none)".to_string())
    ));
    if let Some(x) = d.decided() {
        sb.push_str(&format!("decided:    {x}\n"));
    }
    let supersedes = d.supersedes();
    if !supersedes.is_empty() {
        sb.push_str(&format!("supersedes: {}\n", supersedes.join(", ")));
    }
    if let Some(x) = d.superseded_by() {
        sb.push_str(&format!("superseded_by: {x}\n"));
    }
    if let Some(x) = d.reason() {
        sb.push_str(&format!("reason:     {x}\n"));
    }
    let tags = d.tags();
    if !tags.is_empty() {
        sb.push_str(&format!("tags:       {}\n", tags.join(", ")));
    }
    sb.push_str(&format!(
        "path:       {}:{}\n",
        d.bundle,
        d.doc.bundle_path()
    ));
    if body {
        sb.push_str(&format!("\n{}\n", d.doc.body));
    }
    sb
}
