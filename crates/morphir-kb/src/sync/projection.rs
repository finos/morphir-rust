use morphir_okf::{Frontmatter, parse_frontmatter};

use crate::util::yaml_str;

use super::model::SyncManifest;

pub const FENCE_BEGIN: &str = "# kb:begin";
pub const FENCE_END: &str = "# kb:end";
const FENCE_NOTE: &str = " — added by the knowledge base; removed on export";

/// Written into the opening fence when the frontmatter block itself is ours.
///
/// Without it, a document whose upstream frontmatter is an *empty* `---` / `---`
/// pair is indistinguishable from one that had no frontmatter at all, and export
/// would delete a block upstream actually has.
const WHOLE_BLOCK_FLAG: &str = "block";

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
