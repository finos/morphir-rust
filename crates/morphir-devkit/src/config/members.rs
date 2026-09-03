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
///
/// The entry is tidied first, because `RelativePath` also refuses a redundant
/// separator: `packages/` and `packages//orders` are perfectly ordinary member
/// entries that the rest of this module has always accepted, and saying they
/// leave the workspace would be untrue.
pub(crate) fn is_confined(entry: &str) -> bool {
    RelativePath::parse(without_redundant_separators(entry)).is_ok()
}

/// Collapse the spellings that name the same directory — a trailing separator,
/// a doubled separator, a `.` segment — so the confinement check answers the
/// question the entry actually asks.
///
/// A leading separator survives: that is the entry claiming to be absolute, and
/// the check has to see it. So does a backslash, which is a separator on
/// Windows and has to be refused rather than tidied away. An entry left with
/// nothing at all is the workspace root, which `is_member` skips on its own.
fn without_redundant_separators(entry: &str) -> String {
    let leading = if entry.starts_with('/') { "/" } else { "" };
    let segments: Vec<&str> = entry
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != ".")
        .collect();
    if segments.is_empty() {
        return if leading.is_empty() { "." } else { "/" }.to_owned();
    }
    format!("{leading}{}", segments.join("/"))
}

/// The warning shown for an entry [`is_confined`] rejects.
pub(crate) fn unconfined_warning(entry: &str) -> String {
    format!(
        "workspace member entry '{entry}' is not confined to the workspace directory; it is ignored"
    )
}

/// Where a member's declared directory leads, once the symbolic links along
/// the way have been followed.
///
/// A member is identified by the path it is DECLARED at, relative to the
/// workspace root, the way a Mill module is identified by its position in the
/// build. Where that directory really sits does not change its identity: its
/// task output stays under `.morphir/out/<declared path>/` either way.
/// Confinement is a rule about what Morphir WRITES — the out root and each
/// task's `.dest` — not about where a declared member reads its sources from.
pub(crate) enum MemberDirectory {
    /// Nothing is on disk at that path, or it leads to a directory under the
    /// workspace root. Either way there is nothing to say about it.
    Ordinary,
    /// The path leads outside the workspace directory, through a symbolic
    /// link. Sources are read from there; the member keeps its declared
    /// identity. Carries the resolved location, for the warning.
    Outside(PathBuf),
    /// Something is at that path but it does not resolve: a link with nothing
    /// at the far end, or a directory that cannot be read. Morphir cannot say
    /// what it would load, so it is skipped.
    Unresolvable,
}

/// The warning shown for a member that resolves outside the workspace.
pub(crate) fn outside_workspace_warning(
    workspace_root: &Path,
    directory: &Path,
    resolved: &Path,
) -> String {
    let declared = directory
        .strip_prefix(workspace_root)
        .unwrap_or(directory)
        .display();
    format!(
        "workspace member '{declared}' resolves to '{}', outside the workspace directory; \
         its sources are read from there, and its output stays under .morphir/out/{declared}/",
        resolved.display()
    )
}

/// The warning shown for a member directory that is there but does not
/// resolve.
pub(crate) fn unresolvable_member_warning(directory: &Path) -> String {
    format!(
        "workspace member '{}' does not resolve — it is a symbolic link with no target, or a \
         directory that cannot be read; it is ignored",
        directory.display()
    )
}

/// Follow `directory` and say where it leads relative to `workspace_root`.
///
/// A directory that is not there at all is [`MemberDirectory::Ordinary`]. A
/// literal member entry is expanded whether or not it exists, so that a
/// listed-but-missing member is reported as a not-found source rather than
/// disappearing, and a path with nothing behind it has no link to follow.
///
/// Anything that is there but will not resolve — a dangling link, a directory
/// that cannot be read — is [`MemberDirectory::Unresolvable`], because a
/// member Morphir cannot place is not one it should merge. `canonicalize`
/// reports both a missing path and a dangling link as "not found", so
/// `symlink_metadata` is what tells them apart: it does not follow the last
/// link, so it succeeds on a dangling one and fails on a path with nothing
/// at it.
///
/// Both sides are canonicalized before comparing, so a workspace root that is
/// itself reached through a symbolic link (a temporary directory under
/// `/var` on macOS, say) still contains its own members.
pub(crate) fn classify_member_directory(
    workspace_root: &Path,
    directory: &Path,
) -> MemberDirectory {
    let canonical_directory = match std::fs::canonicalize(directory) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return if std::fs::symlink_metadata(directory).is_err() {
                MemberDirectory::Ordinary
            } else {
                MemberDirectory::Unresolvable
            };
        }
        Err(_) => return MemberDirectory::Unresolvable,
    };
    match std::fs::canonicalize(workspace_root) {
        Ok(canonical_root) if !canonical_directory.starts_with(&canonical_root) => {
            MemberDirectory::Outside(canonical_directory)
        }
        // Either the member sits under the root, or the workspace root is not
        // on disk at all and nothing can be shown to sit outside it.
        _ => MemberDirectory::Ordinary,
    }
}

/// Classify `directory`, add whatever warning that calls for to `warnings`,
/// and say whether it may be loaded as a member.
pub(crate) fn accept_member_directory(
    workspace_root: &Path,
    directory: &Path,
    warnings: &mut Vec<String>,
) -> bool {
    match classify_member_directory(workspace_root, directory) {
        MemberDirectory::Ordinary => true,
        MemberDirectory::Outside(resolved) => {
            warnings.push(outside_workspace_warning(
                workspace_root,
                directory,
                &resolved,
            ));
            true
        }
        MemberDirectory::Unresolvable => {
            warnings.push(unresolvable_member_warning(directory));
            false
        }
    }
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
/// An entry whose SPELLING would leave the workspace directory is skipped,
/// with a warning naming it appended to `warnings`. An entry that is spelled
/// as an ordinary relative path but names a symbolic link to a directory
/// outside the workspace is kept — Morphir reads its sources from where the
/// link leads — with one warning saying so. See [`is_confined`] and
/// [`classify_member_directory`].
pub(crate) fn expand_members(
    workspace_root: &Path,
    members: &[String],
    exclude: &[String],
    warnings: &mut Vec<String>,
) -> Vec<PathBuf> {
    let mut expanded = Vec::new();
    for member in confined_patterns(members, warnings) {
        for path in expand_member_pattern(workspace_root, member) {
            if expanded.contains(&path)
                || !is_member(workspace_root, exclude, &path)
                || !accept_member_directory(workspace_root, &path, warnings)
            {
                continue;
            }
            expanded.push(path);
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
/// The pattern side of the question is answered without touching the
/// filesystem, so it is cheap to ask while walking up from a candidate member
/// towards its workspace. Only a directory that got that far is followed on
/// disk, and one that leads outside the workspace is still a member — it just
/// earns a warning. See [`classify_member_directory`].
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
        && accept_member_directory(workspace_root, directory, warnings)
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
///
/// A directory symlink counts as a subdirectory too: `DirEntry::file_type`
/// reports the link itself, not what it points at, so it is checked with
/// `std::fs::metadata`, which follows the link, before it is ruled out. A
/// symlinked member selected this way is an ordinary member; `expand_members`
/// only adds a warning when it leads outside the workspace, via
/// [`classify_member_directory`].
fn child_directories(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => true,
            Ok(kind) if kind.is_symlink() => {
                std::fs::metadata(entry.path()).is_ok_and(|metadata| metadata.is_dir())
            }
            _ => false,
        })
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
        // A literal entry never touches the filesystem — `expand_member_pattern`
        // joins it onto the workspace root and hands it straight to
        // `is_member` — so the sibling directory these entries name does not
        // have to exist for the test to be honest about what used to happen:
        // `is_member` saw the single segment `outside`, said yes, and the
        // loader went looking for a configuration outside the workspace.
        let root = tempfile::tempdir().unwrap();
        let outside = root.path().parent().unwrap().join("outside-member");

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

    /// The entry `packages/orders` is confined as text — there is nothing in
    /// its spelling to complain about. What it names on disk is a symbolic
    /// link to a directory beside the workspace: linking a sibling checkout
    /// into `packages/` is an ordinary way to work, so the member is loaded
    /// from where the link leads. It keeps the identity its declared path
    /// gives it, so its output still lands under the workspace's out root,
    /// and one warning says both things.
    #[cfg(unix)]
    #[test]
    fn a_member_symlinked_out_of_the_workspace_is_accepted_with_a_warning() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("ws");
        let outside = temp.path().join("outside");
        write(
            &outside.join("morphir.toml"),
            "[project]\nname = \"acme/outside\"\nversion = \"1.0.0\"\n",
        );
        std::fs::create_dir_all(root.join("packages")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("packages/orders")).unwrap();

        let mut warnings = Vec::new();
        assert_eq!(
            expand_members(&root, &["packages/orders".to_owned()], &[], &mut warnings),
            vec![root.join("packages").join("orders")],
            "a member is identified by its declared path, wherever it resolves"
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("outside the workspace directory"),
            "{warnings:?}"
        );
        assert!(warnings[0].contains("packages/orders"), "{warnings:?}");
        assert!(
            warnings[0].contains(".morphir/out/packages/orders/"),
            "{warnings:?}"
        );

        // The walk-up asks the same question from the member's side.
        let mut warnings = Vec::new();
        assert!(members_select(
            &root,
            &["packages/*".to_owned()],
            &[],
            &root.join("packages/orders"),
            &mut warnings,
        ));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("outside the workspace directory"),
            "{warnings:?}"
        );
    }

    /// A link with nothing at the far end is not the same as no member at
    /// all: something is there, and it does not resolve, so Morphir cannot
    /// say where it would place it.
    #[cfg(unix)]
    #[test]
    fn a_member_symlinked_to_nothing_is_skipped_with_a_warning() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("ws");
        std::fs::create_dir_all(root.join("packages")).unwrap();
        std::os::unix::fs::symlink(temp.path().join("gone"), root.join("packages/orders")).unwrap();

        let mut warnings = Vec::new();
        assert_eq!(
            expand_members(&root, &["packages/orders".to_owned()], &[], &mut warnings),
            Vec::<PathBuf>::new()
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");

        // A member that is simply missing still expands, so that it is
        // reported as a not-found source.
        let mut warnings = Vec::new();
        assert_eq!(
            expand_members(&root, &["packages/absent".to_owned()], &[], &mut warnings),
            vec![root.join("packages").join("absent")]
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// A symbolic link is a perfectly ordinary way to arrange a workspace, so
    /// only the ones that leave it are refused.
    #[cfg(unix)]
    #[test]
    fn a_member_symlinked_within_the_workspace_is_still_a_member() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("ws");
        write(
            &root.join("vendor/orders/morphir.toml"),
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        );
        std::fs::create_dir_all(root.join("packages")).unwrap();
        std::os::unix::fs::symlink(root.join("vendor/orders"), root.join("packages/orders"))
            .unwrap();

        let mut warnings = Vec::new();
        assert_eq!(
            expand_members(&root, &["packages/orders".to_owned()], &[], &mut warnings),
            vec![root.join("packages").join("orders")]
        );
        assert!(warnings.is_empty(), "{warnings:?}");

        let mut warnings = Vec::new();
        assert!(members_select(
            &root,
            &["packages/*".to_owned()],
            &[],
            &root.join("packages/orders"),
            &mut warnings,
        ));
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// `packages/*` walks the tree with `child_directories`, which used
    /// `DirEntry::file_type` and so never saw a directory symlink as a
    /// subdirectory to descend into — a member reached the same way through a
    /// literal entry was accepted, but a glob silently dropped it. A
    /// symlinked member resolving inside the workspace must appear in the
    /// wildcard's expansion.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_member_selected_by_a_glob_is_loaded_when_it_resolves_inside() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("ws");
        write(
            &root.join("vendor/orders/morphir.toml"),
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        );
        std::fs::create_dir_all(root.join("packages")).unwrap();
        std::os::unix::fs::symlink(root.join("vendor/orders"), root.join("packages/orders"))
            .unwrap();

        let mut warnings = Vec::new();
        assert_eq!(
            expand_members(&root, &["packages/*".to_owned()], &[], &mut warnings),
            vec![root.join("packages").join("orders")]
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// The same wildcard walk, but the symlinked member resolves outside the
    /// workspace. It is loaded all the same, under the declared path the glob
    /// matched, with one warning saying where its sources come from.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_member_selected_by_a_glob_is_loaded_with_a_warning_when_it_resolves_outside() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("ws");
        let outside = temp.path().join("outside");
        write(
            &outside.join("morphir.toml"),
            "[project]\nname = \"acme/outside\"\nversion = \"1.0.0\"\n",
        );
        std::fs::create_dir_all(root.join("packages")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("packages/orders")).unwrap();

        let mut warnings = Vec::new();
        assert_eq!(
            expand_members(&root, &["packages/*".to_owned()], &[], &mut warnings),
            vec![root.join("packages").join("orders")]
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("outside the workspace directory"),
            "{warnings:?}"
        );
    }

    #[test]
    fn confinement_accepts_ordinary_entries_and_the_root() {
        assert!(is_confined("packages/orders"));
        assert!(is_confined("packages/*"));
        assert!(is_confined("**"));
        assert!(is_confined("."));
        assert!(!is_confined("../outside"));
        assert!(!is_confined("packages/../../outside"));
        assert!(!is_confined("/etc/morphir"));
        assert!(!is_confined(r"..\outside"));
        assert!(!is_confined(r"C:\outside"));
    }

    #[test]
    fn confinement_tolerates_redundant_separators() {
        // These name a directory inside the workspace, however they are
        // spelled, and `pattern_segments` has always read them that way. The
        // confinement check has to agree, or a workspace with a trailing
        // slash in its members list is told its member escapes.
        for entry in [
            "packages/",
            "packages//orders",
            "./packages",
            "packages/./orders",
        ] {
            assert!(is_confined(entry), "{entry}");
        }
        // Nothing but separators is the workspace root, which is never its own
        // member — silently skipped, not reported as an escape.
        for entry in ["", "."] {
            assert!(is_confined(entry), "{entry:?}");
        }
        // A leading separator is not redundant: it is the entry saying it is
        // absolute, and it still has to be refused.
        assert!(!is_confined("/packages/orders"));
        assert!(!is_confined("/"));
    }

    #[test]
    fn a_trailing_separator_still_expands_to_the_same_member() {
        let root = tempfile::tempdir().unwrap();
        write(
            &root.path().join("packages/orders/morphir.toml"),
            "[project]\nname = \"acme/orders\"\nversion = \"1.0.0\"\n",
        );

        let mut warnings = Vec::new();
        assert_eq!(
            expand_members(
                root.path(),
                &["packages/orders/".to_owned()],
                &[],
                &mut warnings
            ),
            vec![root.path().join("packages").join("orders")]
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}
