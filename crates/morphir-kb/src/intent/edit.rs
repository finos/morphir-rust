use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use morphir_okf::frontmatter::split_frontmatter;
use morphir_okf::model::{Bundle, Kb};
use morphir_okf::paths;

use crate::error::{Error, Result};
use crate::util::{slugify, yaml_str};

use super::MARKER;
use super::model::{DocRef, Intent, IntentKind, IntentState, find, next_id, resolve_ref};

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
    let eol = line_terminator(&text);
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
    let rewritten = format!("---\n{}\n---\n{body}", out.join("\n").trim_end());
    fs::write(file, with_terminator(&rewritten, eol))?;
    Ok(())
}

/// The line terminator a document is written with.
///
/// `split_frontmatter` normalizes CRLF→LF, so without restoring the original
/// terminator every edit to a CRLF document rewrites every line — a diff of
/// one key becomes a diff of the whole file. Only a uniformly-CRLF document
/// is reported as CRLF: a mixed document has no single right answer, and LF
/// keeps the behaviour these writers have always had.
pub(super) fn line_terminator(text: &str) -> &'static str {
    let crlf = text.matches("\r\n").count();
    if crlf > 0 && crlf == text.matches('\n').count() {
        "\r\n"
    } else {
        "\n"
    }
}

/// Rewrites LF-terminated `text` with `eol`. A no-op for LF.
pub(super) fn with_terminator(text: &str, eol: &'static str) -> String {
    if eol == "\n" {
        text.to_string()
    } else {
        text.replace('\n', eol)
    }
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
