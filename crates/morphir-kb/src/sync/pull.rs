use std::fs;
use std::path::Path;

use chrono::NaiveDate;

use crate::error::Result;

use super::files::{LOCK_NAME, delete_file, resolve, safe_relative, sha256, write_bytes};
use super::manifest::render_lock;
use super::model::{FileStatus, LockEntry, SyncAction, SyncBundle, SyncKind, SyncLock, SyncState};
use super::projection::{inject, injected_keys, reinjected};
use super::status::status;

#[derive(Debug, Clone)]
pub struct PullResult {
    pub actions: Vec<SyncAction>,
    pub lock: SyncLock,
    /// Non-empty means the CLI exits non-zero: files were never imported.
    pub refused: Vec<FileStatus>,
}

/// Rewrites a mirrored concept's fenced block to what the manifest now implies,
/// when the two have parted company.
///
/// Reported as its own verb rather than folded into `updated`: nothing came from
/// upstream, and a bulk re-injection across a whole mirror should not read as an
/// import. Silent when the file cannot be taken apart — that is the `unreadable`
/// state, and it has its own finding.
fn reinject_if_stale(
    sb: &SyncBundle,
    st: &FileStatus,
    dry_run: bool,
) -> Result<Option<SyncAction>> {
    if !st.injection_stale {
        return Ok(None);
    }
    let file = sb.mirror_file(&st.path)?;
    let bytes = fs::read(&file)?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    match reinjected(&sb.manifest, &st.path, &text) {
        Err(_) => Ok(None),
        Ok(out) => {
            if !dry_run {
                write_bytes(&file, out.as_bytes())?;
            }
            Ok(Some(SyncAction::new(
                "re-injected",
                &st.path,
                "the manifest implies different keys",
            )))
        }
    }
}

pub fn pull(
    sb: &SyncBundle,
    upstream_root: &Path,
    base_commit: &str,
    today: NaiveDate,
    dry_run: bool,
    theirs: bool,
    prune: bool,
) -> Result<PullResult> {
    let rows = status(sb, Some(upstream_root))?;
    let mut actions: Vec<SyncAction> = Vec::new();
    let mut entries: Vec<LockEntry> = Vec::new();
    let mut refused: Vec<FileStatus> = Vec::new();
    for st in rows {
        let rel = st.path.clone();
        let import_it = match st.state {
            SyncState::Untracked | SyncState::UpstreamOnly | SyncState::MissingLocal => true,
            SyncState::Diverged | SyncState::Unreadable => theirs,
            _ => false,
        };
        if !safe_relative(&rel) {
            actions.push(SyncAction::new("refused", &rel, "path escapes the mirror"));
            refused.push(st);
        } else if st.state == SyncState::DeletedUpstreamEdited {
            // Never pruned, and never taken by --theirs either: taking theirs here
            // means deleting our edit, which is not what anyone reaching for that
            // flag is asking for. The lock entry is kept so the file stays tracked.
            actions.push(SyncAction::new(
                "held back",
                &rel,
                "deleted upstream but edited here",
            ));
            entries.extend(sb.lock.get(&rel).cloned());
            refused.push(st);
        } else if st.state == SyncState::DeletedUpstream {
            let act = if prune { "removed" } else { "gone upstream" };
            if prune && !dry_run {
                delete_file(&sb.mirror_file(&rel)?)?;
            }
            actions.push(SyncAction::new(act, &rel, "no longer present upstream"));
            if !prune {
                entries.extend(sb.lock.get(&rel).cloned());
            }
        } else if !import_it {
            if st.state == SyncState::Diverged || st.state == SyncState::Unreadable {
                entries.extend(sb.lock.get(&rel).cloned());
                refused.push(st);
            } else {
                // A clean file whose recorded hash is stale — both sides moved to
                // the same content, which is what an export leaves behind — gets
                // its baseline refreshed. No write, just the lock catching up.
                //
                // And a file whose injected block no longer matches the manifest is
                // rewritten in place. Nothing else would ever reach it: the block
                // is invisible to `state_of`, so an otherwise clean file is passed
                // over and a `type_map` edit never lands. Re-injection touches only
                // the fence, so the upstream form and the hash beside it are
                // unchanged — which is why this can be done to a `local-only` file
                // just as safely.
                let bytes = fs::read(resolve(upstream_root, &rel))?;
                let hash = sha256(&bytes);
                let rebaselined = sb.lock.get(&rel).is_some_and(|e| e.upstream_sha256 != hash);
                if rebaselined {
                    actions.push(SyncAction::new(
                        "rebaselined",
                        &rel,
                        "already in step upstream",
                    ));
                }
                entries.extend(sb.lock.get(&rel).map(|e| LockEntry {
                    upstream_sha256: hash,
                    ..e.clone()
                }));
                actions.extend(reinject_if_stale(sb, &st, dry_run)?);
            }
        } else {
            let bytes = fs::read(resolve(upstream_root, &rel))?;
            let hash = sha256(&bytes);
            let kind = sb.manifest.kind_of(&rel);
            let out: Vec<u8> = match kind {
                SyncKind::Concept => {
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    inject(&text, &injected_keys(&sb.manifest, &rel, &text)).into_bytes()
                }
                SyncKind::Asset => bytes,
            };
            let verb = if sb.lock.get(&rel).is_none() {
                "added"
            } else {
                "updated"
            };
            if !dry_run {
                write_bytes(&sb.mirror_file(&rel)?, &out)?;
            }
            actions.push(SyncAction::new(verb, &rel, kind.label()));
            entries.push(LockEntry {
                path: rel,
                kind,
                upstream_sha256: hash,
            });
        }
    }
    Ok(PullResult {
        actions,
        lock: SyncLock {
            base_commit: base_commit.to_string(),
            imported_at: today.to_string(),
            files: entries,
        },
        refused,
    })
}

/// Writes the lockfile, but leaves `imported_at` alone when nothing was actually
/// imported.
///
/// A pull that changes nothing should leave no diff. Stamping the date
/// unconditionally meant every run dirtied the lockfile, which turns `git status`
/// into noise and trains people to commit it without looking.
pub fn write_lock(sb: &SyncBundle, lock: &SyncLock) -> Result<SyncLock> {
    let sorted = |files: &[LockEntry]| {
        let mut v: Vec<LockEntry> = files.to_vec();
        v.sort_by(|a, b| a.path.cmp(&b.path));
        v
    };
    let same =
        sb.lock.base_commit == lock.base_commit && sorted(&sb.lock.files) == sorted(&lock.files);
    let to_write = if same && !sb.lock.imported_at.is_empty() {
        SyncLock {
            imported_at: sb.lock.imported_at.clone(),
            ..lock.clone()
        }
    } else {
        lock.clone()
    };
    fs::write(sb.bundle.root.join(LOCK_NAME), render_lock(&to_write))?;
    Ok(to_write)
}
