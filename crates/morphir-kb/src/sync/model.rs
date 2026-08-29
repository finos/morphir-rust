use std::path::PathBuf;

use morphir_okf::{Bundle, OkfProfile};

use crate::error::{Error, Result};
use crate::util::resolves_inside;

use super::{files::resolve, glob::glob_matches};

/// One `from:` entry in the manifest, with the globs that carve exceptions out of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncMapping {
    pub from: String,
    pub exclude: Vec<String>,
}

/// A bundle's `sync.yaml` — hand-written, reviewed, and the only place upstream
/// coordinates live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncManifest {
    pub repo: String,
    pub r#ref: String,
    pub refs_path: String,
    pub root: String,
    pub mappings: Vec<SyncMapping>,
    pub exclude: Vec<String>,
    /// Ordered: the first glob that matches supplies the concept's `type`.
    pub type_map: Vec<(String, String)>,
}

impl SyncManifest {
    /// Concepts are markdown; everything else rides along untouched. `.mdx` is
    /// deliberately an asset — commonmark has no business parsing JSX, and widening
    /// the markdown predicate would reclassify files in every other bundle.
    pub fn kind_of(&self, path: &str) -> SyncKind {
        if path.ends_with(".md") {
            SyncKind::Concept
        } else {
            SyncKind::Asset
        }
    }

    pub fn type_for(&self, path: &str) -> String {
        self.type_map
            .iter()
            .find(|(glob, _)| glob_matches(glob, path))
            .map(|(_, t)| t.clone())
            .unwrap_or_else(|| "Source Document".to_string())
    }

    /// `type_map` entries naming a type one of the knowledge base's registers
    /// discovers by.
    ///
    /// A mirrored document describes what it *is* — `Design Source`,
    /// `Decision Source` — and stays out of the vocabulary the registers claim.
    /// Injecting a register-owned type instead conscripts upstream's document into
    /// a register whose schema it was never written against, and the resulting
    /// findings are unfixable from this side.
    pub fn type_map_collisions(&self, profile: &OkfProfile) -> Vec<(String, String)> {
        self.type_map
            .iter()
            .filter(|(_, t)| profile.owns_type(t))
            .cloned()
            .collect()
    }

    pub fn selects(&self, path: &str) -> bool {
        let excluded = self.exclude.iter().any(|g| glob_matches(g, path));
        !excluded
            && self.mappings.iter().any(|m| {
                glob_matches(&m.from, path) && !m.exclude.iter().any(|g| glob_matches(g, path))
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncKind {
    Concept,
    Asset,
}

impl SyncKind {
    pub fn label(&self) -> &'static str {
        match self {
            SyncKind::Concept => "concept",
            SyncKind::Asset => "asset",
        }
    }

    pub fn parse(s: &str) -> SyncKind {
        if s == "asset" {
            SyncKind::Asset
        } else {
            SyncKind::Concept
        }
    }
}

/// One mirrored file as of the last import. `upstream_sha256` hashes the
/// *upstream* form, so a local file is compared by projecting it first — there is
/// no second hash to keep honest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockEntry {
    pub path: String,
    pub kind: SyncKind,
    pub upstream_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncLock {
    pub base_commit: String,
    pub imported_at: String,
    pub files: Vec<LockEntry>,
}

impl SyncLock {
    pub fn empty() -> SyncLock {
        SyncLock {
            base_commit: String::new(),
            imported_at: String::new(),
            files: Vec::new(),
        }
    }

    pub fn get(&self, path: &str) -> Option<&LockEntry> {
        self.files.iter().find(|e| e.path == path)
    }
}

/// Where a mirrored file stands relative to the import baseline. Derived on every
/// run, never stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    Clean,
    LocalOnly,
    UpstreamOnly,
    Diverged,
    MissingLocal,
    DeletedUpstream,
    DeletedUpstreamEdited,
    Untracked,
    Unreadable,
}

impl SyncState {
    pub fn label(&self) -> &'static str {
        match self {
            SyncState::Clean => "clean",
            SyncState::LocalOnly => "local-only",
            SyncState::UpstreamOnly => "upstream-only",
            SyncState::Diverged => "diverged",
            SyncState::MissingLocal => "missing-local",
            SyncState::DeletedUpstream => "deleted-upstream",
            SyncState::DeletedUpstreamEdited => "deleted-upstream-edited",
            SyncState::Untracked => "untracked",
            SyncState::Unreadable => "unreadable",
        }
    }
}

/// Where a mirrored file stands, plus whether the block we inject into it still
/// says what the manifest says.
///
/// Injection staleness is orthogonal to [`SyncState`] rather than another case of
/// it: a file can have drifted upstream *and* carry a block the manifest no longer
/// implies, and collapsing the two would lose one of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStatus {
    pub path: String,
    pub kind: SyncKind,
    pub state: SyncState,
    pub detail: String,
    pub injection_stale: bool,
}

/// What a `pull` or `push` did, or would do under `--dry-run`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncAction {
    pub verb: String,
    pub path: String,
    pub detail: String,
}

impl SyncAction {
    pub(super) fn new(verb: &str, path: &str, detail: &str) -> SyncAction {
        SyncAction {
            verb: verb.to_string(),
            path: path.to_string(),
            detail: detail.to_string(),
        }
    }
}

/// A bundle together with the manifest and lockfile that make it a mirror.
#[derive(Debug, Clone)]
pub struct SyncBundle {
    pub bundle: Bundle,
    pub manifest: SyncManifest,
    pub lock: SyncLock,
}

impl SyncBundle {
    pub fn mirror_root(&self) -> PathBuf {
        resolve(&self.bundle.root, &self.manifest.root)
    }

    pub fn local_file(&self, rel: &str) -> PathBuf {
        resolve(&self.mirror_root(), rel)
    }

    /// The mirrored file at `rel`, refused when the path it names on disk does
    /// not really sit inside the bundle.
    ///
    /// [`crate::sync::safe_relative`] and `validated_root` read the strings; this reads the
    /// filesystem. A mirror root called `sources` that is a symlink to
    /// `../../../victim` satisfies both of them and escapes anyway, and every
    /// mirror read, write and delete goes through here — including the one in
    /// `pull --prune`, which is the operation that destroys work nobody can get
    /// back.
    pub fn mirror_file(&self, rel: &str) -> Result<PathBuf> {
        let file = self.local_file(rel);
        if resolves_inside(&self.bundle.root, &file) {
            Ok(file)
        } else {
            Err(Error::msg(format!(
                "sync.yaml `root: {}` resolves outside the bundle at `{rel}` \
                 — a directory on the way there is a symlink leading out; a mirror is a real directory inside the bundle, e.g. `sources`",
                self.manifest.root
            )))
        }
    }
}
