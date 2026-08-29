//! Resolution of the user-level Morphir home directory.
//!
//! The Morphir home directory holds user-global state such as the tool,
//! extension, and distribution registries, component data, caches, and logs.
//! Advanced users can relocate it (e.g. for testing or sandboxed environments)
//! by setting the `MORPHIR_HOME` environment variable.
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

    /// Path of the global Morphir configuration file.
    pub fn global_config_file(&self) -> PathBuf {
        self.root.join("morphir.toml")
    }

    /// Directory for component-owned configuration.
    pub fn config_dir(&self) -> PathBuf {
        self.root.join("config")
    }

    /// Directory for durable component-owned application data.
    pub fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }

    /// Directory for catalogs maintained by Morphir's distribution kernel.
    pub fn catalog_dir(&self) -> PathBuf {
        self.root.join("catalog")
    }

    /// Directory for verified, content-addressed artifacts.
    pub fn store_dir(&self) -> PathBuf {
        self.root.join("store")
    }

    /// Directory for interprocess coordination locks.
    pub fn locks_dir(&self) -> PathBuf {
        self.root.join("locks")
    }

    /// Directory for disposable cached content.
    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// Directory for downloaded artifacts that can be reacquired.
    pub fn downloads_cache_dir(&self) -> PathBuf {
        self.cache_dir().join("downloads")
    }

    /// Directory for cached registry and release indexes.
    pub fn indexes_cache_dir(&self) -> PathBuf {
        self.cache_dir().join("indexes")
    }

    /// Directory for Desktop-owned re-creatable application caches.
    pub fn desktop_cache_dir(&self) -> PathBuf {
        self.cache_dir().join("desktop")
    }

    /// Directory for component log output.
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// Directory for temporary staging owned by Morphir processes.
    pub fn temp_dir(&self) -> PathBuf {
        self.root.join("tmp")
    }

    /// Durable state used by automatic cache maintenance.
    pub fn cache_maintenance_state_file(&self) -> PathBuf {
        self.data_dir().join("maintenance/cache-cleanup.json")
    }

    /// Interprocess lock shared by manual and automatic maintenance runs.
    pub fn maintenance_lock_file(&self) -> PathBuf {
        self.locks_dir().join("maintenance.lock")
    }

    /// Temporary destination for entries selected for atomic cleanup.
    pub fn maintenance_trash_dir(&self) -> PathBuf {
        self.temp_dir().join("maintenance-trash")
    }

    /// Directory for CLI log output.
    pub fn cli_logs_dir(&self) -> PathBuf {
        self.logs_dir().join("cli")
    }

    /// Directory for Desktop log and crash output.
    pub fn desktop_logs_dir(&self) -> PathBuf {
        self.logs_dir().join("desktop")
    }

    /// Path of the legacy tool-intent registry file.
    pub fn tools_file(&self) -> PathBuf {
        self.root.join("tools.json")
    }

    /// Path of the legacy distribution registry file.
    pub fn distributions_file(&self) -> PathBuf {
        self.root.join("distributions.json")
    }

    /// Path of the legacy extension registry file.
    pub fn extensions_file(&self) -> PathBuf {
        self.root.join("extensions.json")
    }

    /// Durable catalog of installed tool artifacts.
    pub fn tools_catalog_file(&self) -> PathBuf {
        self.catalog_dir().join("tools.json")
    }

    /// Durable catalog of installed distributions.
    pub fn distributions_catalog_file(&self) -> PathBuf {
        self.catalog_dir().join("distributions.json")
    }

    /// Content-addressed store for verified extension artifacts.
    pub fn extensions_store_dir(&self) -> PathBuf {
        self.store_dir().join("extensions/sha256")
    }

    /// Durable catalog of installed extension artifacts.
    pub fn extensions_catalog_file(&self) -> PathBuf {
        self.catalog_dir().join("extensions.json")
    }

    /// Directory containing exact extension selection locks.
    pub fn extensions_locks_dir(&self) -> PathBuf {
        self.locks_dir().join("extensions")
    }

    /// Interprocess lock serializing installed extension state transactions.
    pub fn extensions_state_lock_file(&self) -> PathBuf {
        self.locks_dir().join("extensions.state.lock")
    }
}

/// Root directory for Morphir caches.
///
/// Caches always live under `<MORPHIR_HOME>/cache`, including when Morphir Home
/// uses its default location. Returns `None` only when neither `MORPHIR_HOME`
/// nor an OS user home directory is available.
pub fn cache_root() -> Option<PathBuf> {
    cache_root_from(
        std::env::var_os(MORPHIR_HOME_ENV).as_deref(),
        dirs::home_dir(),
    )
}

/// Pure form of [`cache_root`] taking the raw `MORPHIR_HOME` value (if set)
/// and the OS user home directory (if known).
pub fn cache_root_from(env_value: Option<&OsStr>, os_home: Option<PathBuf>) -> Option<PathBuf> {
    MorphirHome::resolve_from(env_value, os_home)
        .ok()
        .map(|home| home.cache_dir())
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
        assert_eq!(home.global_config_file(), Path::new("/mh/morphir.toml"));
        assert_eq!(home.config_dir(), Path::new("/mh/config"));
        assert_eq!(home.data_dir(), Path::new("/mh/data"));
        assert_eq!(home.catalog_dir(), Path::new("/mh/catalog"));
        assert_eq!(home.store_dir(), Path::new("/mh/store"));
        assert_eq!(home.locks_dir(), Path::new("/mh/locks"));
        assert_eq!(home.cache_dir(), Path::new("/mh/cache"));
        assert_eq!(home.logs_dir(), Path::new("/mh/logs"));
        assert_eq!(home.temp_dir(), Path::new("/mh/tmp"));
        assert_eq!(home.cli_logs_dir(), Path::new("/mh/logs/cli"));
        assert_eq!(home.desktop_logs_dir(), Path::new("/mh/logs/desktop"));
        assert_eq!(home.downloads_cache_dir(), Path::new("/mh/cache/downloads"));
        assert_eq!(home.indexes_cache_dir(), Path::new("/mh/cache/indexes"));
        assert_eq!(home.desktop_cache_dir(), Path::new("/mh/cache/desktop"));
        assert_eq!(
            home.cache_maintenance_state_file(),
            Path::new("/mh/data/maintenance/cache-cleanup.json")
        );
        assert_eq!(
            home.maintenance_lock_file(),
            Path::new("/mh/locks/maintenance.lock")
        );
        assert_eq!(
            home.maintenance_trash_dir(),
            Path::new("/mh/tmp/maintenance-trash")
        );
        assert_eq!(home.tools_file(), Path::new("/mh/tools.json"));
        assert_eq!(
            home.distributions_file(),
            Path::new("/mh/distributions.json")
        );
        assert_eq!(home.extensions_file(), Path::new("/mh/extensions.json"));
        assert_eq!(
            home.tools_catalog_file(),
            Path::new("/mh/catalog/tools.json")
        );
        assert_eq!(
            home.distributions_catalog_file(),
            Path::new("/mh/catalog/distributions.json")
        );
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
    fn default_caches_stay_in_morphir_home() {
        assert_eq!(
            cache_root_from(None, Some("/home/u".into())),
            Some(PathBuf::from("/home/u/.morphir/cache"))
        );
    }

    #[test]
    fn relocated_home_keeps_caches_under_home() {
        assert_eq!(
            cache_root_from(Some(os("/sandbox/mh").as_os_str()), Some("/home/u".into())),
            Some(PathBuf::from("/sandbox/mh/cache"))
        );
    }

    #[test]
    fn empty_env_var_keeps_caches_in_morphir_home() {
        assert_eq!(
            cache_root_from(Some(os("").as_os_str()), Some("/home/u".into())),
            Some(PathBuf::from("/home/u/.morphir/cache"))
        );
    }

    #[test]
    fn no_env_var_and_no_os_home_yields_none() {
        assert_eq!(cache_root_from(None, None), None);
    }
}
