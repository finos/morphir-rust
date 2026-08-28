//! Resolution of the user-level Morphir home directory.
//!
//! The Morphir home directory holds user-global state such as the tool,
//! extension, and distribution registries and fallback log output. Advanced
//! users can relocate it (e.g. for testing or sandboxed environments) by
//! setting the `MORPHIR_HOME` environment variable. When relocated, caches
//! also move under the home directory so the environment stays hermetic.
//!
//! Resolution order:
//! 1. `MORPHIR_HOME` environment variable (an empty value is treated as unset)
//! 2. The OS-specific home directory joined with `.morphir`
//!    (`$HOME/.morphir` on Linux/macOS, `%USERPROFILE%\.morphir` on Windows)

use anyhow::{Result, anyhow};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Environment variable that relocates the Morphir home directory.
pub const MORPHIR_HOME_ENV: &str = "MORPHIR_HOME";

/// The resolved user-level Morphir home directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MorphirHome {
    root: PathBuf,
    relocated: bool,
}

impl MorphirHome {
    /// Resolve the Morphir home directory from the process environment.
    pub fn resolve() -> Result<Self> {
        Self::resolve_from(
            std::env::var_os(MORPHIR_HOME_ENV).as_deref(),
            dirs::home_dir(),
        )
    }

    /// Resolve the Morphir home directory from explicit inputs.
    ///
    /// `env_value` is the raw value of `MORPHIR_HOME` (if set) and `os_home`
    /// is the OS-reported user home directory (if known). Keeping this pure
    /// makes resolution testable without mutating process state.
    pub fn resolve_from(env_value: Option<&OsStr>, os_home: Option<PathBuf>) -> Result<Self> {
        match env_value.filter(|value| !value.is_empty()) {
            Some(value) => Ok(Self {
                root: PathBuf::from(value),
                relocated: true,
            }),
            None => os_home
                .map(|home| Self {
                    root: home.join(".morphir"),
                    relocated: false,
                })
                .ok_or_else(|| {
                    anyhow!(
                        "Could not determine the Morphir home directory: no home directory \
                         reported by the OS and {MORPHIR_HOME_ENV} is not set"
                    )
                }),
        }
    }

    /// The root of the Morphir home directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the home directory was explicitly relocated via `MORPHIR_HOME`.
    pub fn is_relocated(&self) -> bool {
        self.relocated
    }

    /// Path of the tool registry file.
    pub fn tools_file(&self) -> PathBuf {
        self.root.join("tools.json")
    }

    /// Path of the distribution registry file.
    pub fn distributions_file(&self) -> PathBuf {
        self.root.join("distributions.json")
    }

    /// Path of the extension registry file.
    pub fn extensions_file(&self) -> PathBuf {
        self.root.join("extensions.json")
    }

    /// Content-addressed store for verified extension artifacts.
    pub fn extensions_store_dir(&self) -> PathBuf {
        self.root.join("store/extensions/sha256")
    }

    /// Durable catalog of installed extension artifacts.
    pub fn extensions_catalog_file(&self) -> PathBuf {
        self.root.join("catalog/extensions.json")
    }

    /// Directory containing exact extension selection locks.
    pub fn extensions_locks_dir(&self) -> PathBuf {
        self.root.join("locks/extensions")
    }

    /// Interprocess lock serializing installed extension state transactions.
    pub fn extensions_state_lock_file(&self) -> PathBuf {
        self.root.join("locks/extensions.state.lock")
    }

    /// Directory for global (non-workspace) log output.
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }
}

/// Root directory for Morphir caches.
///
/// A relocated home keeps caches under `<home>/cache` so sandboxed
/// environments stay hermetic; otherwise the OS cache directory is used.
/// Returns `None` only when neither `MORPHIR_HOME` nor an OS cache directory
/// is available — callers keep their own fallback so environments without a
/// user home (e.g. service accounts with only `XDG_CACHE_HOME`) still work.
pub fn cache_root() -> Option<PathBuf> {
    cache_root_from(
        std::env::var_os(MORPHIR_HOME_ENV).as_deref(),
        dirs::cache_dir(),
    )
}

/// Pure form of [`cache_root`] taking the raw `MORPHIR_HOME` value (if set)
/// and the OS cache directory (if known).
pub fn cache_root_from(env_value: Option<&OsStr>, os_cache: Option<PathBuf>) -> Option<PathBuf> {
    match env_value.filter(|value| !value.is_empty()) {
        Some(value) => Some(PathBuf::from(value).join("cache")),
        None => os_cache.map(|cache| cache.join("morphir")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn os(s: &str) -> OsString {
        OsString::from(s)
    }

    #[test]
    fn env_var_relocates_home() {
        let home =
            MorphirHome::resolve_from(Some(os("/opt/morphir").as_os_str()), Some("/home/u".into()))
                .unwrap();
        assert_eq!(home.root(), Path::new("/opt/morphir"));
        assert!(home.is_relocated());
    }

    #[test]
    fn defaults_to_dot_morphir_under_os_home() {
        let home = MorphirHome::resolve_from(None, Some("/home/u".into())).unwrap();
        assert_eq!(home.root(), Path::new("/home/u/.morphir"));
        assert!(!home.is_relocated());
    }

    #[test]
    fn empty_env_var_is_treated_as_unset() {
        let home =
            MorphirHome::resolve_from(Some(os("").as_os_str()), Some("/home/u".into())).unwrap();
        assert_eq!(home.root(), Path::new("/home/u/.morphir"));
        assert!(!home.is_relocated());
    }

    #[test]
    fn errors_when_no_env_var_and_no_os_home() {
        let result = MorphirHome::resolve_from(None, None);
        assert!(result.is_err());
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains(MORPHIR_HOME_ENV),
            "error should mention {MORPHIR_HOME_ENV}, got: {message}"
        );
    }

    #[test]
    fn env_var_rescues_missing_os_home() {
        let home = MorphirHome::resolve_from(Some(os("/sandbox/mh").as_os_str()), None).unwrap();
        assert_eq!(home.root(), Path::new("/sandbox/mh"));
    }

    #[test]
    fn well_known_paths_live_under_the_home_root() {
        let home = MorphirHome::resolve_from(Some(os("/mh").as_os_str()), None).unwrap();
        assert_eq!(home.tools_file(), Path::new("/mh/tools.json"));
        assert_eq!(
            home.distributions_file(),
            Path::new("/mh/distributions.json")
        );
        assert_eq!(home.extensions_file(), Path::new("/mh/extensions.json"));
        assert_eq!(home.logs_dir(), Path::new("/mh/logs"));
        assert_eq!(
            home.extensions_store_dir(),
            Path::new("/mh/store/extensions/sha256")
        );
        assert_eq!(
            home.extensions_catalog_file(),
            Path::new("/mh/catalog/extensions.json")
        );
        assert_eq!(
            home.extensions_locks_dir(),
            Path::new("/mh/locks/extensions")
        );
        assert_eq!(
            home.extensions_state_lock_file(),
            Path::new("/mh/locks/extensions.state.lock")
        );
    }

    #[test]
    fn default_caches_stay_in_os_cache_dir() {
        assert_eq!(
            cache_root_from(None, Some("/home/u/.cache".into())),
            Some(PathBuf::from("/home/u/.cache/morphir"))
        );
    }

    #[test]
    fn relocated_home_keeps_caches_under_home() {
        assert_eq!(
            cache_root_from(
                Some(os("/sandbox/mh").as_os_str()),
                Some("/home/u/.cache".into())
            ),
            Some(PathBuf::from("/sandbox/mh/cache"))
        );
    }

    #[test]
    fn empty_env_var_keeps_caches_in_os_cache_dir() {
        assert_eq!(
            cache_root_from(Some(os("").as_os_str()), Some("/home/u/.cache".into())),
            Some(PathBuf::from("/home/u/.cache/morphir"))
        );
    }

    #[test]
    fn caches_resolve_without_a_user_home() {
        // A service account may have an OS cache dir but no home; no MorphirHome
        // resolution is required for caches.
        assert_eq!(
            cache_root_from(None, Some("/var/cache/svc".into())),
            Some(PathBuf::from("/var/cache/svc/morphir"))
        );
    }

    #[test]
    fn no_env_var_and_no_os_cache_dir_yields_none() {
        assert_eq!(cache_root_from(None, None), None);
    }
}
