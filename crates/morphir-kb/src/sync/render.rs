use std::collections::BTreeMap;

use serde::Serialize;

use super::diff::{DiffResult, DiffSelection};
use super::model::{FileStatus, SyncAction, SyncState};

/// The human rendering: `<path>: identical`, or git's unified diff verbatim.
/// Byte-for-byte what the CLI printed before this became a value.
pub fn render_diff_text(d: &DiffResult) -> String {
    if d.identical {
        format!("{}: identical\n", d.path)
    } else {
        d.diff.clone()
    }
}

/// The `--json` rendering: a pretty object carrying the path, the verdict, the
/// human diff and the applicable patch, in the shape the rest of this module
/// emits.
pub fn render_diff_json(d: &DiffResult) -> String {
    serde_json::to_string_pretty(d).expect("a diff result serializes") + "\n"
}

/// The `--raw` rendering: the patch bytes git produced, undecorated, ready for
/// `git apply` in the upstream checkout. An identical pair yields nothing at
/// all — an empty patch is the honest answer, and a `<path>: identical` line
/// here would corrupt the pipe.
pub fn render_diff_raw(d: &DiffResult) -> String {
    d.patch.clone()
}

/// A [`DiffSet`] as `--json` prints it: the same records the single-file payload
/// carries, inside the `{collection, summary}` envelope `sync status` uses.
///
/// An array on its own would have been enough for the files, but a bare array
/// cannot say that eleven paths were compared and none of them differed — which
/// is exactly the answer a reader is most likely to doubt.
#[derive(Serialize)]
struct DiffSetJson<'a> {
    files: &'a [DiffResult],
    summary: DiffSummaryJson,
}

#[derive(Serialize)]
struct DiffSummaryJson {
    differing: usize,
    compared: usize,
    absent: usize,
}

/// One `=== <path> ===` heading per file. The human diff underneath names a
/// checkout path and a scratch path, neither of which is the mirrored path, so
/// without this a multi-file diff would not say which file it was showing.
fn diff_section(path: &str) -> String {
    format!("=== {path} ===\n")
}

/// Git's output always ends in a newline, including after a
/// `\ No newline at end of file` line. Enforced anyway: an unterminated patch
/// would run into the next file's `diff --git` header and take it with it.
fn newline_terminated(s: &str) -> String {
    if s.is_empty() || s.ends_with('\n') {
        s.to_string()
    } else {
        format!("{s}\n")
    }
}

/// The human rendering of a whole selection. A single file renders exactly as
/// [`render_diff_text`] has always rendered it; a set renders one section per
/// differing file, then the tally.
pub fn render_diffs_text(sel: &DiffSelection) -> String {
    match sel {
        DiffSelection::Single(d) => render_diff_text(d),
        DiffSelection::Many(set) => {
            // The tally counts what was actually compared. A lockfile entry
            // absent on both sides was passed over, and saying it was compared
            // and found equal would be a false statement — the one reading it
            // while debugging a missing hunk would be misled precisely when it
            // matters. Absent paths get their own clause, pointing at the tool
            // whose job they are.
            let absent_note = if set.absent > 0 {
                format!(
                    "; {} listed in the lockfile absent on both sides — see `kb sync status`",
                    set.absent
                )
            } else {
                String::new()
            };
            if set.files.is_empty() {
                if set.compared() == 0 && set.absent > 0 {
                    return format!(
                        "{} path(s) matched, none present on either side — see `kb sync status`\n",
                        set.absent
                    );
                }
                return format!(
                    "{} file(s) compared, no differences{absent_note}\n",
                    set.compared()
                );
            }
            let mut sbuf = String::new();
            for d in &set.files {
                sbuf.push_str(&diff_section(&d.path));
                sbuf.push_str(&newline_terminated(&d.diff));
            }
            sbuf.push_str(&format!(
                "\n{} of {} file(s) differ{absent_note}\n",
                set.files.len(),
                set.compared()
            ));
            sbuf
        }
    }
}

/// The `--json` rendering of a whole selection. A single file is the bare object
/// [`render_diff_json`] emits, unchanged, so anything already reading it keeps
/// working; a set is the envelope above.
pub fn render_diffs_json(sel: &DiffSelection) -> String {
    match sel {
        DiffSelection::Single(d) => render_diff_json(d),
        DiffSelection::Many(set) => {
            let payload = DiffSetJson {
                files: &set.files,
                summary: DiffSummaryJson {
                    differing: set.files.len(),
                    compared: set.compared(),
                    absent: set.absent,
                },
            };
            serde_json::to_string_pretty(&payload).expect("a diff set serializes") + "\n"
        }
    }
}

/// The `--raw` rendering of a whole selection: the patches concatenated in
/// mirrored-path order and nothing else, which is a multi-file patch `git apply`
/// takes in one go. Nothing differing means no output at all.
pub fn render_diffs_raw(sel: &DiffSelection) -> String {
    match sel {
        DiffSelection::Single(d) => render_diff_raw(d),
        DiffSelection::Many(set) => set
            .files
            .iter()
            .map(|d| newline_terminated(&d.patch))
            .collect(),
    }
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
