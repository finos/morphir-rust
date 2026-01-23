//! Configuration for remote source access.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

/// Configuration for remote source access.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSourceConfig {
    /// Whether remote sources are enabled.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Allow list (glob patterns). If non-empty, only matching sources allowed.
    #[serde(default)]
    pub allow: Vec<String>,

    /// Deny list (glob patterns). Takes precedence over allow.
    #[serde(default)]
    pub deny: Vec<String>,

    /// Trusted GitHub organizations/users.
    #[serde(default)]
    pub trusted_github_orgs: HashSet<String>,

    /// Cache settings.
    #[serde(default)]
    pub cache: CacheConfig,

    /// Network settings.
    #[serde(default)]
    pub network: NetworkConfig,
}

impl Default for RemoteSourceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow: Vec::new(),
            deny: Vec::new(),
            trusted_github_orgs: HashSet::new(),
            cache: CacheConfig::default(),
            network: NetworkConfig::default(),
        }
    }
}

/// Cache configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheConfig {
    /// Cache directory (defaults to ~/.cache/morphir/sources).
    pub directory: Option<PathBuf>,

    /// Maximum cache size in MB (0 = unlimited).
    #[serde(default)]
    pub max_size_mb: u64,

    /// TTL for cached sources in seconds (0 = never expire).
    #[serde(default)]
    pub ttl_secs: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            directory: None,
            max_size_mb: 0,
            ttl_secs: 0,
        }
    }
}

/// Network configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkConfig {
    /// Connection timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    /// HTTP proxy URL.
    pub http_proxy: Option<String>,

    /// HTTPS proxy URL.
    pub https_proxy: Option<String>,

    /// Maximum number of redirects to follow.
    #[serde(default = "default_max_redirects")]
    pub max_redirects: u32,

    /// User agent string.
    pub user_agent: Option<String>,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            timeout_secs: default_timeout(),
            http_proxy: None,
            https_proxy: None,
            max_redirects: default_max_redirects(),
            user_agent: None,
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_timeout() -> u64 {
    30
}

fn default_max_redirects() -> u32 {
    10
}

impl RemoteSourceConfig {
    /// Create a new config with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a config with remote sources disabled.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::default()
        }
    }

    /// Check if a URL matches the allow/deny patterns.
    pub fn is_allowed(&self, url: &str) -> bool {
        if !self.enabled {
            return false;
        }

        // Check deny list first (deny takes precedence)
        for pattern in &self.deny {
            if Self::matches_pattern(pattern, url) {
                return false;
            }
        }

        // If allow list is empty, allow everything not denied
        if self.allow.is_empty() {
            return true;
        }

        // Check allow list
        for pattern in &self.allow {
            if Self::matches_pattern(pattern, url) {
                return true;
            }
        }

        false
    }

    /// Check if a GitHub org/user is trusted.
    pub fn is_trusted_github_org(&self, org: &str) -> bool {
        self.trusted_github_orgs.contains(org)
    }

    /// Simple glob pattern matching.
    fn matches_pattern(pattern: &str, url: &str) -> bool {
        // Use the glob crate for pattern matching
        if let Ok(glob_pattern) = glob::Pattern::new(pattern) {
            glob_pattern.matches(url)
        } else {
            // Fall back to simple prefix/suffix matching
            if pattern.starts_with('*') && pattern.ends_with('*') {
                let middle = &pattern[1..pattern.len() - 1];
                url.contains(middle)
            } else if pattern.starts_with('*') {
                url.ends_with(&pattern[1..])
            } else if pattern.ends_with('*') {
                url.starts_with(&pattern[..pattern.len() - 1])
            } else {
                url == pattern
            }
        }
    }

    /// Get the cache directory, using default if not specified.
    pub fn cache_directory(&self) -> PathBuf {
        self.cache.directory.clone().unwrap_or_else(|| {
            dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from(".cache"))
                .join("morphir")
                .join("sources")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = RemoteSourceConfig::default();
        assert!(config.enabled);
        assert!(config.allow.is_empty());
        assert!(config.deny.is_empty());
    }

    #[test]
    fn test_disabled_config() {
        let config = RemoteSourceConfig::disabled();
        assert!(!config.enabled);
        assert!(!config.is_allowed("https://example.com"));
    }

    #[test]
    fn test_allow_list() {
        let config = RemoteSourceConfig {
            enabled: true,
            allow: vec!["https://github.com/*".to_string()],
            deny: vec![],
            ..Default::default()
        };

        assert!(config.is_allowed("https://github.com/finos/morphir"));
        assert!(!config.is_allowed("https://gitlab.com/other/repo"));
    }

    #[test]
    fn test_deny_takes_precedence() {
        let config = RemoteSourceConfig {
            enabled: true,
            allow: vec!["https://github.com/*".to_string()],
            deny: vec!["https://github.com/untrusted/*".to_string()],
            ..Default::default()
        };

        assert!(config.is_allowed("https://github.com/finos/morphir"));
        assert!(!config.is_allowed("https://github.com/untrusted/repo"));
    }

    #[test]
    fn test_trusted_github_orgs() {
        let mut orgs = HashSet::new();
        orgs.insert("finos".to_string());

        let config = RemoteSourceConfig {
            trusted_github_orgs: orgs,
            ..Default::default()
        };

        assert!(config.is_trusted_github_org("finos"));
        assert!(!config.is_trusted_github_org("other"));
    }
}
