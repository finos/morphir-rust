//! Port of the in-scope cases of the `KbPathsSpec` suite from
//! `KbTests.scala`. The DocRef / intent-vocabulary / slugify cases belong to
//! the intent register and scaffold, which live in the later `morphir-kb`
//! crate.

use morphir_okf::paths;
use std::path::Path;

#[test]
fn segments_under_relativises_a_path_below_the_base() {
    assert_eq!(
        paths::segments_under(Path::new("/a/b/c/d.md"), Path::new("/a/b")),
        Some(vec!["c".to_string(), "d.md".to_string()])
    );
}

#[test]
fn segments_under_refuses_an_unrelated_path() {
    assert_eq!(
        paths::segments_under(Path::new("/x/y"), Path::new("/a/b")),
        None
    );
}

#[test]
fn segments_under_of_the_base_itself_is_empty() {
    assert_eq!(
        paths::segments_under(Path::new("/a/b"), Path::new("/a/b")),
        Some(vec![])
    );
}

#[test]
fn is_under_agrees() {
    assert!(paths::is_under(Path::new("/a/b/c"), Path::new("/a/b")));
    assert!(!paths::is_under(Path::new("/x"), Path::new("/a/b")));
}

#[test]
fn render_uses_forward_slashes_with_a_leading_slash() {
    assert_eq!(paths::render(Path::new("/a/b/c.md")), "/a/b/c.md");
    assert_eq!(paths::render(Path::new("a/b")), "/a/b");
}
