use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use morphir_okf::{Bundle, Kb};

use crate::error::{Error, Result};
use crate::util::contained_relative;

use super::manifest::{parse_lock, parse_manifest};
use super::model::{SyncBundle, SyncLock};

pub const MANIFEST_NAME: &str = "sync.yaml";
pub const LOCK_NAME: &str = "sync.lock.yaml";

pub fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub(super) fn write_bytes(p: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(p, bytes)?;
    Ok(())
}

pub(super) fn delete_file(p: &Path) -> Result<()> {
    match fs::remove_file(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Every file at or below `dir`, as `/`-separated paths relative to it.
/// `.git` is never content, and walking it on a full checkout costs seconds.
pub fn relative_files_under(dir: &Path) -> Result<Vec<String>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| !(e.file_type().is_dir() && e.file_name() == ".git"))
    {
        let entry = entry.map_err(|e| Error::msg(e.to_string()))?;
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(dir)
                .map_err(|e| Error::msg(e.to_string()))?
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push(rel);
        }
    }
    Ok(out)
}

pub fn resolve(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for seg in rel.split('/').filter(|s| !s.is_empty()) {
        p.push(seg);
    }
    p
}

/// Rejects a manifest path that would write outside the mirror — the same guard
/// `add-concept` carries, and on the same terms: see [`crate::util::path_fault`]
/// for why `\` counts as a separator and why a lone backslash in a filename
/// does not.
pub fn safe_relative(rel: &str) -> bool {
    contained_relative(rel)
}

/// Current HEAD of a local checkout, or `None` when it is not a git repository.
///
/// Validated by shape rather than by exit code: a successful `git rev-parse HEAD`
/// is exactly one 40-character hex SHA. Anything else means "not a git checkout".
pub fn git_head(repo: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    let head = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let is_sha = head.len() == 40 && head.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'));
    is_sha.then_some(head)
}

// ------------------------------------------------------------------- loading

/// The bundle declaring `sync: true`, or a named one. Nothing is hardcoded, so
/// this works in any repository.
pub fn find_bundle<'a>(kb: &'a Kb, label: Option<&str>) -> Option<&'a Bundle> {
    match label {
        Some(l) => kb.bundle(l),
        // Truthiness matters: `sync: false` is a bundle saying no, and treating
        // mere presence as yes would make turning the marker off require deleting it.
        None => kb.bundles.iter().find(|b| {
            b.index
                .fm()
                .str_at("sync")
                .is_some_and(|v| v == "true" || v == "yes")
        }),
    }
}

pub fn load(b: &Bundle) -> Result<SyncBundle> {
    let mf = b.root.join(MANIFEST_NAME);
    if !mf.exists() {
        return Err(Error::msg(format!("{} has no {MANIFEST_NAME}", b.label())));
    }
    let raw_m = fs::read_to_string(&mf)?;
    let lf = b.root.join(LOCK_NAME);
    let lock = if lf.exists() {
        parse_lock(&fs::read_to_string(&lf)?)?
    } else {
        SyncLock::empty()
    };
    let manifest = parse_manifest(&raw_m)?;
    Ok(SyncBundle {
        bundle: b.clone(),
        manifest,
        lock,
    })
}

/// The upstream checkout a sync bundle reads from, when it is present on disk.
/// `.refs/` sits beside `kb/`, which is the convention `kb check` already follows
/// for provenance.
pub fn upstream_root(refs: &Path, sb: &SyncBundle) -> Option<PathBuf> {
    let candidate = resolve(refs, &sb.manifest.refs_path);
    candidate.exists().then_some(candidate)
}
