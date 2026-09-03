//! Workspace member patterns: expanding `[workspace].members` into member
//! directories and deciding whether a directory is a member of a workspace.
//!
//! The matcher is deliberately small. A pattern is a `/`-separated list of
//! segments; `*` matches any run of characters inside one segment, and a whole
//! `**` segment matches zero or more segments. That is everything the
//! `packages/*` style of member list needs, so no glob dependency is pulled in.

use super::discovery::discover_config_at;
use morphir_workspace::RelativePath;
use std::path::{Component, Path, PathBuf};

/// Does this pattern contain a wildcard, so it has to be expanded against the
/// filesystem rather than joined onto the workspace root?
pub(crate) fn is_pattern(text: &str) -> bool {
    text.contains('*')
}

/// Does this `members`, `default_member`, or `exclude` entry stay inside the
/// workspace directory?
///
/// It has to be asked before the entry is joined onto the workspace root,
/// because nothing after that point can tell the difference: `path_segments`
/// keeps only the `Normal` components, so `../outside` reduces to the single
/// segment `outside` and looks like an ordinary member, while
/// `workspace_root.join("../outside")` names a sibling of the workspace and the
/// loader would merge a configuration from there.
///
/// `morphir-workspace` already refuses the same input with
/// `WORKSPACE_PATH_NOT_CONFINED`, so the rule here is its `RelativePath` type
/// rather than a second, possibly divergent, opinion: an absolute path, a
/// Windows drive prefix, a backslash separator, and a `.` or `..` component are
/// all rejected, and `.` alone (the workspace root itself) is allowed, since
/// `is_member` already refuses to treat the root as its own member.
pub(crate) fn is_confined(entry: &str) -> bool {
    RelativePath::parse(entry).is_ok()
}

/// The warning shown for an entry [`is_confined`] rejects.
pub(crate) fn unconfined_warning(entry: &str) -> String {
    format!(
        "workspace member entry '{entry}' is not confined to the workspace directory; it is ignored"
    )
}

/// The entries of `patterns` that stay inside the workspace, adding a warning
/// for each one that does not.
fn confined_patterns<'a>(patterns: &'a [String], warnings: &mut Vec<String>) -> Vec<&'a String> {
    patterns
        .iter()
        .filter(|pattern| {
            let confined = is_confined(pattern);
            if !confined {
                warnings.push(unconfined_warning(pattern));
            }
            confined
        })
        .collect()
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

/// Expand every member entry of a workspace, in the order they are listed,
/// dropping the workspace root itself and anything `exclude` rules out.
///
/// An entry that would leave the workspace directory is skipped, with a
/// warning naming it appended to `warnings`. See [`is_confined`].
pub(crate) fn expand_members(
    workspace_root: &Path,
    members: &[String],
    exclude: &[String],
    warnings: &mut Vec<String>,
) -> Vec<PathBuf> {
    let mut expanded = Vec::new();
    for member in confined_patterns(members, warnings) {
        for path in expand_member_pattern(workspace_root, member) {
            if !expanded.contains(&path) && is_member(workspace_root, exclude, &path) {
                expanded.push(path);
            }
        }
    }
    expanded
}

/// Is `directory` a member of the workspace at `workspace_root`, given that it
/// already matched the `members` list?
///
/// The workspace root is never its own member: a `members` entry such as `**`
/// or `.` selects it, and merging the workspace configuration a second time as
/// the member layer would misattribute every value it sets. `morphir-workspace`
/// applies the same guard while discovering members.
pub(crate) fn is_member(workspace_root: &Path, exclude: &[String], directory: &Path) -> bool {
    let Ok(relative) = directory.strip_prefix(workspace_root) else {
        return false;
    };
    // An empty relative path is the root; so is one made only of `.`, which is
    // what `members = ["."]` and `default_member = "."` join to.
    !path_segments(relative).is_empty() && !patterns_select(workspace_root, exclude, directory)
}

/// Does `directory` sit at a path that one of `members` selects, relative to
/// `workspace_root`, and survive the `exclude` patterns?
///
/// This answers the question without touching the filesystem, so it is safe to
/// ask while walking up from a candidate member towards its workspace.
pub(crate) fn members_select(
    workspace_root: &Path,
    members: &[String],
    exclude: &[String],
    directory: &Path,
    warnings: &mut Vec<String>,
) -> bool {
    let members = confined_patterns(members, warnings);
    patterns_select_refs(workspace_root, &members, directory)
        && is_member(workspace_root, exclude, directory)
}

/// Does any pattern select `directory`, relative to `workspace_root`?
fn patterns_select(workspace_root: &Path, patterns: &[String], directory: &Path) -> bool {
    patterns_select_refs(
        workspace_root,
        &patterns.iter().collect::<Vec<_>>(),
        directory,
    )
}

fn patterns_select_refs(workspace_root: &Path, patterns: &[&String], directory: &Path) -> bool {
    let Ok(relative) = directory.strip_prefix(workspace_root) else {
        return false;
    };
    let actual = path_segments(relative);
    patterns.iter().any(|pattern| {
        let pattern = pattern_segments(pattern);
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

    /// `expand_members` and `members_select` with a warning sink the test
    /// does not inspect. Tests that check warnings call them directly.
    fn expand(root: &Path, members: &[String], exclude: &[String]) -> Vec<PathBuf> {
        expand_members(root, members, exclude, &mut Vec::new())
    }

    fn selects(root: &Path, members: &[String], exclude: &[String], directory: &Path) -> bool {
        members_select(root, members, exclude, directory, &mut Vec::new())
    }

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
            expand(root.path(), &members, &[]),
            vec![
                root.path().join("tools").join("cli"),
                root.path().join("packages").join("orders"),
            ]
        );
    }

    #[test]
    fn expansion_drops_excluded_directories() {
        let root = tempfile::tempdir().unwrap();
        for name in ["orders", "ignored"] {
            write(
                &root.path().join("packages").join(name).join("morphir.toml"),
                &format!("[project]\nname = \"acme/{name}\"\nversion = \"1.0.0\"\n"),
            );
        }
        let members = vec!["packages/*".to_owned()];
        assert_eq!(
            expand(root.path(), &members, &["packages/ignored".to_owned()]),
            vec![root.path().join("packages").join("orders")]
        );
        assert_eq!(
            expand(root.path(), &members, &["packages/*".to_owned()]),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn expansion_never_yields_the_workspace_root_itself() {
        let root = tempfile::tempdir().unwrap();
        write(
            &root.path().join("morphir.toml"),
            "[workspace]\nmembers = [\"**\"]\n",
        );
        write(
            &root.path().join("packages/orders/morphir.toml"),
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        );
        assert_eq!(
            expand(root.path(), &["**".to_owned()], &[]),
            vec![root.path().join("packages").join("orders")]
        );
        assert_eq!(
            expand(root.path(), &[".".to_owned()], &[]),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn membership_is_decided_from_the_relative_path() {
        let root = Path::new("/ws");
        let members = vec!["packages/*".to_owned()];
        assert!(selects(
            root,
            &members,
            &[],
            Path::new("/ws/packages/orders")
        ));
        assert!(!selects(
            root,
            &members,
            &[],
            Path::new("/ws/packages/acme/orders")
        ));
        assert!(!selects(
            root,
            &members,
            &[],
            Path::new("/other/packages/orders")
        ));
        assert!(selects(
            root,
            &["packages/**".to_owned()],
            &[],
            Path::new("/ws/packages/acme/orders")
        ));
    }

    #[test]
    fn exclude_patterns_beat_the_members_list() {
        let root = Path::new("/ws");
        let members = vec!["packages/*".to_owned()];
        assert!(!selects(
            root,
            &members,
            &["packages/ignored".to_owned()],
            Path::new("/ws/packages/ignored")
        ));
        assert!(selects(
            root,
            &members,
            &["packages/ignored".to_owned()],
            Path::new("/ws/packages/orders")
        ));
        // Exclude patterns take wildcards too.
        assert!(!selects(
            root,
            &members,
            &["packages/*-internal".to_owned()],
            Path::new("/ws/packages/tooling-internal")
        ));
        // The workspace root is never its own member.
        assert!(!selects(root, &["**".to_owned()], &[], root));
    }

    #[test]
    fn entries_that_leave_the_workspace_are_skipped_with_a_warning() {
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().parent().unwrap().join("outside-member");
        write(
            &outside.join("morphir.toml"),
            "[project]\nname = \"acme/outside\"\nversion = \"1.0.0\"\n",
        );

        let mut warnings = Vec::new();
        let members = vec![
            "../outside-member".to_owned(),
            outside.to_string_lossy().into_owned(),
            r"..\outside-member".to_owned(),
        ];
        assert_eq!(
            expand_members(root.path(), &members, &[], &mut warnings),
            Vec::<PathBuf>::new(),
            "no entry that leaves the workspace may expand to a member"
        );
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings[0].contains("../outside-member"), "{warnings:?}");
        assert!(warnings[0].contains("not confined"), "{warnings:?}");
        assert!(warnings[2].contains(r"..\outside-member"), "{warnings:?}");
        std::fs::remove_dir_all(&outside).unwrap();
    }

    #[test]
    fn selecting_a_member_also_skips_entries_that_leave_the_workspace() {
        let root = Path::new("/ws");
        let mut warnings = Vec::new();
        assert!(!members_select(
            root,
            &["../outside".to_owned()],
            &[],
            Path::new("/outside"),
            &mut warnings,
        ));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("../outside"), "{warnings:?}");
    }

    #[test]
    fn confinement_accepts_ordinary_entries_and_the_root() {
        assert!(is_confined("packages/orders"));
        assert!(is_confined("packages/*"));
        assert!(is_confined("**"));
        assert!(is_confined("."));
        assert!(!is_confined(""));
        assert!(!is_confined("../outside"));
        assert!(!is_confined("packages/../../outside"));
        assert!(!is_confined("/etc/morphir"));
        assert!(!is_confined(r"..\outside"));
        assert!(!is_confined(r"C:\outside"));
    }
}
