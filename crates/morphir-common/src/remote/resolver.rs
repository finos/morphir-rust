//! Remote source resolver.
//!
//! The resolver is the main entry point for fetching remote sources.
//! It handles caching, allow/deny lists, and delegates to the appropriate
//! fetcher based on the source type.

use crate::remote::cache::SourceCache;
use crate::remote::config::RemoteSourceConfig;
use crate::remote::error::{RemoteSourceError, Result};
use crate::remote::git::GitFetcher;
use crate::remote::http::{fetch_gist, HttpFetcher};
use crate::remote::source::RemoteSource;
use std::path::PathBuf;

/// Options for resolving a remote source.
#[derive(Debug, Clone, Default)]
pub struct ResolveOptions {
    /// Whether to use the cache.
    pub use_cache: bool,

    /// Force refresh even if cached.
    pub force_refresh: bool,

    /// Timeout override in seconds.
    pub timeout_secs: Option<u64>,
}

impl ResolveOptions {
    /// Create default options (use cache, no force refresh).
    pub fn new() -> Self {
        Self {
            use_cache: true,
            force_refresh: false,
            timeout_secs: None,
        }
    }

    /// Create options that skip the cache.
    pub fn no_cache() -> Self {
        Self {
            use_cache: false,
            force_refresh: false,
            timeout_secs: None,
        }
    }

    /// Create options that force refresh.
    pub fn force_refresh() -> Self {
        Self {
            use_cache: true,
            force_refresh: true,
            timeout_secs: None,
        }
    }
}

/// Remote source resolver.
///
/// The resolver fetches remote sources, handles caching, and enforces
/// allow/deny lists from the configuration.
pub struct RemoteSourceResolver {
    /// Configuration.
    config: RemoteSourceConfig,

    /// Source cache.
    cache: SourceCache,

    /// HTTP fetcher.
    http: HttpFetcher,

    /// Git fetcher.
    git: GitFetcher,
}

impl RemoteSourceResolver {
    /// Create a new resolver with the given configuration.
    pub fn new(config: RemoteSourceConfig) -> Result<Self> {
        let cache = SourceCache::new(config.cache.clone())?;
        let http = HttpFetcher::new(config.network.clone())?;
        let git = GitFetcher::new();

        Ok(Self {
            config,
            cache,
            http,
            git,
        })
    }

    /// Create a resolver with default configuration.
    pub fn with_defaults() -> Result<Self> {
        Self::new(RemoteSourceConfig::default())
    }

    /// Check if a source is allowed by the configuration.
    pub fn is_allowed(&self, source: &RemoteSource) -> bool {
        if !self.config.enabled {
            // Local sources are always allowed
            return source.is_local();
        }

        // Check trusted GitHub orgs
        if let RemoteSource::GitHub { owner, .. } = source {
            if self.config.is_trusted_github_org(owner) {
                return true;
            }
        }

        // Check URL patterns
        let url = source.to_url_string();
        self.config.is_allowed(&url)
    }

    /// Resolve a source string to a local path.
    ///
    /// This is a convenience method that parses the source and resolves it.
    pub fn resolve_string(&mut self, source_str: &str, options: &ResolveOptions) -> Result<PathBuf> {
        let source = RemoteSource::parse(source_str)?;
        self.resolve(&source, options)
    }

    /// Resolve a source to a local path.
    ///
    /// This method handles caching, allow/deny checks, and delegates to
    /// the appropriate fetcher based on the source type.
    pub fn resolve(&mut self, source: &RemoteSource, options: &ResolveOptions) -> Result<PathBuf> {
        // Check allow/deny lists
        if !self.is_allowed(source) {
            return Err(RemoteSourceError::NotAllowed(source.to_string()));
        }

        // Handle local sources directly
        if let RemoteSource::Local { path } = source {
            if !path.exists() {
                return Err(RemoteSourceError::NotFound(path.display().to_string()));
            }
            return Ok(path.clone());
        }

        // Check cache
        if options.use_cache && !options.force_refresh {
            if let Some(cached_path) = self.cache.get(source) {
                return Ok(cached_path);
            }
        }

        // Fetch based on source type
        match source {
            RemoteSource::Local { path } => {
                // Already handled above
                Ok(path.clone())
            }

            RemoteSource::Http { url, subpath } => {
                self.http.fetch_to_cache(url, subpath.as_deref(), &mut self.cache, source)
            }

            RemoteSource::Git { url, reference, subpath } => {
                self.git.clone_to_cache(
                    url,
                    reference.as_ref(),
                    subpath.as_deref(),
                    &mut self.cache,
                    source,
                )
            }

            RemoteSource::GitHub { owner, repo, reference, subpath } => {
                self.git.clone_github(
                    owner,
                    repo,
                    reference.as_ref(),
                    subpath.as_deref(),
                    &mut self.cache,
                    source,
                )
            }

            RemoteSource::Gist { id, revision, filename } => {
                fetch_gist(
                    &self.http,
                    id,
                    revision.as_deref(),
                    filename.as_deref(),
                    &mut self.cache,
                    source,
                )
            }
        }
    }

    /// Clear the cache.
    pub fn clear_cache(&mut self) -> Result<()> {
        self.cache.clear()
    }

    /// Get cache statistics.
    pub fn cache_stats(&self) -> crate::remote::cache::CacheStats {
        self.cache.stats()
    }

    /// Get the cache directory path.
    pub fn cache_directory(&self) -> PathBuf {
        self.config.cache_directory()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolver_creation() {
        let resolver = RemoteSourceResolver::with_defaults();
        assert!(resolver.is_ok());
    }

    #[test]
    fn test_local_source_allowed() {
        let resolver = RemoteSourceResolver::with_defaults().unwrap();
        let source = RemoteSource::parse("./local/file.json").unwrap();
        assert!(resolver.is_allowed(&source));
    }

    #[test]
    fn test_resolve_options() {
        let opts = ResolveOptions::new();
        assert!(opts.use_cache);
        assert!(!opts.force_refresh);

        let opts = ResolveOptions::no_cache();
        assert!(!opts.use_cache);

        let opts = ResolveOptions::force_refresh();
        assert!(opts.use_cache);
        assert!(opts.force_refresh);
    }

    #[test]
    fn test_trusted_github_org() {
        use std::collections::HashSet;

        let mut orgs = HashSet::new();
        orgs.insert("finos".to_string());

        let config = RemoteSourceConfig {
            trusted_github_orgs: orgs,
            ..Default::default()
        };

        let resolver = RemoteSourceResolver::new(config).unwrap();

        let source = RemoteSource::parse("github:finos/morphir").unwrap();
        assert!(resolver.is_allowed(&source));

        let source = RemoteSource::parse("github:other/repo").unwrap();
        // With no deny rules, other orgs are also allowed
        assert!(resolver.is_allowed(&source));
    }

    #[test]
    fn test_deny_list() {
        let config = RemoteSourceConfig {
            deny: vec!["https://evil.com/*".to_string()],
            ..Default::default()
        };

        let resolver = RemoteSourceResolver::new(config).unwrap();

        let source = RemoteSource::parse("https://evil.com/malware.json").unwrap();
        assert!(!resolver.is_allowed(&source));

        let source = RemoteSource::parse("https://good.com/data.json").unwrap();
        assert!(resolver.is_allowed(&source));
    }

    #[test]
    fn test_disabled_config() {
        let config = RemoteSourceConfig::disabled();
        let resolver = RemoteSourceResolver::new(config).unwrap();

        // Local sources should still be allowed
        let source = RemoteSource::parse("./local.json").unwrap();
        assert!(resolver.is_allowed(&source));

        // Remote sources should be blocked
        let source = RemoteSource::parse("https://example.com/file.json").unwrap();
        assert!(!resolver.is_allowed(&source));
    }
}
