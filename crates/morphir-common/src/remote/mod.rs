//! Remote source support for Morphir.
//!
//! This module provides functionality for fetching Morphir IR from remote sources,
//! including HTTP/HTTPS, Git repositories, GitHub shorthand, and GitHub Gists.
//!
//! # Overview
//!
//! The remote source system supports:
//!
//! - **Local paths**: `./morphir-ir.json`, `/absolute/path/to/file.json`
//! - **HTTP/HTTPS**: `https://example.com/morphir-ir.json`
//! - **Git repositories**: `https://github.com/org/repo.git`, `git@github.com:org/repo.git`
//! - **GitHub shorthand**: `github:owner/repo`, `github:owner/repo@tag`, `github:owner/repo/path`
//! - **GitHub Gists**: `gist:abc123`, `gist:abc123#filename.json`
//!
//! # Configuration
//!
//! Remote source access can be configured in `morphir.toml`:
//!
//! ```toml
//! [sources]
//! enabled = true
//! allow = ["github:finos/*", "https://artifacts.example.com/*"]
//! deny = ["*://untrusted.com/*"]
//! trusted_github_orgs = ["finos", "morphir-org"]
//!
//! [sources.cache]
//! directory = "~/.cache/morphir/sources"
//! max_size_mb = 500
//! ttl_secs = 86400  # 24 hours
//!
//! [sources.network]
//! timeout_secs = 30
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use morphir_common::remote::{RemoteSource, RemoteSourceResolver, ResolveOptions};
//!
//! let mut resolver = RemoteSourceResolver::with_defaults()?;
//!
//! // Resolve a source string to a local path
//! let local_path = resolver.resolve_string(
//!     "github:finos/morphir-examples/examples/basic",
//!     &ResolveOptions::new(),
//! )?;
//!
//! // Load the IR from the local path
//! let ir = load_distribution(&local_path)?;
//! ```
//!
//! # Caching
//!
//! Remote sources are cached locally to improve performance and reduce network usage.
//! The cache can be configured with size limits and TTL.

pub mod cache;
pub mod config;
pub mod error;
pub mod git;
pub mod http;
pub mod resolver;
pub mod source;

// Re-exports for convenience
pub use cache::{CacheEntry, CacheStats, SourceCache};
pub use config::{CacheConfig, NetworkConfig, RemoteSourceConfig};
pub use error::{RemoteSourceError, Result};
pub use resolver::{RemoteSourceResolver, ResolveOptions};
pub use source::{GitRef, RemoteSource};
