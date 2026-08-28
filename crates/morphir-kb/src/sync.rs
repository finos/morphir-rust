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

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use chrono::NaiveDate;
use regex::Regex;
use serde::Serialize;
use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};

use morphir_okf::{Bundle, Finding, Frontmatter, Kb, OkfProfile, Severity, parse_frontmatter};

use crate::error::{Error, Result};
use crate::util::{PathFault, contained_relative, path_fault, resolves_inside, yaml_str};

// --------------------------------------------------------------------- model

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
    fn new(verb: &str, path: &str, detail: &str) -> SyncAction {
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
    /// [`safe_relative`] and [`validated_root`] read the strings; this reads the
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

// ---------------------------------------------------------------------- glob

static GLOB_CACHE: LazyLock<Mutex<HashMap<String, Regex>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Glob matching over `/`-separated relative paths. Supports `*`, `?` and `**`.
pub fn glob_matches(glob: &str, path: &str) -> bool {
    let regex = {
        let mut cache = GLOB_CACHE.lock().expect("glob cache poisoned");
        cache
            .entry(glob.to_string())
            .or_insert_with(|| compile_glob(glob))
            .clone()
    };
    regex.is_match(path)
}

fn compile_glob(glob: &str) -> Regex {
    let chars: Vec<char> = glob.chars().collect();
    let mut sb = String::from("^");
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            // `**/` also matches zero directories, so `docs/**/x.md` finds `docs/x.md`.
            if i + 2 < chars.len() && chars[i + 2] == '/' {
                sb.push_str("(?:.*/)?");
                i += 3;
            } else {
                sb.push_str(".*");
                i += 2;
            }
        } else {
            i += 1;
            match c {
                '*' => sb.push_str("[^/]*"),
                '?' => sb.push_str("[^/]"),
                ch if "\\.+()^$|{}[]".contains(ch) => {
                    sb.push('\\');
                    sb.push(ch);
                }
                ch => sb.push(ch),
            }
        }
    }
    sb.push('$');
    Regex::new(&sb).expect("compiled glob is a valid regex")
}

// ---------------------------------------------------------------- projection

pub const FENCE_BEGIN: &str = "# kb:begin";
pub const FENCE_END: &str = "# kb:end";
const FENCE_NOTE: &str = " — added by the knowledge base; removed on export";

/// Written into the opening fence when the frontmatter block itself is ours.
///
/// Without it, a document whose upstream frontmatter is an *empty* `---` / `---`
/// pair is indistinguishable from one that had no frontmatter at all, and export
/// would delete a block upstream actually has.
const WHOLE_BLOCK_FLAG: &str = "block";

/// The generated region in a sync bundle's index.
pub const INDEX_MARKER: &str = "<!-- kb:sources -->";

pub const MANIFEST_NAME: &str = "sync.yaml";
pub const LOCK_NAME: &str = "sync.lock.yaml";

/// A document split at its frontmatter fences, preserving every byte:
/// `open + fm + close + body == text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Split {
    pub open: String,
    pub fm: String,
    pub close: String,
    pub body: String,
}

/// Splits without normalizing line endings.
///
/// `morphir_okf::split_frontmatter` converts CRLF to LF, which is right for
/// parsing and fatal for round-tripping — a CRLF file would come back from
/// [`project`] with different bytes than it went in with.
pub fn split(text: &str) -> Option<Split> {
    let open = if text.starts_with("---\r\n") {
        "---\r\n"
    } else if text.starts_with("---\n") {
        "---\n"
    } else {
        return None;
    };
    let rest = &text[open.len()..];
    fence_at(rest).map(|(start, len)| Split {
        open: open.to_string(),
        fm: rest[..start].to_string(),
        close: rest[start..start + len].to_string(),
        body: rest[start + len..].to_string(),
    })
}

/// Offset and length of the first line that is exactly `---`, including its line
/// terminator.
fn fence_at(s: &str) -> Option<(usize, usize)> {
    let mut idx = 0;
    while idx <= s.len() {
        let nl = s[idx..].find('\n').map(|n| idx + n);
        let line_end = nl.unwrap_or(s.len());
        if s[idx..line_end].trim() == "---" {
            let end = match nl {
                Some(n) => n + 1,
                None => line_end,
            };
            return Some((idx, end - idx));
        }
        idx = nl? + 1;
    }
    None
}

fn eol_of(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// Splits into lines, each keeping its terminator, so concatenation rebuilds the
/// input exactly.
fn lines_keeping_eol(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut idx = 0;
    while idx < s.len() {
        let end = s[idx..].find('\n').map(|n| idx + n + 1).unwrap_or(s.len());
        out.push(&s[idx..end]);
        idx = end;
    }
    out
}

/// Adds the knowledge base's own frontmatter keys inside a fenced region.
///
/// When the document has no frontmatter at all, the whole block is ours and
/// [`project`] removes all of it.
pub fn inject(text: &str, keys: &[(String, String)]) -> String {
    let eol = eol_of(text);
    let block = |whole: bool| -> String {
        let open = if whole {
            format!("{FENCE_BEGIN} {WHOLE_BLOCK_FLAG}{FENCE_NOTE}")
        } else {
            format!("{FENCE_BEGIN}{FENCE_NOTE}")
        };
        let mut lines = vec![open];
        lines.extend(keys.iter().map(|(k, v)| format!("{k}: {v}")));
        lines.push(FENCE_END.to_string());
        lines
            .into_iter()
            .map(|l| l + eol)
            .collect::<Vec<_>>()
            .concat()
    };
    match split(text) {
        Some(s) => format!("{}{}{}{}{}", s.open, s.fm, block(false), s.close, s.body),
        None => format!("---{eol}{}---{eol}{text}", block(true)),
    }
}

/// Removes the fenced region, yielding the upstream form. `Err` when the fence is
/// damaged.
pub fn project(text: &str) -> std::result::Result<String, String> {
    let Some(s) = split(text) else {
        return Ok(text.to_string());
    };
    let lines = lines_keeping_eol(&s.fm);
    let b = lines.iter().position(|l| l.trim().starts_with(FENCE_BEGIN));
    let e = lines.iter().position(|l| l.trim() == FENCE_END);
    match (b, e) {
        (None, None) => Ok(text.to_string()),
        (None, Some(_)) => Err(format!("{FENCE_END} without {FENCE_BEGIN}")),
        (Some(_), None) => Err(format!("{FENCE_BEGIN} without {FENCE_END}")),
        (Some(b), Some(e)) if e < b => Err("kb fence closes before it opens".to_string()),
        (Some(b), Some(e)) => {
            let kept: Vec<&str> = lines[..b]
                .iter()
                .chain(lines[e + 1..].iter())
                .copied()
                .collect();
            let whole_block = lines[b]
                .trim()
                .strip_prefix(FENCE_BEGIN)
                .map(str::trim)
                .is_some_and(|rest| rest.starts_with(WHOLE_BLOCK_FLAG));
            // The block goes only when we created it *and* nobody has since added a
            // key of their own to it.
            if whole_block && kept.is_empty() {
                Ok(s.body)
            } else {
                Ok(format!("{}{}{}{}", s.open, kept.concat(), s.close, s.body))
            }
        }
    }
}

/// Frontmatter keys the injection owns.
///
/// Fixed rather than derived from a particular file's [`injected_keys`], because
/// which of them apply changes with upstream: the day upstream adds a `title` of
/// its own, ours has to go or the frontmatter carries the key twice and stops
/// parsing. Anything inside the fence that is *not* one of these was put there by
/// hand and survives.
pub const GENERATED_KEYS: [&str; 4] = ["type", "title", "description", "kb_upstream"];

fn is_generated(line: &str) -> bool {
    match line.find([':', '\n', '\r']) {
        Some(pos) if line[pos..].starts_with(':') => GENERATED_KEYS.contains(&&line[..pos]),
        _ => false,
    }
}

/// Rewrites the fenced region to `keys`, keeping every line in it the injection
/// does not own.
///
/// The counterpart to [`inject`] for a file already on disk: same result, but
/// hand-added keys stay. A file with no fence gets one, which is what a failed or
/// lost injection needs. `Err` when the fence is damaged, on the same terms as
/// [`project`] — a file we cannot take apart is one we must not write back.
pub fn reinject(text: &str, keys: &[(String, String)]) -> std::result::Result<String, String> {
    let Some(s) = split(text) else {
        return Ok(inject(text, keys));
    };
    let lines = lines_keeping_eol(&s.fm);
    let b = lines.iter().position(|l| l.trim().starts_with(FENCE_BEGIN));
    let e = lines.iter().position(|l| l.trim() == FENCE_END);
    match (b, e) {
        (None, None) => Ok(inject(text, keys)),
        (None, Some(_)) => Err(format!("{FENCE_END} without {FENCE_BEGIN}")),
        (Some(_), None) => Err(format!("{FENCE_BEGIN} without {FENCE_END}")),
        (Some(b), Some(e)) if e < b => Err("kb fence closes before it opens".to_string()),
        (Some(b), Some(e)) => {
            let eol = eol_of(text);
            // The opening line is kept verbatim so the `block` flag survives: it
            // records whether the whole frontmatter block is ours, which is a fact
            // about upstream and not something re-injection may re-decide.
            let hand_added: String = lines[b + 1..e]
                .iter()
                .filter(|l| !is_generated(l))
                .copied()
                .collect();
            let keyed: String = keys.iter().map(|(k, v)| format!("{k}: {v}{eol}")).collect();
            let block = format!("{}{}{}{}", lines[b], keyed, hand_added, lines[e]);
            Ok(format!(
                "{}{}{}{}{}{}",
                s.open,
                lines[..b].concat(),
                block,
                lines[e + 1..].concat(),
                s.close,
                s.body
            ))
        }
    }
}

/// A mirrored concept as the manifest now implies it: upstream's own bytes, with
/// the injected block recomputed.
///
/// This is what makes the manifest self-correcting. State comparison works on
/// projected forms, so the injected block is invisible to it by construction —
/// right for detecting upstream drift, and the reason nothing used to notice when
/// our own injection went stale. Comparing a file against this closes that gap
/// without a second hash.
pub fn reinjected(
    manifest: &SyncManifest,
    rel: &str,
    text: &str,
) -> std::result::Result<String, String> {
    let upstream = project(text)?;
    reinject(text, &injected_keys(manifest, rel, &upstream))
}

/// True when the file on disk is not what the manifest would now produce. Damaged
/// fences say no: they are already reported as `unreadable`, and rewriting one
/// would mean guessing at what it was meant to hold.
pub fn injection_stale(manifest: &SyncManifest, rel: &str, text: &str) -> bool {
    reinjected(manifest, rel, text).is_ok_and(|out| out != text)
}

/// The keys the knowledge base injects: `type` always, plus whatever OKF needs and
/// upstream did not supply.
pub fn injected_keys(manifest: &SyncManifest, path: &str, upstream: &str) -> Vec<(String, String)> {
    let fm = split(upstream)
        .and_then(|s| parse_frontmatter(&s.fm).ok())
        .unwrap_or_else(Frontmatter::empty);
    let name = path.rsplit('/').next().unwrap_or(path);
    let name = name.strip_suffix(".md").unwrap_or(name);
    let fallback_title = name
        .split(['-', '_'])
        .filter(|t| !t.is_empty())
        .map(capitalize)
        .collect::<Vec<_>>()
        .join(" ");
    let mut keys = vec![("type".to_string(), manifest.type_for(path))];
    if fm.title().is_none() {
        keys.push(("title".to_string(), yaml_str(&fallback_title)));
    }
    if fm.description().is_none() {
        keys.push((
            "description".to_string(),
            yaml_str(&format!(
                "Upstream source document {}:{}.",
                manifest.repo, path
            )),
        ));
    }
    keys.push(("kb_upstream".to_string(), path.to_string()));
    keys
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// ---------------------------------------------------------------------- yaml

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

// ----------------------------------------------------------------------- io

pub fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn write_bytes(p: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(p, bytes)?;
    Ok(())
}

fn delete_file(p: &Path) -> Result<()> {
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

// -------------------------------------------------------------------- status

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
struct LocalCopy {
    upstream_form: std::result::Result<Vec<u8>, String>,
    text: Option<String>,
}

/// Reads a mirrored file into its [`LocalCopy`], or `None` when the mirror does
/// not have it.
fn local_copy_of(sb: &SyncBundle, rel: &str, kind: SyncKind) -> Result<Option<LocalCopy>> {
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

fn upstream_form_of(
    sb: &SyncBundle,
    rel: &str,
    kind: SyncKind,
) -> Result<Option<std::result::Result<Vec<u8>, String>>> {
    Ok(local_copy_of(sb, rel, kind)?.map(|c| c.upstream_form))
}

pub fn status(sb: &SyncBundle, upstream_root: Option<&Path>) -> Result<Vec<FileStatus>> {
    let upstream_rels: Vec<String> = match upstream_root {
        None => Vec::new(),
        Some(r) => relative_files_under(r)?
            .into_iter()
            .filter(|rel| sb.manifest.selects(rel))
            .collect(),
    };
    let mut upstream_hashes: HashMap<String, String> = HashMap::new();
    if let Some(r) = upstream_root {
        for rel in &upstream_rels {
            let bytes = fs::read(resolve(r, rel))?;
            upstream_hashes.insert(rel.clone(), sha256(&bytes));
        }
    }
    let mut paths: Vec<String> = sb
        .lock
        .files
        .iter()
        .map(|e| e.path.clone())
        .chain(upstream_rels.iter().cloned())
        .collect();
    paths.sort();
    paths.dedup();
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

// ---------------------------------------------------------------------- pull

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

// ---------------------------------------------------------------------- push

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

// ---------------------------------------------------------------------- diff

/// The `git diff --no-index` between the upstream copy of `rel` and the projected
/// form of the local copy — the thing an export would send. Empty output means the
/// two are identical.
pub fn diff(sb: &SyncBundle, upstream_root: &Path, rel: &str) -> Result<String> {
    let local = fs::read(sb.mirror_file(rel)?)?;
    let text = String::from_utf8_lossy(&local).into_owned();
    let projected = project(&text).map_err(|err| Error::msg(format!("{rel}: {err}")))?;
    let tmp = std::env::temp_dir().join(format!("kb-sync-{}", rel.replace('/', "_")));
    write_bytes(&tmp, projected.as_bytes())?;
    let out = std::process::Command::new("git")
        .arg("diff")
        .arg("--no-index")
        .arg("--")
        .arg(resolve(upstream_root, rel))
        .arg(&tmp)
        .output()
        .map_err(|_| Error::msg("git diff failed — is git on PATH?"))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

// -------------------------------------------------------------- index region

/// Rewrites the bundle index below the marker, grouped by the mirror's top-level
/// directories. Generated because forty hand-written bullets rot on the first
/// `pull`. Returns whether the file changed.
pub fn generate_index(sb: &SyncBundle, lock: &SyncLock, today: NaiveDate) -> Result<bool> {
    let file = &sb.bundle.index.file;
    let text = fs::read_to_string(file)?;
    let normalized = text.replace("\r\n", "\n");
    let preamble = match normalized.find(INDEX_MARKER) {
        Some(at) => normalized[..at + INDEX_MARKER.len()].to_string(),
        None => format!("{}\n\n{INDEX_MARKER}", normalized.trim_end()),
    };
    let updated = format!("{preamble}\n\n{}", render_index_body(sb, lock, today));
    if updated == normalized {
        Ok(false)
    } else {
        fs::write(file, updated)?;
        Ok(true)
    }
}

fn render_index_body(sb: &SyncBundle, lock: &SyncLock, today: NaiveDate) -> String {
    let mut sbuf = String::new();
    // The lockfile's date, not today's: this region has to be stable across a pull
    // that imports nothing, or the index churns on every run exactly as the
    // lockfile used to.
    let built = if lock.imported_at.is_empty() {
        today.to_string()
    } else {
        lock.imported_at.clone()
    };
    sbuf.push_str(&format!(
        "_Generated by `kb sync pull` — do not edit below the marker. Last built {built}"
    ));
    if !lock.base_commit.is_empty() {
        let short: String = lock.base_commit.chars().take(8).collect();
        sbuf.push_str(&format!(", from {}@{short}", sb.manifest.repo));
    }
    sbuf.push_str("._\n");
    let mut concepts: Vec<&LockEntry> = lock
        .files
        .iter()
        .filter(|e| e.kind == SyncKind::Concept)
        .collect();
    concepts.sort_by(|a, b| a.path.cmp(&b.path));
    let mut assets: Vec<&LockEntry> = lock
        .files
        .iter()
        .filter(|e| e.kind == SyncKind::Asset)
        .collect();
    assets.sort_by(|a, b| a.path.cmp(&b.path));
    let mut groups: BTreeMap<String, Vec<&LockEntry>> = BTreeMap::new();
    for e in concepts {
        let group = e.path.split('/').take(3).collect::<Vec<_>>().join("/");
        groups.entry(group).or_default().push(e);
    }
    for (group, items) in groups {
        sbuf.push_str(&format!("\n## {group}\n\n"));
        for e in items {
            let link = format!("/{}/{}", sb.manifest.root, e.path);
            // The bullet text must equal the concept's own `description` verbatim —
            // that equality is what `index-description-drift` enforces, and
            // generating the text from anything else would break it on import.
            let doc = sb.bundle.concept_at(&link);
            let title = doc
                .map(|d| d.display_title())
                .unwrap_or_else(|| e.path.rsplit('/').next().unwrap_or(&e.path).to_string());
            let description = doc
                .and_then(|d| d.fm().description())
                .map(|d| d.trim().to_string())
                .unwrap_or_else(|| {
                    format!("Upstream source document {}:{}.", sb.manifest.repo, e.path)
                });
            sbuf.push_str(&format!("* [{title}]({link}) - {description}\n"));
        }
    }
    if !assets.is_empty() {
        sbuf.push_str(&format!("\n## Assets ({})\n\n", assets.len()));
        sbuf.push_str(
            "Mirrored byte-for-byte and not concept documents — schemas, fixtures, examples, sidebar metadata.\n\n",
        );
        for e in &assets {
            sbuf.push_str(&format!("* `{}/{}`\n", sb.manifest.root, e.path));
        }
    }
    sbuf
}

// -------------------------------------------------------------------- checks

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

// ----------------------------------------------------------------- rendering

#[derive(Serialize)]
struct StatusFileJson<'a> {
    path: &'a str,
    kind: &'static str,
    state: &'static str,
    detail: &'a str,
    #[serde(rename = "injectionStale")]
    injection_stale: bool,
}

#[derive(Serialize)]
struct StatusJson<'a> {
    files: Vec<StatusFileJson<'a>>,
    summary: BTreeMap<&'static str, usize>,
}

pub fn render_status(rows: &[FileStatus], json: bool, verbose: bool) -> String {
    if json {
        let payload = StatusJson {
            files: rows
                .iter()
                .map(|r| StatusFileJson {
                    path: &r.path,
                    kind: r.kind.label(),
                    state: r.state.label(),
                    detail: &r.detail,
                    injection_stale: r.injection_stale,
                })
                .collect(),
            summary: state_counts(rows),
        };
        serde_json::to_string_pretty(&payload).expect("status serializes") + "\n"
    } else {
        let mut sbuf = String::new();
        // A stale block is interesting whatever the state says: it is the one
        // thing `state_of` cannot see, so leaving it out of an otherwise clean
        // listing is exactly how a manifest edit goes unnoticed.
        let interesting: Vec<&FileStatus> = rows
            .iter()
            .filter(|r| r.state != SyncState::Clean || r.injection_stale)
            .collect();
        let mut shown: Vec<&FileStatus> = if verbose {
            rows.iter().collect()
        } else {
            interesting
        };
        if shown.is_empty() {
            sbuf.push_str(&format!("{} file(s), all clean\n", rows.len()));
        } else {
            shown.sort_by(|a, b| {
                (a.state.label(), a.path.as_str()).cmp(&(b.state.label(), b.path.as_str()))
            });
            for r in shown {
                sbuf.push_str(&format!("{:<17} {}", r.state.label(), r.path));
                if r.injection_stale {
                    sbuf.push_str("  [injection stale]");
                }
                if !r.detail.is_empty() {
                    sbuf.push_str(&format!("  — {}", r.detail));
                }
                sbuf.push('\n');
            }
            sbuf.push('\n');
            for (label, count) in state_counts(rows) {
                sbuf.push_str(&format!("{label}: {count}\n"));
            }
            let stale = rows.iter().filter(|r| r.injection_stale).count();
            if stale > 0 {
                sbuf.push_str(&format!("injection stale: {stale}\n"));
            }
        }
        sbuf
    }
}

fn state_counts(rows: &[FileStatus]) -> BTreeMap<&'static str, usize> {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for r in rows {
        *counts.entry(r.state.label()).or_default() += 1;
    }
    counts
}

#[derive(Serialize)]
struct ActionJson<'a> {
    verb: &'a str,
    path: &'a str,
    detail: &'a str,
}

#[derive(Serialize)]
struct RefusedJson<'a> {
    path: &'a str,
    state: &'static str,
    detail: &'a str,
}

#[derive(Serialize)]
struct ActionsJson<'a> {
    #[serde(rename = "dryRun")]
    dry_run: bool,
    actions: Vec<ActionJson<'a>>,
    refused: Vec<RefusedJson<'a>>,
}

pub fn render_actions(
    actions: &[SyncAction],
    refused: &[FileStatus],
    dry_run: bool,
    json: bool,
) -> String {
    if json {
        let payload = ActionsJson {
            dry_run,
            actions: actions
                .iter()
                .map(|a| ActionJson {
                    verb: &a.verb,
                    path: &a.path,
                    detail: &a.detail,
                })
                .collect(),
            refused: refused
                .iter()
                .map(|r| RefusedJson {
                    path: &r.path,
                    state: r.state.label(),
                    detail: &r.detail,
                })
                .collect(),
        };
        serde_json::to_string_pretty(&payload).expect("actions serialize") + "\n"
    } else {
        let mut sbuf = String::new();
        if actions.is_empty() && refused.is_empty() {
            sbuf.push_str("nothing to do\n");
        }
        let mut by_verb: BTreeMap<&str, Vec<&SyncAction>> = BTreeMap::new();
        for a in actions {
            by_verb.entry(&a.verb).or_default().push(a);
        }
        for (verb, mut items) in by_verb {
            let label = if dry_run {
                format!("would {verb}")
            } else {
                verb.to_string()
            };
            sbuf.push_str(&format!("{label} ({})\n", items.len()));
            items.sort_by(|a, b| a.path.cmp(&b.path));
            for a in items {
                sbuf.push_str(&format!("  {}\n", a.path));
            }
        }
        if !refused.is_empty() {
            sbuf.push_str(&format!(
                "\nrefused ({}) — resolve by hand, or re-run with --theirs to take upstream\n",
                refused.len()
            ));
            let mut sorted: Vec<&FileStatus> = refused.iter().collect();
            sorted.sort_by(|a, b| a.path.cmp(&b.path));
            for r in sorted {
                sbuf.push_str(&format!("  {}  [{}]", r.path, r.state.label()));
                if !r.detail.is_empty() {
                    sbuf.push_str(&format!(" — {}", r.detail));
                }
                sbuf.push('\n');
            }
        }
        sbuf
    }
}
