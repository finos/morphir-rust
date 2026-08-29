use std::collections::{BTreeMap, HashSet};

use chrono::NaiveDate;
use morphir_okf::model::{Bundle, Finding, Kb, Severity};

use super::model::{
    DocRef, Intent, IntentConfig, IntentKind, IntentState, config, intents, resolve_ref,
};

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
