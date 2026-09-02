//! Workspace member patterns: expanding `[workspace].members` into member
//! directories and deciding whether a directory is a member of a workspace.
//!
//! The matcher is deliberately small. A pattern is a `/`-separated list of
//! segments; `*` matches any run of characters inside one segment, and a whole
//! `**` segment matches zero or more segments. That is everything the
//! `packages/*` style of member list needs, so no glob dependency is pulled in.

use super::discovery::discover_config_at;
use std::path::{Component, Path, PathBuf};

/// Does this pattern contain a wildcard, so it has to be expanded against the
/// filesystem rather than joined onto the workspace root?
pub(crate) fn is_pattern(text: &str) -> bool {
    text.contains('*')
}

/// Expand one `[workspace].members` entry into member directories.
///
/// A literal entry yields exactly the joined path, whether or not it exists,
/// so a member that is listed but missing is still reported as a not-found
/// source. A wildcard entry yields only existing directories that hold a
/// Morphir configuration, sorted for a stable order.
pub(crate) fn expand_member_pattern(workspace_root: &Path, pattern: &str) -> Vec<PathBuf> {
    if !is_pattern(pattern) {
        return vec![workspace_root.join(pattern)];
    }
    let segments = pattern_segments(pattern);
    let mut found = Vec::new();
    collect_matches(workspace_root, &segments, &mut found);
    found.sort();
    found.dedup();
    found
}

/// Expand every member entry of a workspace, in the order they are listed.
pub(crate) fn expand_members(workspace_root: &Path, members: &[String]) -> Vec<PathBuf> {
    let mut expanded = Vec::new();
    for member in members {
        for path in expand_member_pattern(workspace_root, member) {
            if !expanded.contains(&path) {
                expanded.push(path);
            }
        }
    }
    expanded
}

/// Does `directory` sit at a path that one of `members` selects, relative to
/// `workspace_root`?
///
/// This answers the question without touching the filesystem, so it is safe to
/// ask while walking up from a candidate member towards its workspace.
pub(crate) fn members_select(workspace_root: &Path, members: &[String], directory: &Path) -> bool {
    let Ok(relative) = directory.strip_prefix(workspace_root) else {
        return false;
    };
    let actual = path_segments(relative);
    members.iter().any(|member| {
        let pattern = pattern_segments(member);
        segments_match(&pattern, &actual)
    })
}

fn pattern_segments(pattern: &str) -> Vec<&str> {
    pattern
        .split(['/', '\\'])
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect()
}

fn path_segments(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// Match a segment list against a pattern segment list, where `**` stands for
/// zero or more whole segments.
fn segments_match(pattern: &[&str], actual: &[String]) -> bool {
    match pattern.split_first() {
        None => actual.is_empty(),
        Some((&"**", rest)) => {
            (0..=actual.len()).any(|skipped| segments_match(rest, &actual[skipped..]))
        }
        Some((head, rest)) => match actual.split_first() {
            Some((name, tail)) if segment_matches(head, name) => segments_match(rest, tail),
            _ => false,
        },
    }
}

/// Match one path segment against one pattern segment, where `*` stands for
/// any run of characters (including none) inside that segment.
fn segment_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut at_pattern, mut at_name) = (0, 0);
    let (mut star, mut retry_from) = (None, 0);
    while at_name < name.len() {
        if at_pattern < pattern.len() && pattern[at_pattern] == name[at_name] {
            at_pattern += 1;
            at_name += 1;
        } else if at_pattern < pattern.len() && pattern[at_pattern] == '*' {
            star = Some(at_pattern);
            at_pattern += 1;
            retry_from = at_name;
        } else if let Some(star) = star {
            at_pattern = star + 1;
            retry_from += 1;
            at_name = retry_from;
        } else {
            return false;
        }
    }
    pattern[at_pattern..].iter().all(|char| *char == '*')
}

/// Walk the tree below `directory` following `segments`, collecting the
/// directories that both match and hold a Morphir configuration.
fn collect_matches(directory: &Path, segments: &[&str], found: &mut Vec<PathBuf>) {
    let Some((head, rest)) = segments.split_first() else {
        if discover_config_at(directory).ok().flatten().is_some() {
            found.push(directory.to_path_buf());
        }
        return;
    };
    if *head == "**" {
        collect_matches(directory, rest, found);
        for child in child_directories(directory) {
            collect_matches(&child, segments, found);
        }
        return;
    }
    for child in child_directories(directory) {
        let matched = child
            .file_name()
            .is_some_and(|name| segment_matches(head, &name.to_string_lossy()));
        if matched {
            collect_matches(&child, rest, found);
        }
    }
}

/// Visible subdirectories of `directory`. Hidden directories are skipped so a
/// wildcard never descends into `.git`, `.morphir`, or a build tree.
fn child_directories(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| !name.to_string_lossy().starts_with('.'))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn segment_wildcards_match_inside_one_segment() {
        assert!(segment_matches("*", "orders"));
        assert!(segment_matches("orders", "orders"));
        assert!(segment_matches("morphir-*", "morphir-orders"));
        assert!(segment_matches("*-api", "orders-api"));
        assert!(segment_matches("a*c*e", "abcde"));
        assert!(!segment_matches("orders", "billing"));
        assert!(!segment_matches("morphir-*", "orders"));
        assert!(segment_matches("*", ""));
    }

    #[test]
    fn double_star_spans_whole_segments() {
        let path = [
            "packages".to_owned(),
            "acme".to_owned(),
            "orders".to_owned(),
        ];
        assert!(segments_match(&pattern_segments("**"), &path));
        assert!(segments_match(&pattern_segments("packages/**"), &path));
        assert!(segments_match(
            &pattern_segments("packages/**/orders"),
            &path
        ));
        assert!(!segments_match(&pattern_segments("packages/*"), &path));
        assert!(segments_match(
            &pattern_segments("packages/*"),
            &["packages".to_owned(), "orders".to_owned()]
        ));
        assert!(!segments_match(&pattern_segments("libs/**"), &path));
    }

    #[test]
    fn literal_patterns_expand_even_when_missing() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            expand_member_pattern(root.path(), "packages/orders"),
            vec![root.path().join("packages").join("orders")]
        );
    }

    #[test]
    fn wildcards_expand_only_to_directories_holding_a_config() {
        let root = tempfile::tempdir().unwrap();
        write(
            &root.path().join("packages/orders/morphir.toml"),
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        );
        write(
            &root.path().join("packages/billing/morphir.yaml"),
            "project:\n  name: acme/billing\n  version: 1.0.0\n",
        );
        std::fs::create_dir_all(root.path().join("packages/scratch")).unwrap();
        std::fs::create_dir_all(root.path().join("packages/.hidden")).unwrap();

        assert_eq!(
            expand_member_pattern(root.path(), "packages/*"),
            vec![
                root.path().join("packages").join("billing"),
                root.path().join("packages").join("orders"),
            ]
        );
    }

    #[test]
    fn expanding_every_member_keeps_list_order_without_duplicates() {
        let root = tempfile::tempdir().unwrap();
        write(
            &root.path().join("packages/orders/morphir.toml"),
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        );
        let members = vec![
            "tools/cli".to_owned(),
            "packages/*".to_owned(),
            "packages/orders".to_owned(),
        ];
        assert_eq!(
            expand_members(root.path(), &members),
            vec![
                root.path().join("tools").join("cli"),
                root.path().join("packages").join("orders"),
            ]
        );
    }

    #[test]
    fn membership_is_decided_from_the_relative_path() {
        let root = Path::new("/ws");
        let members = vec!["packages/*".to_owned()];
        assert!(members_select(
            root,
            &members,
            Path::new("/ws/packages/orders")
        ));
        assert!(!members_select(
            root,
            &members,
            Path::new("/ws/packages/acme/orders")
        ));
        assert!(!members_select(
            root,
            &members,
            Path::new("/other/packages/orders")
        ));
        assert!(members_select(
            root,
            &["packages/**".to_owned()],
            Path::new("/ws/packages/acme/orders")
        ));
    }
}
