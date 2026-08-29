use std::path::Path;

use morphir_okf::{Finding, Kb, Severity};

use crate::error::Result;

use super::files::{MANIFEST_NAME, load, upstream_root};
use super::model::{SyncBundle, SyncState};
use super::projection::{FENCE_BEGIN, FENCE_END};
use super::status::status;

/// Findings a sync bundle owes, in the same shape as every other check so
/// `kb check` renders them identically.
///
/// Drift is a prompt, not a failure: only a damaged fence and a lockfile that
/// disagrees with the disk are errors, because those mean an export would send the
/// wrong bytes.
pub fn check_findings(
    kb: &Kb,
    sb: &SyncBundle,
    upstream_root: Option<&Path>,
) -> Result<Vec<Finding>> {
    let rows = status(sb, upstream_root)?;
    let mut findings = Vec::new();
    for r in rows {
        let where_ = kb.rel(&sb.local_file(&r.path));
        // Only where nothing else is already saying "pull this". A file that has
        // drifted upstream will be re-injected by the same import that takes
        // upstream's change, so reporting both would be two findings for one action.
        if r.injection_stale && matches!(r.state, SyncState::Clean | SyncState::LocalOnly) {
            findings.push(Finding {
                severity: Severity::Warn,
                check: "sync-injection-stale".to_string(),
                path: where_.clone(),
                line: None,
                message: "the `# kb:begin` block does not say what sync.yaml now implies"
                    .to_string(),
                hint: Some(
                    "run `kb sync pull` — it rewrites the block in place; keys you added inside the fence are kept"
                        .to_string(),
                ),
            });
        }
        let state_finding = match r.state {
            SyncState::Unreadable => Some(Finding {
                severity: Severity::Error,
                check: "sync-projection-broken".to_string(),
                path: where_,
                line: Some(1),
                message: format!("cannot reduce this file to its upstream form: {}", r.detail),
                hint: Some(format!(
                    "the `{FENCE_BEGIN}` … `{FENCE_END}` region is damaged; restore it or re-run `kb sync pull --theirs`"
                )),
            }),
            SyncState::MissingLocal => Some(Finding {
                severity: Severity::Error,
                check: "sync-lock-drift".to_string(),
                path: where_,
                line: None,
                message: "listed in sync.lock.yaml but absent from the mirror".to_string(),
                hint: Some(
                    "run `kb sync pull` to restore it, or `kb sync pull --prune` if upstream dropped it"
                        .to_string(),
                ),
            }),
            SyncState::Untracked => Some(Finding {
                severity: Severity::Warn,
                check: "sync-untracked".to_string(),
                path: where_,
                line: None,
                message: "matches a manifest mapping but is not in sync.lock.yaml".to_string(),
                hint: Some("run `kb sync pull` to import it".to_string()),
            }),
            SyncState::Diverged => Some(Finding {
                severity: Severity::Warn,
                check: "sync-diverged".to_string(),
                path: where_,
                line: None,
                message: "changed here and upstream since the last import".to_string(),
                hint: Some(
                    "reconcile by hand, then export; `kb sync diff` shows both sides".to_string(),
                ),
            }),
            SyncState::UpstreamOnly => Some(Finding {
                severity: Severity::Warn,
                check: "sync-upstream-drift".to_string(),
                path: where_,
                line: None,
                message: "upstream has moved on since the last import".to_string(),
                hint: Some(
                    "run `kb sync pull` to take it — nothing here is lost, this file has no local edits"
                        .to_string(),
                ),
            }),
            SyncState::DeletedUpstreamEdited => Some(Finding {
                severity: Severity::Error,
                check: "sync-deleted-upstream-edited".to_string(),
                path: where_,
                line: None,
                message: "deleted upstream, but edited here since the last import".to_string(),
                hint: Some(
                    "nothing will prune or overwrite it; restore it upstream and export, or revert the local edit"
                        .to_string(),
                ),
            }),
            SyncState::DeletedUpstream => Some(Finding {
                severity: Severity::Warn,
                check: "sync-deleted-upstream".to_string(),
                path: where_,
                line: None,
                message: "no longer present upstream".to_string(),
                hint: Some(
                    "`kb sync pull --prune` removes it here too, if that is what you want"
                        .to_string(),
                ),
            }),
            SyncState::Clean | SyncState::LocalOnly => None,
        };
        findings.extend(state_finding);
    }
    Ok(findings)
}

/// Sync findings for every bundle that mirrors something, folded into `kb check`
/// alongside the rest.
///
/// These bundles have a `sync.yaml` by construction — `mirror` is read from it —
/// so a failure to load is a manifest this tooling refuses, and it used to pass in
/// silence because every sync command was refusing it too. An error rather than a
/// warning: nothing can be pulled or exported until it is fixed.
pub fn all_sync_findings(kb: &Kb, refs: &Path, use_upstream: bool) -> Result<Vec<Finding>> {
    let mut out = Vec::new();
    for b in kb.bundles.iter().filter(|b| b.mirror.is_some()) {
        match load(b) {
            Err(err) => out.push(Finding {
                severity: Severity::Error,
                check: "sync-manifest-invalid".to_string(),
                path: kb.rel(&b.root.join(MANIFEST_NAME)),
                line: None,
                message: err.to_string(),
                hint: Some(
                    "`kb sync status` reports the same failure; fix sync.yaml and re-run `kb sync pull`"
                        .to_string(),
                ),
            }),
            Ok(sb) => {
                let up = if use_upstream {
                    upstream_root(refs, &sb)
                } else {
                    None
                };
                out.extend(check_findings(kb, &sb, up.as_deref())?);
            }
        }
    }
    Ok(out)
}
