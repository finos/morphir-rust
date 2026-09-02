//! Task identity and on-disk locations under the out root.

use std::fmt;
use std::path::{Path, PathBuf};

/// Errors raised while building task identities.
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

    /// Parse a task id written as a `/`-separated path. Segments after the
    /// first are sanitized.
    pub fn parse(text: &str) -> Result<Self, OutError> {
        if text.is_empty() {
            return Err(OutError::EmptyTaskId);
        }
        let mut rendered = Vec::new();
        for (position, segment) in text.split('/').enumerate() {
            if segment.is_empty() {
                return Err(OutError::EmptySegment { position });
            }
            rendered.push(if position == 0 {
                segment.to_owned()
            } else {
                sanitize_segment(segment)
            });
        }
        Ok(Self(rendered.join("/")))
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
        assert_eq!(
            TaskId::parse("generate/scala").unwrap().as_str(),
            "generate/scala"
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
