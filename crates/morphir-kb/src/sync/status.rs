use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::error::Result;

use super::files::{relative_files_under, resolve, sha256};
use super::model::{FileStatus, SyncBundle, SyncKind, SyncState};
use super::projection::{injection_stale, project};

/// Compares local, baseline and upstream for one path. Upstream is optional so
/// status works without a checkout.
fn state_of(
    lock_hash: Option<&str>,
    local_upstream_form: Option<&std::result::Result<Vec<u8>, String>>,
    upstream_hash: Option<&str>,
) -> SyncState {
    match (lock_hash, local_upstream_form, upstream_hash) {
        (_, Some(Err(_)), _) => SyncState::Unreadable,
        (None, _, _) => SyncState::Untracked,
        (Some(_), None, _) => SyncState::MissingLocal,
        (Some(base), Some(Ok(local)), up) => {
            let local_hash = sha256(local);
            let local_changed = local_hash != base;
            match up {
                None => {
                    if local_changed {
                        SyncState::LocalOnly
                    } else {
                        SyncState::Clean
                    }
                }
                // Agreement beats the baseline. After an export the checkout holds
                // our change, so both sides differ from the recorded hash while
                // being identical to each other — there is nothing to send and
                // nothing to take. Without this, pushing twice into the same
                // checkout reports divergence the first push created.
                Some(h) if h == local_hash => SyncState::Clean,
                Some(h) if h == base => {
                    if local_changed {
                        SyncState::LocalOnly
                    } else {
                        SyncState::Clean
                    }
                }
                Some(_) => {
                    if local_changed {
                        SyncState::Diverged
                    } else {
                        SyncState::UpstreamOnly
                    }
                }
            }
        }
    }
}

/// A mirrored file as it sits on disk: the bytes it would go back upstream as,
/// and — for a concept — the text they came from, which is the only place the
/// injected block can be read.
///
/// Bytes, not text, for the upstream form: an asset is whatever upstream stores,
/// and decoding one as UTF-8 to hash or export it would replace any invalid
/// sequence with U+FFFD. That makes a freshly pulled binary look locally modified,
/// and then writes the corruption out on push. Only concepts are text, because
/// only concepts carry a frontmatter fence — which is also why `text` is `None`
/// for an asset.
pub(super) struct LocalCopy {
    pub(super) upstream_form: std::result::Result<Vec<u8>, String>,
    pub(super) text: Option<String>,
}

/// Reads a mirrored file into its [`LocalCopy`], or `None` when the mirror does
/// not have it.
pub(super) fn local_copy_of(
    sb: &SyncBundle,
    rel: &str,
    kind: SyncKind,
) -> Result<Option<LocalCopy>> {
    let f = sb.mirror_file(rel)?;
    if !f.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&f)?;
    Ok(Some(match kind {
        SyncKind::Concept => {
            let text = String::from_utf8_lossy(&bytes).into_owned();
            LocalCopy {
                upstream_form: project(&text).map(String::into_bytes),
                text: Some(text),
            }
        }
        SyncKind::Asset => LocalCopy {
            upstream_form: Ok(bytes),
            text: None,
        },
    }))
}

pub(super) fn upstream_form_of(
    sb: &SyncBundle,
    rel: &str,
    kind: SyncKind,
) -> Result<Option<std::result::Result<Vec<u8>, String>>> {
    Ok(local_copy_of(sb, rel, kind)?.map(|c| c.upstream_form))
}

/// The files upstream holds that the manifest claims — everything a checkout
/// contributes to the mirror's file set.
fn selected_upstream_rels(sb: &SyncBundle, upstream_root: Option<&Path>) -> Result<Vec<String>> {
    match upstream_root {
        None => Ok(Vec::new()),
        Some(r) => Ok(relative_files_under(r)?
            .into_iter()
            .filter(|rel| sb.manifest.selects(rel))
            .collect()),
    }
}

/// The lockfile's paths and the checkout's, as one sorted, deduplicated list.
fn union_of(sb: &SyncBundle, upstream_rels: &[String]) -> Vec<String> {
    let mut paths: Vec<String> = sb
        .lock
        .files
        .iter()
        .map(|e| e.path.clone())
        .chain(upstream_rels.iter().cloned())
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

/// Every mirrored path this bundle knows about: what the lockfile records, plus
/// whatever the checkout now holds that the manifest selects.
///
/// One list, two callers. [`status`] reports a row per entry and [`crate::sync::diff_many`]
/// selects from it, so the two cannot disagree about which files exist — a path
/// `sync status` lists is a path `sync diff` will compare, and a glob can reach
/// nothing that `sync status` does not show.
pub fn known_paths(sb: &SyncBundle, upstream_root: Option<&Path>) -> Result<Vec<String>> {
    Ok(union_of(sb, &selected_upstream_rels(sb, upstream_root)?))
}

pub fn status(sb: &SyncBundle, upstream_root: Option<&Path>) -> Result<Vec<FileStatus>> {
    let upstream_rels = selected_upstream_rels(sb, upstream_root)?;
    let mut upstream_hashes: HashMap<String, String> = HashMap::new();
    if let Some(r) = upstream_root {
        for rel in &upstream_rels {
            let bytes = fs::read(resolve(r, rel))?;
            upstream_hashes.insert(rel.clone(), sha256(&bytes));
        }
    }
    let paths = union_of(sb, &upstream_rels);
    let mut rows = Vec::new();
    for rel in paths {
        let entry = sb.lock.get(&rel);
        let kind = entry
            .map(|e| e.kind)
            .unwrap_or_else(|| sb.manifest.kind_of(&rel));
        let copy = local_copy_of(sb, &rel, kind)?;
        let local = copy.as_ref().map(|c| &c.upstream_form);
        let up = upstream_hashes.get(rel.as_str()).map(String::as_str);
        let gone_upstream = entry.is_some() && upstream_root.is_some() && up.is_none();
        // A file upstream has deleted but we have since edited is a conflict, not a
        // clean deletion. Reporting it as `deleted-upstream` would let
        // `pull --prune` throw the edit away without asking — the one operation
        // here that destroys work nobody can recover.
        let edited_locally = match (entry, local) {
            (Some(e), Some(Ok(bytes))) => sha256(bytes) != e.upstream_sha256,
            (_, Some(Err(_))) => true,
            _ => false,
        };
        let state = if gone_upstream && edited_locally {
            SyncState::DeletedUpstreamEdited
        } else if gone_upstream {
            SyncState::DeletedUpstream
        } else {
            state_of(entry.map(|e| e.upstream_sha256.as_str()), local, up)
        };
        let detail = match local {
            Some(Err(err)) => err.clone(),
            _ => String::new(),
        };
        // Derived from the local file alone, so it is decided with or without a
        // reference checkout — a manifest edit that was never applied is visible
        // from `kb check` and `kb sync status --no-upstream` both.
        let stale = copy
            .as_ref()
            .and_then(|c| c.text.as_deref())
            .is_some_and(|t| injection_stale(&sb.manifest, &rel, t));
        rows.push(FileStatus {
            path: rel,
            kind,
            state,
            detail,
            injection_stale: stale,
        });
    }
    Ok(rows)
}

/// How many rows `sync status --strict` counts against the exit code: anything
/// diverged or unreadable, plus any stale injection. A stale injection counts as
/// strict-bad because it means sync.yaml was edited and never applied, which is a
/// manifest that is only true on paper; `kb sync pull` fixes it mechanically.
pub fn strict_violations(rows: &[FileStatus]) -> usize {
    rows.iter()
        .filter(|r| {
            matches!(r.state, SyncState::Diverged | SyncState::Unreadable) || r.injection_stale
        })
        .count()
}
