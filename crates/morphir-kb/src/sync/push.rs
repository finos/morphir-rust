use std::path::Path;

use crate::error::Result;

use super::files::{resolve, safe_relative, write_bytes};
use super::model::{FileStatus, SyncAction, SyncBundle, SyncState};
use super::status::{status, upstream_form_of};

#[derive(Debug, Clone)]
pub struct PushResult {
    pub actions: Vec<SyncAction>,
    /// Non-empty means the CLI exits non-zero: something was held back or refused.
    pub refused: Vec<FileStatus>,
}

/// Writes the upstream form of everything changed here into `target`.
///
/// `upstream_root` is what makes `include_diverged` mean anything: without it a
/// file that moved on both sides is indistinguishable from one that only moved
/// here, and would be exported silently — overwriting upstream's own change. Pass
/// the checkout whenever there is one.
pub fn push(
    sb: &SyncBundle,
    target: &Path,
    upstream_root: Option<&Path>,
    dry_run: bool,
    include_diverged: bool,
) -> Result<PushResult> {
    let rows = status(sb, upstream_root)?;
    let mut actions: Vec<SyncAction> = Vec::new();
    let mut refused: Vec<FileStatus> = Vec::new();
    for st in rows {
        let exportable = match st.state {
            SyncState::LocalOnly => true,
            SyncState::Diverged => include_diverged,
            _ => false,
        };
        // A diverged file held back is reported rather than passed over in
        // silence: it is the one case where doing nothing loses work, because
        // somebody has to reconcile the two changes by hand.
        if st.state == SyncState::Diverged && !include_diverged {
            actions.push(SyncAction::new(
                "held back",
                &st.path,
                "diverged — reconcile, or pass --include-diverged",
            ));
            refused.push(st);
        } else if st.state == SyncState::DeletedUpstreamEdited {
            actions.push(SyncAction::new(
                "held back",
                &st.path,
                "deleted upstream but edited here — restore it there, or drop the edit",
            ));
            refused.push(st);
        } else if !exportable || !safe_relative(&st.path) {
            // Nothing to do.
        } else {
            // Through `upstream_form_of` so assets travel as the bytes they are;
            // only concepts get text projection.
            match upstream_form_of(sb, &st.path, st.kind)? {
                Some(Err(err)) => {
                    actions.push(SyncAction::new("refused", &st.path, &err));
                    refused.push(st);
                }
                Some(Ok(out)) => {
                    if !dry_run {
                        write_bytes(&resolve(target, &st.path), &out)?;
                    }
                    actions.push(SyncAction::new("wrote", &st.path, st.state.label()));
                }
                None => {
                    actions.push(SyncAction::new(
                        "refused",
                        &st.path,
                        "vanished before it could be written",
                    ));
                    refused.push(st);
                }
            }
        }
    }
    Ok(PushResult { actions, refused })
}
