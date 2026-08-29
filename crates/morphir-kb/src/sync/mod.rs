//! Vendoring external documents into a bundle, and projecting them back out.
//!
//! Ported from `KbSync.scala` (and the sync findings assembly in `kb.scala`). A
//! bundle may declare a `sync.yaml` naming an upstream repository and the paths it
//! mirrors. Markdown lands as OKF concepts; everything else lands as byte-identical
//! assets. The knowledge base owns a fenced region inside each mirrored concept's
//! frontmatter, and [`project`] removes exactly that region — so the file that goes
//! back upstream is the file that came from it.
//!
//! The whole design rests on one invariant, pinned by the sync test suite:
//!
//! ```text
//! project(inject(bytes)) == bytes
//! ```
//!
//! byte for byte, including line endings. That is why nothing here re-serializes
//! YAML: upstream frontmatter is moved around as lines, never parsed and rewritten,
//! so a fractional `sidebar_position` or a nested `tracking:` block cannot be
//! reformatted by accident.
//!
//! The invariant has a corollary worth stating, because missing it was a bug in the
//! reference implementation: since projection strips the fenced region, the region
//! is invisible to every hash comparison here, and nothing about upstream drift can
//! tell you that *our own* injection has gone stale. [`reinjected`] is the answer —
//! the manifest is compared against each file directly, so editing `type_map`
//! reaches files that were imported long ago.

mod check;
mod diff;
mod files;
mod glob;
mod index;
mod manifest;
mod model;
mod projection;
mod pull;
mod push;
mod render;
mod status;

pub use check::{all_sync_findings, check_findings};
pub use diff::{DiffResult, DiffSelection, DiffSet, diff, diff_many, diff_selected, is_glob};
pub use files::{
    LOCK_NAME, MANIFEST_NAME, find_bundle, git_head, load, relative_files_under, resolve,
    safe_relative, sha256, upstream_root,
};
pub use glob::glob_matches;
pub use index::{INDEX_MARKER, generate_index};
pub use manifest::{collision_message, parse_lock, parse_manifest, render_lock};
pub use model::{
    FileStatus, LockEntry, SyncAction, SyncBundle, SyncKind, SyncLock, SyncManifest, SyncMapping,
    SyncState,
};
pub use projection::{
    FENCE_BEGIN, FENCE_END, GENERATED_KEYS, Split, inject, injected_keys, injection_stale, project,
    reinject, reinjected, split,
};
pub use pull::{PullResult, pull, write_lock};
pub use push::{PushResult, push};
pub use render::{
    render_actions, render_diff_json, render_diff_raw, render_diff_text, render_diffs_json,
    render_diffs_raw, render_diffs_text, render_status,
};
pub use status::{known_paths, status, strict_violations};
