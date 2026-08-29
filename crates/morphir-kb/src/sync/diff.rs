use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

use crate::error::{Error, Result};

use super::files::{resolve, safe_relative, write_bytes};
use super::glob::glob_matches;
use super::model::SyncBundle;
use super::status::{known_paths, local_copy_of};

/// Distinguishes concurrent diffs within one process; the process id alone
/// does not separate two threads.
static DIFF_SEQ: AtomicU64 = AtomicU64::new(0);

/// The outcome of a [`diff`]: which mirrored path was compared, whether the two
/// sides agree, and the unified diff when they do not.
///
/// Returned rather than printed so the CLI can honour `--json`. The Scala tool
/// prints from inside the diff operation and so has nothing left to serialize;
/// `--json` there yields unparseable stdout. Everything the text renderer needs
/// is here, so both forms derive from one value and cannot drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffResult {
    /// The mirrored path, exactly as it was asked for.
    pub path: String,
    /// True when the projected local copy and the upstream copy agree.
    pub identical: bool,
    /// `git diff`'s unified output over the two files where they actually sit,
    /// so its headers name a checkout path and a scratch path. This is what the
    /// CLI has always shown a human; empty when the two sides are identical.
    pub diff: String,
    /// The same change as a patch whose headers read `a/<path>` and `b/<path>`,
    /// so `git apply` lands it in the upstream repository. Empty when the two
    /// sides are identical.
    pub patch: String,
}

/// The `git diff --no-index` between the upstream copy of `rel` and the projected
/// form of the local copy — the thing an export would send. Empty output means the
/// two are identical.
///
/// Git runs twice, over one staging directory, because the two outputs answer
/// different questions. The human diff names the files where they really are,
/// which is what someone reading the terminal wants. The patch has to name
/// `a/<rel>` and `b/<rel>` or it applies nowhere, and the only way to get those
/// headers without rewriting git's output by hand — which would break on the
/// first filename holding a space — is to hand git paths that already have the
/// shape it should print.
///
/// A side that is not there is compared against nothing rather than passed over:
/// a file deleted upstream diffs as an addition and carries a patch that restores
/// it, one deleted here diffs as a removal, and a path neither side holds is
/// refused by name. `rel` itself is checked for containment first, as `pull` and
/// `push` check every path they touch.
pub fn diff(sb: &SyncBundle, upstream_root: &Path, rel: &str) -> Result<DiffResult> {
    // The guard `pull` and `push` already put in front of every path they act
    // on. Without it a `rel` carrying `..` reached the staging writes below,
    // and a mirror root a few directories deep absorbs enough of those segments
    // for `mirror_file` to call the path contained — while `scratch/a` and
    // `scratch/b`, one directory down rather than several, do not: the copy and
    // the write landed outside the scratch tree, and `create_dir_all` made the
    // directories to receive them. Tightening `mirror_file` cannot close this,
    // because its containment root is legitimately the deeper of the two.
    if !safe_relative(rel) {
        return Err(Error::msg(format!(
            "`{rel}` leaves the mirror — diff a path relative to the mirror root, \
             e.g. `docs/types.md`, with no leading separator and no `.` or `..` segments"
        )));
    }
    let upstream_file = resolve(upstream_root, rel);
    // The same reading `status` takes of the same file, rather than a second one
    // of its own: bytes for an asset, the projected text for a concept. Reading
    // every file as text meant an asset that is not valid UTF-8 came back with
    // U+FFFD where its bytes had been, and a freshly pulled binary then diffed
    // against itself as a change. Nothing shows that up on one named `.md`; a
    // diff over the whole mirror walks into it on the first image.
    //
    // Either side may be absent, and both absences are states `sync status`
    // already has names for: `deleted-upstream-edited` when the checkout has
    // dropped a file the mirror still holds an edit of, `missing-local` the
    // other way round. Modelled here rather than left to fail — the first used
    // to make git complain into a dropped stderr and hand back an empty diff,
    // which then read as "identical", and the second died on an `fs::read` that
    // named nothing.
    let kind = sb
        .lock
        .get(rel)
        .map(|e| e.kind)
        .unwrap_or_else(|| sb.manifest.kind_of(rel));
    let projected: Option<Vec<u8>> = match local_copy_of(sb, rel, kind)? {
        Some(copy) => Some(
            copy.upstream_form
                .map_err(|err| Error::msg(format!("{rel}: {err}")))?,
        ),
        None => None,
    };
    let upstream = upstream_file.is_file().then_some(upstream_file.as_path());
    if projected.is_none() && upstream.is_none() {
        return Err(Error::msg(format!(
            "`{rel}` is in neither the mirror nor the upstream checkout — \
             name a path one side still holds, e.g. `docs/types.md`"
        )));
    }
    // A scratch directory unique to this call. A fixed name under the system
    // temp directory was a shared mutable file: two diffs running at once —
    // two shells, or two tests — would compare upstream against whichever
    // projection landed last.
    let scratch = std::env::temp_dir().join(format!(
        "kb-sync-{}-{}",
        std::process::id(),
        DIFF_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let outcome = diff_in(&scratch, upstream, rel, projected.as_deref());
    // Best effort: scratch files left behind are untidy, never wrong, and any
    // git failure above is the more interesting thing to report.
    let _ = fs::remove_dir_all(&scratch);
    let (diff, patch) = outcome?;
    // Whitespace-only output still means "no change"; git emits nothing at
    // all for a match, but callers should not have to rely on that.
    let identical = diff.trim().is_empty();
    Ok(DiffResult {
        path: rel.to_string(),
        identical,
        diff: if identical { String::new() } else { diff },
        patch: if identical { String::new() } else { patch },
    })
}

/// The name git reads as "there was nothing on this side" in `--no-index`.
/// Git special-cases the string itself, so it means the same on Windows as it
/// does here and never reaches a filesystem.
const NULL_PATH: &str = "/dev/null";

/// Both renderings of the same comparison, staged under `scratch`.
///
/// `upstream` is `None` when the checkout no longer holds the file and
/// `projected` is `None` when the mirror does not; the caller guarantees at
/// least one of them. Git is never handed a path that is not there — it refuses
/// one outright, saying so on stderr and printing nothing — so the absent side
/// is named [`NULL_PATH`] instead, which is what makes the comparison a
/// `new file mode` or `deleted file mode` patch rather than a silent failure.
///
/// Staging an empty `a/<rel>` would carry the same headers and the same content,
/// and it is the smaller change, but `git apply` refuses the result wherever the
/// file is missing — "No such file or directory" — because nothing in such a
/// patch says a file is being created. Only the `/dev/null` form lands, and a
/// patch that does not land is not worth emitting.
fn diff_in(
    scratch: &Path,
    upstream: Option<&Path>,
    rel: &str,
    projected: Option<&[u8]>,
) -> Result<(String, String)> {
    let null = Path::new(NULL_PATH);
    let flat = scratch.join(rel.replace('/', "_"));
    let projected_side = match projected {
        Some(bytes) => {
            write_bytes(&flat, bytes)?;
            flat.as_path()
        }
        None => null,
    };
    let human = git_diff(
        None,
        &[],
        upstream.unwrap_or(null).as_os_str(),
        projected_side.as_os_str(),
    )?;

    // The staged pair carries the relative path under an `a/` and a `b/` root.
    // With the prefixes blanked, git prints exactly `a/<rel>` and `b/<rel>` —
    // its own quoting and escaping rules intact, which is the whole point of
    // not touching the text afterwards. Where only one side exists there is
    // only one root to stage, and git's own default prefixes put those same two
    // names on it.
    let staged_a = resolve(&scratch.join("a"), rel);
    let staged_b = resolve(&scratch.join("b"), rel);
    // `--binary` on the patch side only. Without it a changed asset that is not
    // text becomes the line `Binary files a/x and b/x differ`, which carries no
    // content and which `git apply` refuses — and one such file in a multi-file
    // patch takes every other file in it down too, since `git apply` is all or
    // nothing. The human diff keeps the summary line, because a screenful of
    // base85 is not a reading of anything.
    let patch = match (upstream, projected) {
        (Some(up), Some(bytes)) => {
            fs::copy(up, ensure_parent(&staged_a)?)?;
            write_bytes(&staged_b, bytes)?;
            git_diff(
                Some(scratch),
                &["--src-prefix=", "--dst-prefix=", "--binary"],
                format!("a/{rel}").as_ref(),
                format!("b/{rel}").as_ref(),
            )?
        }
        (None, Some(bytes)) => {
            write_bytes(&staged_b, bytes)?;
            git_diff(
                Some(&scratch.join("b")),
                &["--binary"],
                null.as_os_str(),
                rel.as_ref(),
            )?
        }
        (Some(up), None) => {
            fs::copy(up, ensure_parent(&staged_a)?)?;
            git_diff(
                Some(&scratch.join("a")),
                &["--binary"],
                rel.as_ref(),
                null.as_os_str(),
            )?
        }
        // Refused by `diff` before anything is staged.
        (None, None) => String::new(),
    };
    Ok((human, patch))
}

fn ensure_parent(p: &Path) -> Result<&Path> {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(p)
}

/// One `git diff --no-index` invocation, with the two paths always behind `--`
/// so a name beginning with a dash cannot be read as an option.
///
/// `diff.noprefix` and `diff.mnemonicPrefix` are pinned off: a user's global
/// config must not decide whether the patch we hand back is applicable.
fn git_diff(cwd: Option<&Path>, opts: &[&str], old: &OsStr, new: &OsStr) -> Result<String> {
    let mut cmd = std::process::Command::new("git");
    if let Some(dir) = cwd {
        cmd.arg("-C").arg(dir);
    }
    let out = cmd
        .arg("-c")
        .arg("diff.noprefix=false")
        .arg("-c")
        .arg("diff.mnemonicPrefix=false")
        .arg("diff")
        .arg("--no-index")
        .args(opts)
        .arg("--")
        .arg(old)
        .arg(new)
        .output()
        .map_err(|_| Error::msg("git diff failed — is git on PATH?"))?;
    // `--no-index` exits 1 to say "the two files differ", which is the ordinary
    // outcome here, so the exit status cannot tell that apart from a real
    // failure — checking `status.success()` would reject every diff that found
    // something. Where the words come out can tell them apart: git writes the
    // diff to stdout and its complaints to stderr, so nothing on stdout
    // together with something on stderr is a failure. Dropping both, as this
    // used to, turned each such failure into an empty diff, and an empty diff
    // reads as "identical".
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    if stdout.is_empty() && !stderr.trim().is_empty() {
        return Err(Error::msg(format!(
            "`git diff --no-index` failed — {}",
            stderr.trim()
        )));
    }
    Ok(stdout)
}

// ------------------------------------------------------------ diff: many

/// True when `pattern` is a glob rather than a literal mirrored path.
///
/// The two metacharacters `compile_glob` acts on, and only those: `[` and `.`
/// are ordinary characters in this dialect, so a filename holding one is still
/// a literal path.
pub fn is_glob(pattern: &str) -> bool {
    pattern.contains(['*', '?'])
}

/// The files a multi-file diff found, and how many it looked at.
///
/// `files` holds only the ones whose two sides differ — a diff of everything is
/// a diff, not an inventory, and under `--raw` a file that agrees contributes no
/// bytes anyway. `matched` is what the selection reached and `absent` how many
/// of those held content on neither side, so the renderings can say that twenty
/// files were compared and none moved — and never claim a comparison for a path
/// that had nothing to compare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffSet {
    /// Differing files only, in mirrored-path order.
    pub files: Vec<DiffResult>,
    /// How many mirrored paths the selection matched, differing or not.
    pub matched: usize,
    /// Matched paths holding content on neither side — lockfile entries that
    /// outlived both copies. Passed over rather than compared; `sync status`
    /// reports them as `missing-local` / `deleted-upstream`.
    pub absent: usize,
}

impl DiffSet {
    /// How many matched paths actually had two sides to compare.
    pub fn compared(&self) -> usize {
        self.matched - self.absent
    }
}

/// What one `sync diff` invocation asked for: one named file, or a set of them.
///
/// The distinction is the argument's shape, not how many files it turned out to
/// reach: `sync diff docs/types.md` is [`DiffSelection::Single`] and everything
/// else — no argument, a glob, several patterns — is [`DiffSelection::Many`].
/// Keeping them apart is what lets the single-file renderings stay byte-for-byte
/// what they have always been while the multi-file ones are free to frame their
/// output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSelection {
    Single(DiffResult),
    Many(DiffSet),
}

impl DiffSelection {
    /// The differing files, whichever form the selection took. A `Single` that
    /// is identical still has a record — that is the case that prints
    /// `<path>: identical` — so this is not the same as "what would be printed".
    pub fn files(&self) -> &[DiffResult] {
        match self {
            DiffSelection::Single(d) => std::slice::from_ref(d),
            DiffSelection::Many(set) => &set.files,
        }
    }
}

/// Resolves `select` and diffs what it names, in the form the CLI should print.
///
/// `select` is a list because the caller is a shell: a glob is as likely to
/// arrive already expanded by the shell as intact, and `find`, `ls` and
/// `git diff --name-only` all hand over many paths at once. Empty means every
/// file the mirror knows about. Each element is either a literal mirrored path
/// or a glob in the [`glob_matches`] dialect — the one `sync.yaml` mappings are
/// written in, so there is nothing new to learn and `docs/**` means here what it
/// means there.
///
/// Exactly one literal path is the single-file case, unchanged in every respect
/// including its output. Anything else is a union: matched once, deduplicated,
/// and sorted by mirrored path, so the same patterns in any order produce the
/// same bytes.
///
/// A shell expands an unquoted glob before this ever sees it. Mirrored paths
/// rarely exist in the working directory, so the pattern usually survives
/// untouched, but quoting it — `'docs/**'` — is the only way to be sure which
/// dialect is doing the matching.
pub fn diff_selected(
    sb: &SyncBundle,
    upstream_root: &Path,
    select: &[String],
) -> Result<DiffSelection> {
    match select {
        [only] if !is_glob(only) => Ok(DiffSelection::Single(diff(sb, upstream_root, only)?)),
        _ => Ok(DiffSelection::Many(diff_many(sb, upstream_root, select)?)),
    }
}

/// Every differing file among those `select` names, in mirrored-path order.
///
/// See [`diff_selected`] for what `select` accepts. The file set comes from
/// [`known_paths`], so this and `sync status` never disagree about which files
/// exist; a pattern reaching outside that set is refused rather than silently
/// producing nothing.
pub fn diff_many(sb: &SyncBundle, upstream_root: &Path, select: &[String]) -> Result<DiffSet> {
    // Per pattern, and before anything is read or staged. A list is not a way
    // around the containment guard `diff` puts in front of a single path:
    // one `..` element among twenty good ones refuses the whole invocation.
    let escaping: Vec<&String> = select.iter().filter(|p| !safe_relative(p)).collect();
    if !escaping.is_empty() {
        return Err(Error::msg(format!(
            "{} {} the mirror — diff paths relative to the mirror root, e.g. \
             `docs/types.md` or `docs/**`, with no leading separator and no `.` or `..` segments",
            quoted(&escaping),
            if escaping.len() == 1 {
                "leaves"
            } else {
                "leave"
            }
        )));
    }
    let known = known_paths(sb, Some(upstream_root))?;
    if known.is_empty() {
        return Err(Error::msg(
            "this mirror holds no files — `kb sync pull` imports what `sync.yaml` selects, \
             and there is nothing to compare until it has",
        ));
    }
    let selected = if select.is_empty() {
        known
    } else {
        // Named individually so the refusal can be too: with twenty patterns
        // arriving down a pipe, "matched nothing" without saying which one is a
        // message that costs the reader a bisect.
        let mut unmatched: Vec<&String> = Vec::new();
        let mut chosen: Vec<String> = Vec::new();
        for pattern in select {
            let hits: Vec<&String> = known.iter().filter(|p| glob_matches(pattern, p)).collect();
            if hits.is_empty() {
                unmatched.push(pattern);
            }
            chosen.extend(hits.into_iter().cloned());
        }
        if !unmatched.is_empty() {
            return Err(Error::msg(format!(
                "{} {} no mirrored file — `kb sync status` lists every path this mirror knows \
                 about, and a pattern is matched against those; quote a glob so the shell hands \
                 it over intact, e.g. `'docs/**'`",
                quoted(&unmatched),
                if unmatched.len() == 1 {
                    "matches"
                } else {
                    "match"
                }
            )));
        }
        // Two patterns may reach the same file; it is one file either way, and
        // the sort is what makes the patch independent of the order the
        // patterns arrived in.
        chosen.sort();
        chosen.dedup();
        chosen
    };
    let matched = selected.len();
    let mut files = Vec::new();
    let mut absent = 0;
    for rel in selected {
        // A path neither side holds is refused when it is asked for by name,
        // because the asker is wrong about it. In a set it is passed over: two
        // absent sides make no hunk, and the lockfile entry that outlived them
        // both is `sync status`'s business — `missing-local`, `deleted-upstream`
        // — not a patch's. Passed over, but counted: reporting it as compared
        // would claim a comparison that never happened.
        let held_here = sb.mirror_file(&rel)?.is_file();
        let held_upstream = resolve(upstream_root, &rel).is_file();
        if !held_here && !held_upstream {
            absent += 1;
            continue;
        }
        let d = diff(sb, upstream_root, &rel)?;
        if !d.identical {
            files.push(d);
        }
    }
    Ok(DiffSet {
        files,
        matched,
        absent,
    })
}

/// `` `a`, `b` `` — the way this module names paths back to the reader.
fn quoted(items: &[&String]) -> String {
    items
        .iter()
        .map(|p| format!("`{p}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
