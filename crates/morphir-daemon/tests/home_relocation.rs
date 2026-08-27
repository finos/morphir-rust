//! With `MORPHIR_HOME` set, the daemon's extension caches and virtual path
//! mappings must stay under the relocated home so sandboxed environments
//! never touch the real user directories.
//!
//! These tests mutate the process environment, so they live in their own
//! integration-test binary. `MORPHIR_HOME` is set once before both tests
//! read it; the value is identical either way.

use morphir_daemon::extensions::loader::ExtensionLoader;
use morphir_daemon::extensions::virtual_paths::VirtualPathConfig;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static MORPHIR_HOME: OnceLock<PathBuf> = OnceLock::new();

fn morphir_home() -> &'static Path {
    MORPHIR_HOME.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        // SAFETY: set once, before any test reads it; both tests race to this
        // same initializer so no reader observes a partial state.
        unsafe { std::env::set_var("MORPHIR_HOME", &dir) };
        dir
    })
}

#[test]
fn default_extension_cache_lives_under_relocated_home() {
    let home = morphir_home();

    let loader = ExtensionLoader::with_default_cache().unwrap();
    assert_eq!(loader.cache_dir(), home.join("cache").join("extensions"));
}

#[test]
fn virtual_cache_path_maps_to_relocated_home() {
    let home = morphir_home();

    let config = VirtualPathConfig::for_workspace(Path::new("/ws"), Path::new("/out"));
    assert_eq!(
        config.get_mapping("/cache"),
        Some(home.join("cache").as_path())
    );
}
