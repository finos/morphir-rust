//! With `MORPHIR_HOME` set, caches must stay under the relocated home so
//! sandboxed and hermetic environments never touch the real user directories.
//!
//! These tests mutate the process environment, so they live in their own
//! integration-test binary (one process) instead of the unit-test suite.

use morphir_common::home::{MorphirHome, cache_root_from, cache_root_from_home};
use morphir_common::remote::RemoteSourceConfig;
use std::path::{Path, PathBuf};

fn set_morphir_home(path: &str) {
    // SAFETY: this test binary is single-purpose; no other thread reads the
    // environment while these tests run.
    unsafe { std::env::set_var("MORPHIR_HOME", path) };
}

#[test]
fn relocated_home_keeps_remote_source_cache_under_home() {
    set_morphir_home("/sandbox/morphir-home");

    let home = MorphirHome::resolve().unwrap();
    assert_eq!(home.root(), Path::new("/sandbox/morphir-home"));
    assert!(home.is_relocated());

    assert_eq!(
        RemoteSourceConfig::default().cache_directory(),
        Path::new("/sandbox/morphir-home/cache/sources")
    );
}

#[test]
fn public_cache_helpers_distinguish_home_and_legacy_cache_inputs() {
    assert_eq!(
        cache_root_from_home(None, Some(PathBuf::from("/home/u"))),
        Some(PathBuf::from("/home/u/.morphir/cache"))
    );
    assert_eq!(
        cache_root_from(None, Some(PathBuf::from("/home/u/.cache"))),
        Some(PathBuf::from("/home/u/.cache/morphir"))
    );
}
