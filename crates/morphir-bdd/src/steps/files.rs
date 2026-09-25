//! File steps. Paths are relative to the scenario's temporary directory.

use std::path::{Component, Path, PathBuf};

use cucumber::gherkin::Step;
use cucumber::{given, then};

use crate::diff::unified_diff;
use crate::world::MorphirWorld;

/// The scenario's own temporary directory. File steps read and write only inside it.
#[derive(Debug)]
pub struct Workspace {
    /// The temporary directory. It is removed when this value drops.
    pub dir: tempfile::TempDir,
}

/// Returns the scenario's [`Workspace`], creating one if this is the first file step to run.
pub fn workspace(world: &mut MorphirWorld) -> &Workspace {
    if world.context.get::<Workspace>().is_none() {
        world.context.insert(Workspace {
            dir: tempfile::tempdir().expect("create a temporary directory"),
        });
    }
    world.context.get::<Workspace>().expect("inserted")
}

/// Resolves `path` against the scenario's [`Workspace`], creating the workspace if needed.
///
/// Panics if `path` is absolute or has a `..` component: file steps must stay inside the
/// scenario's temporary directory, never touch the rest of the filesystem.
fn resolve(world: &mut MorphirWorld, path: &str) -> PathBuf {
    let candidate = Path::new(path);
    assert!(
        !candidate.is_absolute(),
        "file step path {path:?} must be relative to the workspace, not absolute"
    );
    assert!(
        !candidate
            .components()
            .any(|c| matches!(c, Component::ParentDir)),
        "file step path {path:?} must not contain a `..` component"
    );
    workspace(world).dir.path().join(candidate)
}

/// Reads a step's doc string, panicking if the step has none.
fn doc_string(step: &Step) -> &str {
    step.docstring
        .as_deref()
        .expect("this step takes a doc string")
}

/// `Given a temporary directory` creates the scenario's [`Workspace`] if it does not exist yet.
#[given("a temporary directory")]
fn a_temporary_directory(world: &mut MorphirWorld) {
    workspace(world);
}

/// `Given a file {string} containing:` writes the doc string to `path` inside the workspace,
/// creating the workspace and any parent directories it needs.
#[given(expr = "a file {string} containing:")]
fn a_file_containing(world: &mut MorphirWorld, path: String, step: &Step) {
    let contents = doc_string(step).to_owned();
    let target = resolve(world, &path);
    std::fs::create_dir_all(target.parent().expect("a file path has a parent"))
        .expect("create parent directories");
    std::fs::write(&target, contents).expect("write the file");
}

/// `Then the file {string} should exist` asserts that `path` names a file inside the workspace.
#[then(expr = "the file {string} should exist")]
fn the_file_should_exist(world: &mut MorphirWorld, path: String) {
    let target = resolve(world, &path);
    assert!(target.is_file(), "{} does not exist", target.display());
}

/// `Then the file {string} should contain:` asserts that `path`'s contents equal the doc string,
/// printing a unified diff on mismatch.
#[then(expr = "the file {string} should contain:")]
fn the_file_should_contain(world: &mut MorphirWorld, path: String, step: &Step) {
    let expected = doc_string(step).to_owned();
    let target = resolve(world, &path);
    let actual = std::fs::read_to_string(&target)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", target.display()));
    if actual != expected {
        panic!(
            "{path} differs:\n{}",
            unified_diff(
                &expected,
                &actual,
                &format!("expected {path}"),
                &format!("actual {path}")
            )
        );
    }
}

#[cfg(test)]
mod tests {
    use super::resolve;
    use crate::world::MorphirWorld;

    #[test]
    fn resolve_joins_a_relative_path_under_the_workspace() {
        let mut world = MorphirWorld::new();

        let target = resolve(&mut world, "a/b.txt");

        let dir = super::workspace(&mut world).dir.path().to_owned();
        assert_eq!(target, dir.join("a/b.txt"));
    }

    #[test]
    #[should_panic(expected = "must be relative to the workspace, not absolute")]
    fn resolve_refuses_an_absolute_path() {
        let mut world = MorphirWorld::new();

        resolve(&mut world, "/etc/passwd");
    }

    #[test]
    #[should_panic(expected = "must not contain a `..` component")]
    fn resolve_refuses_a_path_with_a_parent_dir_component() {
        let mut world = MorphirWorld::new();

        resolve(&mut world, "../escape.txt");
    }
}
