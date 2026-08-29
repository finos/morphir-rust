use serde_yaml::{Mapping, Value};

use morphir_okf::OkfProfile;

use crate::error::{Error, Result};
use crate::util::{PathFault, path_fault, yaml_str};

use super::model::{LockEntry, SyncKind, SyncLock, SyncManifest, SyncMapping};

fn first_line(msg: &str) -> String {
    msg.lines().next().unwrap_or_default().to_string()
}

fn top_mapping(raw: &str) -> std::result::Result<Mapping, String> {
    let value: Value = serde_yaml::from_str(raw).map_err(|e| first_line(&e.to_string()))?;
    match value {
        Value::Mapping(m) => Ok(m),
        _ => Ok(Mapping::new()),
    }
}

fn value_at<'a>(m: &'a Mapping, key: &str) -> Option<&'a Value> {
    m.get(Value::String(key.to_string()))
}

fn mapping_at<'a>(m: &'a Mapping, key: &str) -> Option<&'a Mapping> {
    value_at(m, key).and_then(Value::as_mapping)
}

fn list_at<'a>(m: &'a Mapping, key: &str) -> Vec<&'a Value> {
    match value_at(m, key) {
        Some(Value::Sequence(items)) => items.iter().collect(),
        _ => Vec::new(),
    }
}

/// A scalar as a string, on the same terms as the reference implementation's `str`:
/// strings and integers only. serde_yaml never resolves an unquoted `2026-08-02`
/// into a date type — it stays a string — so date-valued keys (`imported_at`) read
/// back verbatim through the string arm.
fn scalar_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) if n.is_i64() || n.is_u64() => Some(n.to_string()),
        _ => None,
    }
}

fn str_at(m: &Mapping, key: &str) -> Option<String> {
    value_at(m, key).and_then(scalar_str)
}

fn strs_at(m: &Mapping, key: &str) -> Vec<String> {
    match value_at(m, key) {
        Some(Value::Sequence(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// The mirror directory a manifest declares, refused when it would not be inside
/// the bundle.
///
/// [`SyncBundle::mirror_root`] resolves `root` segment by segment onto the bundle
/// directory, so a `..` in it puts the whole mirror somewhere else: `../shared`
/// wrote into a sibling bundle, and `pull --prune` then deleted files there. This
/// is the same guard [`safe_relative`] already gives each mirrored file, applied to
/// the directory they all hang from.
///
/// An absolute root is refused outright rather than reinterpreted. Resolving it
/// would quietly turn `/etc/morphir` into `<bundle>/etc/morphir`, which is neither
/// what it says nor something anybody would write on purpose — better to say so
/// than to mirror into a directory the author did not name. An absent or empty
/// root keeps its historical default of `sources`.
///
/// "Absolute" and "escaping" are decided by [`crate::util::path_fault`], which
/// reads `\` as a separator as well as `/`. A manifest is committed and pulled
/// on Windows too, where `..\victim` and `C:\victim` are exactly the escapes
/// `../victim` and `/victim` are here.
fn validated_root(declared: Option<String>) -> Result<String> {
    let root = declared.unwrap_or_default();
    if root.is_empty() {
        return Ok("sources".to_string());
    }
    match path_fault(&root) {
        Some(PathFault::Anchored) => Err(Error::msg(format!(
            "sync.yaml `root: {root}` must be relative to the bundle, e.g. `sources` \
             — an absolute path is refused rather than silently reread as a bundle subdirectory"
        ))),
        Some(PathFault::Escapes) => Err(Error::msg(format!(
            "sync.yaml `root: {root}` leaves the bundle \
             — a root is a plain directory inside it, e.g. `sources`, with no `.` or `..` segments"
        ))),
        None => Ok(root),
    }
}

pub fn parse_manifest(raw: &str) -> Result<SyncManifest> {
    let top = top_mapping(raw).map_err(Error::msg)?;
    let empty = Mapping::new();
    let up = mapping_at(&top, "upstream").unwrap_or(&empty);
    let Some(repo) = str_at(up, "repo") else {
        return Err(Error::msg(
            "sync.yaml needs `upstream.repo`, e.g. `finos/morphir`",
        ));
    };
    let mappings: Vec<SyncMapping> = list_at(&top, "mappings")
        .into_iter()
        .filter_map(|entry| match entry {
            Value::String(s) => Some(SyncMapping {
                from: s.clone(),
                exclude: Vec::new(),
            }),
            other => {
                let m = other.as_mapping()?;
                str_at(m, "from").map(|from| SyncMapping {
                    from,
                    exclude: strs_at(m, "exclude"),
                })
            }
        })
        .collect();
    if mappings.is_empty() {
        return Err(Error::msg(
            "sync.yaml needs at least one entry under `mappings:`",
        ));
    }
    let root = validated_root(str_at(&top, "root"))?;
    let manifest = SyncManifest {
        refs_path: str_at(up, "refs_path").unwrap_or_else(|| repo.clone()),
        r#ref: str_at(up, "ref").unwrap_or_else(|| "main".to_string()),
        root,
        mappings,
        exclude: strs_at(&top, "exclude"),
        // serde_yaml's Mapping preserves document order, and order decides which
        // glob wins.
        type_map: mapping_at(&top, "type_map")
            .map(|m| {
                m.iter()
                    .filter_map(|(k, v)| match v {
                        Value::String(t) => {
                            Some((scalar_str(k).unwrap_or_else(|| format!("{k:?}")), t.clone()))
                        }
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        repo,
    };
    // Rejected at parse time so that every command reading the manifest refuses it,
    // rather than each having to remember to ask. `kb check` reports the same
    // failure without a checkout — see [`all_sync_findings`].
    let profile = OkfProfile::default();
    let bad = manifest.type_map_collisions(&profile);
    if bad.is_empty() {
        Ok(manifest)
    } else {
        Err(Error::msg(collision_message(&bad, &profile)))
    }
}

/// Why a `type_map` entry is refused, naming the entry and what to write instead.
pub fn collision_message(bad: &[(String, String)], profile: &OkfProfile) -> String {
    let entries = bad
        .iter()
        .map(|(glob, t)| format!("`\"{glob}\": {t}`"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "sync.yaml type_map injects a type a register owns: {entries} — \
         a mirrored document would be pulled into that register and judged against a schema that is not its own; \
         name what the file is instead (e.g. `Decision Source`). Register-owned: {}",
        profile.register_owned_types.join(", ")
    )
}

pub fn parse_lock(raw: &str) -> Result<SyncLock> {
    let top = top_mapping(raw).map_err(Error::msg)?;
    let files = list_at(&top, "files")
        .into_iter()
        .filter_map(|entry| {
            let m = entry.as_mapping()?;
            let path = str_at(m, "path")?;
            let hash = str_at(m, "upstream_sha256")?;
            Some(LockEntry {
                path,
                kind: SyncKind::parse(&str_at(m, "kind").unwrap_or_else(|| "concept".to_string())),
                upstream_sha256: hash,
            })
        })
        .collect();
    Ok(SyncLock {
        base_commit: str_at(&top, "base_commit").unwrap_or_default(),
        imported_at: str_at(&top, "imported_at").unwrap_or_default(),
        files,
    })
}

pub fn render_lock(lock: &SyncLock) -> String {
    let mut sb = String::new();
    sb.push_str("# Generated by `kb sync pull`. Do not edit by hand.\n");
    sb.push_str(&format!("base_commit: {}\n", lock.base_commit));
    sb.push_str(&format!("imported_at: {}\n", lock.imported_at));
    sb.push_str("files:\n");
    let mut sorted: Vec<&LockEntry> = lock.files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    for e in sorted {
        // Quoted through `yaml_str`, which quotes only what would otherwise change
        // meaning. A comma is a legal filename byte and used to end the flow entry
        // early — `docs/a,b.md` read back as `docs/a`, a phantom the pruner would
        // act on while the real file stayed untracked — and `:`, `{` or `}` made
        // the lockfile fail to parse at all.
        //
        // Quoting only when needed is what keeps this safe to change: an ordinary
        // path renders exactly as it did before, so `sync.lock.yaml`, which is
        // committed, sees no diff from a no-op pull, and we stay byte-identical
        // with the Scala `renderLock` (`KbSync.scala`) for every realistic path.
        sb.push_str(&format!(
            "  - {{ path: {}, kind: {}, upstream_sha256: {} }}\n",
            yaml_str(&e.path),
            yaml_str(e.kind.label()),
            yaml_str(&e.upstream_sha256)
        ));
    }
    sb
}
