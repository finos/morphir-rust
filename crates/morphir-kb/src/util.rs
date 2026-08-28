//! Shared helpers ported from the small utilities duplicated across the Scala sources
//! (`KbScaffold.scala:20-26`, `KbIntentEdit.scala:52-54`, `KbSync.scala:328-330`),
//! plus the path containment both `sync` and `scaffold` need before they write.

use std::path::{Component, Path, PathBuf};

/// Quote a string for emission into YAML frontmatter when it contains characters that
/// would change its meaning unquoted, or leading/trailing spaces.
pub fn yaml_str(s: &str) -> String {
    let needs_quote = s.chars().any(|c| ":#{}[]&*!|>'\"%@`,".contains(c))
        || s.starts_with(' ')
        || s.ends_with(' ');
    if needs_quote {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Lowercase slug: non-alphanumeric runs collapse to `-`, trimmed at both ends.
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_dash = false;
    for c in s.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c);
        } else {
            pending_dash = true;
        }
    }
    out
}

// -------------------------------------------------------------- containment

/// Why a relative path is refused. Kept apart so callers can say the right
/// thing: an anchored path is a different mistake from one that climbs out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathFault {
    /// Anchored somewhere of its own: a leading separator, or a Windows drive.
    Anchored,
    /// Climbs out of, or navigates around, the directory it is relative to.
    Escapes,
}

/// True when a `\` or `/` at position 0 of `rel` would anchor it.
fn leading_separator(rel: &str) -> bool {
    rel.starts_with('/') || rel.starts_with('\\')
}

/// True when `rel` opens with a Windows drive designator — `C:\victim`, and the
/// drive-*relative* `C:victim`, which resolves against that drive's own working
/// directory and is no more contained than the first.
fn drive_designator(rel: &str) -> bool {
    let b = rel.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// What is wrong with `rel` as a path relative to a directory it must stay
/// inside, or `None` when nothing is.
///
/// The policy is *containment under every platform's reading of the path*, not
/// separator purity, and it is uniform rather than conditioned on the host. Two
/// facts force that. A `sync.yaml` root and a bundle group are committed to a
/// repository and read back wherever the tool runs — the release workflow ships
/// `x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc` — so a value that is
/// safe on Linux and an escape on Windows is a bad value on Linux too, and
/// saying so at the machine that authored it is the only place it is cheap to
/// fix. And the converse: a backslash is a legal filename character on Unix, so
/// `a\b.md` is one ordinary file there and the directory `a` holding `b.md` on
/// Windows. Different readings, both inside — nothing to refuse. What is refused
/// is what leaves: `..\`, a leading separator, and a drive designator.
///
/// The string walk below treats `\` and `/` alike, which is what makes the
/// answer the same on every platform; [`Path::components`] then re-checks the
/// same value under the running platform's own parser, so anything the walk has
/// not thought of is still caught where it would actually do harm.
pub fn path_fault(rel: &str) -> Option<PathFault> {
    if leading_separator(rel) || drive_designator(rel) {
        return Some(PathFault::Anchored);
    }
    if rel.split(['/', '\\']).any(|s| s == "." || s == "..") {
        return Some(PathFault::Escapes);
    }
    Path::new(rel).components().find_map(|c| match c {
        Component::ParentDir | Component::CurDir => Some(PathFault::Escapes),
        Component::RootDir | Component::Prefix(_) => Some(PathFault::Anchored),
        Component::Normal(_) => None,
    })
}

/// True when `rel` names something inside the directory it is relative to.
pub fn contained_relative(rel: &str) -> bool {
    !rel.is_empty() && path_fault(rel).is_none()
}

/// `p` with every symlink on it resolved, falling back to the nearest ancestor
/// that exists with the missing tail rejoined — so a path that has not been
/// created yet still resolves through the links that lead to it.
fn resolve_existing(p: &Path) -> Option<PathBuf> {
    if let Ok(c) = p.canonicalize() {
        return Some(c);
    }
    let parent = p.parent()?;
    let name = p.file_name()?;
    let base = if parent.as_os_str().is_empty() {
        std::env::current_dir().ok()?.canonicalize().ok()?
    } else {
        resolve_existing(parent)?
    };
    Some(base.join(name))
}

/// True when `target` really lands at or below `inside` once symlinks are
/// followed.
///
/// [`contained_relative`] is lexical, and a lexical check cannot see a link: a
/// mirror root named `sources` that points at `../../../victim` reads as a plain
/// name and escapes anyway, taking every read, write and delete through it with
/// it. Both sides are resolved, not just `target`, so a knowledge base reached
/// through a link of its own — `/tmp` on macOS is `/private/tmp` — still
/// compares as itself.
pub fn resolves_inside(inside: &Path, target: &Path) -> bool {
    match (resolve_existing(inside), resolve_existing(target)) {
        (Some(i), Some(t)) => t.starts_with(i),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_str_plain_stays_bare() {
        assert_eq!(yaml_str("plain words"), "plain words");
    }

    #[test]
    fn yaml_str_quotes_specials_and_escapes() {
        assert_eq!(yaml_str("a: b"), "\"a: b\"");
        assert_eq!(yaml_str("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(yaml_str("back\\slash, comma"), "\"back\\\\slash, comma\"");
        assert_eq!(yaml_str(" leading"), "\" leading\"");
        assert_eq!(yaml_str("trailing "), "\"trailing \"");
    }

    #[test]
    fn path_fault_names_the_two_ways_out() {
        assert_eq!(path_fault("/etc/passwd"), Some(PathFault::Anchored));
        assert_eq!(path_fault("\\victim"), Some(PathFault::Anchored));
        assert_eq!(path_fault("C:\\victim"), Some(PathFault::Anchored));
        assert_eq!(path_fault("C:victim"), Some(PathFault::Anchored));
        assert_eq!(path_fault("../shared"), Some(PathFault::Escapes));
        assert_eq!(path_fault("..\\shared"), Some(PathFault::Escapes));
        assert_eq!(path_fault("a/..\\b"), Some(PathFault::Escapes));
        assert_eq!(path_fault("a/./b"), Some(PathFault::Escapes));
        assert_eq!(path_fault("vendor/sources"), None);
        assert_eq!(path_fault("a\\b.md"), None, "contained on both platforms");
    }

    #[test]
    fn contained_relative_refuses_the_empty_path() {
        assert!(!contained_relative(""));
        assert!(contained_relative("sources"));
    }

    #[test]
    fn resolves_inside_follows_symlinks_on_both_sides() {
        let tmp = std::env::temp_dir().join(format!("kb-util-{}", std::process::id()));
        let inside = tmp.join("kb");
        let outside = tmp.join("victim");
        std::fs::create_dir_all(inside.join("real")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        assert!(resolves_inside(&inside, &inside.join("real/new.md")));
        assert!(!resolves_inside(&inside, &outside.join("new.md")));
        #[cfg(unix)]
        {
            let link = inside.join("linked");
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(
                !resolves_inside(&inside, &link.join("new.md")),
                "a link is not contained just because its name is"
            );
        }
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn slugify_collapses_and_trims() {
        assert_eq!(
            slugify("  OKF Knowledge  Library! "),
            "okf-knowledge-library"
        );
        assert_eq!(slugify("Morphir IR v5"), "morphir-ir-v5");
        assert_eq!(slugify("--x--"), "x");
    }
}
