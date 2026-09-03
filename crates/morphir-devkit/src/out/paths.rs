//! Task identity and on-disk locations under the out root.

use std::fmt;
use std::path::{Path, PathBuf};

/// Errors raised while building task identities and reading result records.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OutError {
    /// The task id text was empty.
    #[error("task id must not be empty")]
    EmptyTaskId,
    /// One `/`-separated segment was empty.
    #[error("task id segment {position} must not be empty")]
    EmptySegment {
        /// Zero-based index of the empty segment.
        position: usize,
    },
    /// One segment named something other than a single directory segment.
    // One line on purpose: a `\` continuation here reads well in the source but
    // rustfmt rejoins it and the indentation lands in the message users see.
    #[error(
        "task id segment {position} (`{segment}`) must name one directory segment, not `.`, `..`, or a path"
    )]
    UnsafeSegment {
        /// Zero-based index of the offending segment.
        position: usize,
        /// The text that was offered as a segment.
        segment: String,
    },
    /// The text did not name an IR layout.
    #[error("unknown IR layout `{value}`; expected `single-file` or `document-tree`")]
    UnknownIrLayout {
        /// Text that was offered as a layout name.
        value: String,
    },
}

/// Path-like task identity such as `compile`, `generate/scala`, or
/// `transform/inline-sdk`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TaskId(String);

impl TaskId {
    /// The compile task. One per module.
    pub fn compile() -> Self {
        Self("compile".to_owned())
    }

    /// The generate task for one target.
    pub fn generate(target: &str) -> Self {
        Self(format!("generate/{}", sanitize_segment(target)))
    }

    /// The transform task with one name.
    pub fn transform(name: &str) -> Self {
        Self(format!("transform/{}", sanitize_segment(name)))
    }

    /// Parse a task id written as a `/`-separated path.
    ///
    /// Every segment has to name one ordinary directory segment, because
    /// [`TaskPaths::new`] joins each one onto the out root in turn: an empty
    /// segment, `.`, `..`, a segment holding a backslash (a separator on
    /// Windows), and a Windows drive prefix such as `C:` are all refused. The
    /// first segment is checked like every other one — it used to be taken
    /// verbatim, so `TaskId::parse("../compile")` produced a task whose
    /// `.dest` sat beside the out root rather than under it.
    ///
    /// The constructors ([`TaskId::compile`], [`TaskId::generate`],
    /// [`TaskId::transform`]) hardcode their first segment and sanitize the
    /// rest, so they are unaffected by this stricter rule.
    pub fn parse(text: &str) -> Result<Self, OutError> {
        if text.is_empty() {
            return Err(OutError::EmptyTaskId);
        }
        for (position, segment) in text.split('/').enumerate() {
            if segment.is_empty() {
                return Err(OutError::EmptySegment { position });
            }
            if !is_one_directory_segment(segment) {
                return Err(OutError::UnsafeSegment {
                    position,
                    segment: segment.to_owned(),
                });
            }
        }
        Ok(Self(text.to_owned()))
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Does this text name exactly one ordinary directory segment, so it can be
/// joined onto a path without moving anywhere but downwards?
///
/// `.` and `..` move nowhere and upwards; a backslash separates directories on
/// Windows; and a leading drive letter (`C:`) makes a path drive-relative
/// there, which `Path::join` honours by discarding the base.
fn is_one_directory_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    let drive_prefix = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.contains('\\')
        && !drive_prefix
}

/// Replace path separators, spaces, the parent-directory segment, and the
/// empty segment so a user-supplied name is one safe, visible directory
/// segment.
///
/// An empty segment would otherwise produce a hidden `.dest` directory and a
/// `.json` file with no stem, so it maps to `-` the same way `..` does.
pub fn sanitize_segment(segment: &str) -> String {
    if segment.is_empty() || segment == ".." {
        return "-".to_owned();
    }
    segment.replace(['/', ' ', '\\'], "-")
}

/// Scratch directory and result record of one task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskPaths {
    /// `<root>/<module_path>/<task_id>.dest`
    pub dest: PathBuf,
    /// `<root>/<module_path>/<task_id>.json`
    pub result: PathBuf,
}

impl TaskPaths {
    /// Build both locations for a task under `root`. `module_path` is the
    /// module's path relative to the workspace root; empty for the root module.
    pub fn new(root: &Path, module_path: &Path, task: &TaskId) -> Self {
        let mut base = root.join(module_path);
        let segments: Vec<&str> = task.segments().collect();
        let (leaf, parents) = segments
            .split_last()
            .expect("task id has at least one segment");
        for parent in parents {
            base = base.join(parent);
        }
        Self {
            dest: base.join(format!("{leaf}.dest")),
            result: base.join(format!("{leaf}.json")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn task_ids_render_as_paths() {
        assert_eq!(TaskId::compile().as_str(), "compile");
        assert_eq!(TaskId::generate("scala").as_str(), "generate/scala");
        assert_eq!(
            TaskId::transform("inline-sdk").as_str(),
            "transform/inline-sdk"
        );
    }

    #[test]
    fn task_id_segments_after_the_first_are_sanitized() {
        assert_eq!(
            TaskId::generate("acme/orders").as_str(),
            "generate/acme-orders"
        );
        assert_eq!(TaskId::generate("a b").as_str(), "generate/a-b");
        assert_eq!(TaskId::generate(r"a\b").as_str(), "generate/a-b");
        assert_eq!(TaskId::generate("..").as_str(), "generate/-");
    }

    #[test]
    fn an_empty_name_never_produces_a_hidden_task_directory() {
        assert_eq!(sanitize_segment(""), "-");
        assert_eq!(TaskId::generate("").as_str(), "generate/-");
        assert_eq!(TaskId::transform("").as_str(), "transform/-");

        let paths = TaskPaths::new(
            Path::new("/ws/.morphir/out"),
            Path::new(""),
            &TaskId::generate(""),
        );
        assert_eq!(
            paths.dest,
            PathBuf::from("/ws/.morphir/out/generate/-.dest")
        );
        assert_eq!(
            paths.result,
            PathBuf::from("/ws/.morphir/out/generate/-.json")
        );
    }

    #[test]
    fn parse_rejects_empty_ids_and_segments() {
        assert!(matches!(TaskId::parse(""), Err(OutError::EmptyTaskId)));
        assert!(matches!(
            TaskId::parse("generate/"),
            Err(OutError::EmptySegment { position: 1 })
        ));
        // A leading `/` makes the first segment empty.
        assert!(matches!(
            TaskId::parse("/etc/passwd"),
            Err(OutError::EmptySegment { position: 0 })
        ));
        assert_eq!(
            TaskId::parse("generate/scala").unwrap().as_str(),
            "generate/scala"
        );
        assert_eq!(TaskId::parse("compile").unwrap().as_str(), "compile");
    }

    #[test]
    fn parse_refuses_a_first_segment_that_would_escape_the_out_root() {
        // `TaskPaths::new` joins every segment but the last onto the out root,
        // so a first segment of `..` used to put a task's `.dest` beside the
        // out root instead of under it.
        for (id, position) in [
            ("../compile", 0),
            ("..", 0),
            (".", 0),
            ("./compile", 0),
            ("generate/..", 1),
            (r"..\compile", 0),
            ("C:/compile", 0),
        ] {
            match TaskId::parse(id) {
                Err(OutError::UnsafeSegment { position: at, .. }) => {
                    assert_eq!(at, position, "{id}")
                }
                other => panic!("`{id}` must be refused, got {other:?}"),
            }
        }
        assert!(
            TaskId::parse("../compile")
                .unwrap_err()
                .to_string()
                .contains("must name one directory segment")
        );
    }

    #[test]
    fn root_module_paths_sit_directly_under_the_root() {
        let paths = TaskPaths::new(
            Path::new("/ws/.morphir/out"),
            Path::new(""),
            &TaskId::compile(),
        );
        assert_eq!(paths.dest, PathBuf::from("/ws/.morphir/out/compile.dest"));
        assert_eq!(paths.result, PathBuf::from("/ws/.morphir/out/compile.json"));
    }

    #[test]
    fn member_paths_nest_under_the_member_path() {
        let paths = TaskPaths::new(
            Path::new("/ws/.morphir/out"),
            Path::new("packages/orders"),
            &TaskId::generate("scala"),
        );
        assert_eq!(
            paths.dest,
            PathBuf::from("/ws/.morphir/out/packages/orders/generate/scala.dest")
        );
        assert_eq!(
            paths.result,
            PathBuf::from("/ws/.morphir/out/packages/orders/generate/scala.json")
        );
    }
}
