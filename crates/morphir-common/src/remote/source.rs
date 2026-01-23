//! Remote source type definitions and parsing.

use crate::remote::error::{RemoteSourceError, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// Represents a parsed remote source specification.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum RemoteSource {
    /// Local file path
    Local {
        /// Path to the local file or directory
        path: PathBuf,
    },

    /// HTTP/HTTPS URL
    Http {
        /// The URL to fetch
        url: String,
        /// Optional path within an archive
        subpath: Option<String>,
    },

    /// Git repository
    Git {
        /// Git URL (HTTPS or SSH)
        url: String,
        /// Git reference (branch, tag, or commit)
        reference: Option<GitRef>,
        /// Optional path within the repository
        subpath: Option<String>,
    },

    /// GitHub shorthand (github:user/repo)
    GitHub {
        /// Repository owner
        owner: String,
        /// Repository name
        repo: String,
        /// Git reference (branch, tag, or commit)
        reference: Option<GitRef>,
        /// Optional path within the repository
        subpath: Option<String>,
    },

    /// GitHub Gist (gist:id or gist:user/id)
    Gist {
        /// Gist ID
        id: String,
        /// Gist revision (commit SHA)
        revision: Option<String>,
        /// Specific file within the gist
        filename: Option<String>,
    },
}

/// Git reference type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum GitRef {
    /// Branch name
    Branch(String),
    /// Tag name
    Tag(String),
    /// Commit SHA
    Commit(String),
}

impl RemoteSource {
    /// Parse a source string into a RemoteSource.
    ///
    /// Supported formats:
    /// - Local path: `./path/to/file.json`, `/absolute/path`, `relative/path`
    /// - HTTP/HTTPS: `https://example.com/file.json`, `http://example.com/archive.zip#path/in/archive`
    /// - Git HTTPS: `https://github.com/org/repo.git`
    /// - Git SSH: `git@github.com:org/repo.git`
    /// - GitHub shorthand: `github:owner/repo`, `github:owner/repo@tag`, `github:owner/repo/path/to/file`
    /// - Gist: `gist:abc123`, `gist:user/abc123`, `gist:abc123#filename.json`
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();

        if input.is_empty() {
            return Err(RemoteSourceError::InvalidFormat(
                "Empty source string".to_string(),
            ));
        }

        // Check for scheme prefixes
        if let Some(rest) = input.strip_prefix("github:") {
            return Self::parse_github(rest);
        }

        if let Some(rest) = input.strip_prefix("gist:") {
            return Self::parse_gist(rest);
        }

        if input.starts_with("http://") || input.starts_with("https://") {
            return Self::parse_http(input);
        }

        if input.starts_with("git@") || input.ends_with(".git") {
            return Self::parse_git(input);
        }

        // Default to local path
        Ok(RemoteSource::Local {
            path: PathBuf::from(input),
        })
    }

    /// Parse an HTTP/HTTPS URL.
    fn parse_http(input: &str) -> Result<Self> {
        // Check if it's a .git URL (clone URL)
        if input.ends_with(".git") {
            return Self::parse_git(input);
        }

        // Check for fragment (subpath in archive)
        let (url, subpath) = if let Some(hash_pos) = input.find('#') {
            let (url_part, path_part) = input.split_at(hash_pos);
            (url_part.to_string(), Some(path_part[1..].to_string()))
        } else {
            (input.to_string(), None)
        };

        Ok(RemoteSource::Http { url, subpath })
    }

    /// Parse a Git URL.
    fn parse_git(input: &str) -> Result<Self> {
        let mut url = input.to_string();
        let mut reference = None;
        let mut subpath = None;

        // Check for @ref suffix (before any path)
        if let Some(at_pos) = url.rfind('@') {
            // Make sure @ is not part of git@... prefix
            if !url[..at_pos].contains("://") && url.starts_with("git@") {
                // This is git@host:... format, @ is part of URL
            } else if at_pos > url.find("://").unwrap_or(0) {
                let ref_part = url[at_pos + 1..].to_string();
                url = url[..at_pos].to_string();
                reference = Some(Self::parse_git_ref(&ref_part));
            }
        }

        // Check for path after .git
        if let Some(git_pos) = url.find(".git") {
            let after_git = &url[git_pos + 4..];
            if !after_git.is_empty() && after_git.starts_with('/') {
                subpath = Some(after_git[1..].to_string());
                url = url[..git_pos + 4].to_string();
            }
        }

        Ok(RemoteSource::Git {
            url,
            reference,
            subpath,
        })
    }

    /// Parse a GitHub shorthand.
    fn parse_github(input: &str) -> Result<Self> {
        if input.is_empty() {
            return Err(RemoteSourceError::InvalidFormat(
                "Empty GitHub reference".to_string(),
            ));
        }

        let mut parts = input.to_string();
        let mut reference = None;
        let mut subpath = None;

        // Check for @ref suffix
        if let Some(at_pos) = parts.find('@') {
            let ref_part = parts[at_pos + 1..].to_string();
            parts = parts[..at_pos].to_string();
            reference = Some(Self::parse_git_ref(&ref_part));
        }

        // Split by /
        let segments: Vec<&str> = parts.split('/').collect();

        if segments.len() < 2 {
            return Err(RemoteSourceError::InvalidFormat(format!(
                "Invalid GitHub reference: {}. Expected format: owner/repo[/path][@ref]",
                input
            )));
        }

        let owner = segments[0].to_string();
        let repo = segments[1].to_string();

        // Remaining segments form the subpath
        if segments.len() > 2 {
            subpath = Some(segments[2..].join("/"));
        }

        Ok(RemoteSource::GitHub {
            owner,
            repo,
            reference,
            subpath,
        })
    }

    /// Parse a Gist reference.
    fn parse_gist(input: &str) -> Result<Self> {
        if input.is_empty() {
            return Err(RemoteSourceError::InvalidFormat(
                "Empty Gist reference".to_string(),
            ));
        }

        let mut parts = input.to_string();
        let mut filename = None;
        let mut revision = None;

        // Check for #filename suffix
        if let Some(hash_pos) = parts.find('#') {
            filename = Some(parts[hash_pos + 1..].to_string());
            parts = parts[..hash_pos].to_string();
        }

        // Check for @revision suffix
        if let Some(at_pos) = parts.find('@') {
            revision = Some(parts[at_pos + 1..].to_string());
            parts = parts[..at_pos].to_string();
        }

        // Gist ID can be just the ID or user/id format
        let id = if parts.contains('/') {
            // user/id format - extract just the id
            parts.split('/').next_back().unwrap_or(&parts).to_string()
        } else {
            parts
        };

        Ok(RemoteSource::Gist {
            id,
            revision,
            filename,
        })
    }

    /// Parse a git reference string into a GitRef.
    fn parse_git_ref(ref_str: &str) -> GitRef {
        // Check if it looks like a commit SHA (40 hex chars)
        if ref_str.len() == 40 && ref_str.chars().all(|c| c.is_ascii_hexdigit()) {
            GitRef::Commit(ref_str.to_string())
        } else if ref_str.starts_with('v')
            && ref_str[1..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit())
        {
            // Looks like a version tag (v1.0.0, v2, etc.)
            GitRef::Tag(ref_str.to_string())
        } else {
            // Default to branch
            GitRef::Branch(ref_str.to_string())
        }
    }

    /// Generate a unique cache key for this source.
    pub fn cache_key(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// Check if this source is local.
    pub fn is_local(&self) -> bool {
        matches!(self, RemoteSource::Local { .. })
    }

    /// Check if this source requires network access.
    pub fn requires_network(&self) -> bool {
        !self.is_local()
    }

    /// Get the display name for this source type.
    pub fn source_type(&self) -> &'static str {
        match self {
            RemoteSource::Local { .. } => "local",
            RemoteSource::Http { .. } => "http",
            RemoteSource::Git { .. } => "git",
            RemoteSource::GitHub { .. } => "github",
            RemoteSource::Gist { .. } => "gist",
        }
    }

    /// Convert to a canonical URL string (for matching allow/deny patterns).
    pub fn to_url_string(&self) -> String {
        match self {
            RemoteSource::Local { path } => format!("file://{}", path.display()),
            RemoteSource::Http { url, subpath } => {
                if let Some(sub) = subpath {
                    format!("{}#{}", url, sub)
                } else {
                    url.clone()
                }
            }
            RemoteSource::Git {
                url,
                reference,
                subpath,
            } => {
                let mut result = url.clone();
                if let Some(ref_) = reference {
                    result = format!("{}@{}", result, ref_);
                }
                if let Some(sub) = subpath {
                    result = format!("{}/{}", result, sub);
                }
                result
            }
            RemoteSource::GitHub {
                owner,
                repo,
                reference,
                subpath,
            } => {
                let mut result = format!("https://github.com/{}/{}", owner, repo);
                if let Some(sub) = subpath {
                    result = format!("{}/{}", result, sub);
                }
                if let Some(ref_) = reference {
                    result = format!("{}@{}", result, ref_);
                }
                result
            }
            RemoteSource::Gist {
                id,
                revision,
                filename,
            } => {
                let mut result = format!("https://gist.github.com/{}", id);
                if let Some(rev) = revision {
                    result = format!("{}/{}", result, rev);
                }
                if let Some(file) = filename {
                    result = format!("{}#{}", result, file);
                }
                result
            }
        }
    }
}

impl fmt::Display for RemoteSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemoteSource::Local { path } => write!(f, "{}", path.display()),
            RemoteSource::Http { url, subpath } => {
                if let Some(sub) = subpath {
                    write!(f, "{}#{}", url, sub)
                } else {
                    write!(f, "{}", url)
                }
            }
            RemoteSource::Git {
                url,
                reference,
                subpath,
            } => {
                write!(f, "{}", url)?;
                if let Some(ref_) = reference {
                    write!(f, "@{}", ref_)?;
                }
                if let Some(sub) = subpath {
                    write!(f, "/{}", sub)?;
                }
                Ok(())
            }
            RemoteSource::GitHub {
                owner,
                repo,
                reference,
                subpath,
            } => {
                write!(f, "github:{}/{}", owner, repo)?;
                if let Some(sub) = subpath {
                    write!(f, "/{}", sub)?;
                }
                if let Some(ref_) = reference {
                    write!(f, "@{}", ref_)?;
                }
                Ok(())
            }
            RemoteSource::Gist {
                id,
                revision,
                filename,
            } => {
                write!(f, "gist:{}", id)?;
                if let Some(rev) = revision {
                    write!(f, "@{}", rev)?;
                }
                if let Some(file) = filename {
                    write!(f, "#{}", file)?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for GitRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GitRef::Branch(name) => write!(f, "{}", name),
            GitRef::Tag(name) => write!(f, "{}", name),
            GitRef::Commit(sha) => write!(f, "{}", sha),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_local_path() {
        let source = RemoteSource::parse("./morphir-ir.json").unwrap();
        assert!(
            matches!(source, RemoteSource::Local { path } if path == PathBuf::from("./morphir-ir.json"))
        );

        let source = RemoteSource::parse("/absolute/path/to/file.json").unwrap();
        assert!(
            matches!(source, RemoteSource::Local { path } if path == PathBuf::from("/absolute/path/to/file.json"))
        );
    }

    #[test]
    fn test_parse_http_url() {
        let source = RemoteSource::parse("https://example.com/morphir-ir.json").unwrap();
        assert!(
            matches!(source, RemoteSource::Http { url, subpath: None } if url == "https://example.com/morphir-ir.json")
        );

        let source =
            RemoteSource::parse("https://example.com/archive.zip#path/to/ir.json").unwrap();
        assert!(
            matches!(source, RemoteSource::Http { url, subpath: Some(sub) }
            if url == "https://example.com/archive.zip" && sub == "path/to/ir.json")
        );
    }

    #[test]
    fn test_parse_github_shorthand() {
        let source = RemoteSource::parse("github:finos/morphir-examples").unwrap();
        assert!(
            matches!(source, RemoteSource::GitHub { owner, repo, reference: None, subpath: None }
            if owner == "finos" && repo == "morphir-examples")
        );

        let source = RemoteSource::parse("github:finos/morphir-examples@v1.0").unwrap();
        assert!(
            matches!(source, RemoteSource::GitHub { owner, repo, reference: Some(GitRef::Tag(t)), subpath: None }
            if owner == "finos" && repo == "morphir-examples" && t == "v1.0")
        );

        let source = RemoteSource::parse("github:finos/morphir-examples/examples/basic").unwrap();
        assert!(
            matches!(source, RemoteSource::GitHub { owner, repo, reference: None, subpath: Some(sub) }
            if owner == "finos" && repo == "morphir-examples" && sub == "examples/basic")
        );

        let source =
            RemoteSource::parse("github:finos/morphir-examples/examples/basic@main").unwrap();
        assert!(
            matches!(source, RemoteSource::GitHub { owner, repo, reference: Some(GitRef::Branch(b)), subpath: Some(sub) }
            if owner == "finos" && repo == "morphir-examples" && sub == "examples/basic" && b == "main")
        );
    }

    #[test]
    fn test_parse_gist() {
        let source = RemoteSource::parse("gist:abc123").unwrap();
        assert!(
            matches!(source, RemoteSource::Gist { id, revision: None, filename: None } if id == "abc123")
        );

        let source = RemoteSource::parse("gist:user/abc123").unwrap();
        assert!(
            matches!(source, RemoteSource::Gist { id, revision: None, filename: None } if id == "abc123")
        );

        let source = RemoteSource::parse("gist:abc123#morphir-ir.json").unwrap();
        assert!(
            matches!(source, RemoteSource::Gist { id, revision: None, filename: Some(f) }
            if id == "abc123" && f == "morphir-ir.json")
        );

        let source = RemoteSource::parse("gist:abc123@rev456#file.json").unwrap();
        assert!(
            matches!(source, RemoteSource::Gist { id, revision: Some(r), filename: Some(f) }
            if id == "abc123" && r == "rev456" && f == "file.json")
        );
    }

    #[test]
    fn test_parse_git_url() {
        let source = RemoteSource::parse("https://github.com/finos/morphir.git").unwrap();
        assert!(
            matches!(source, RemoteSource::Git { url, reference: None, subpath: None }
            if url == "https://github.com/finos/morphir.git")
        );

        let source = RemoteSource::parse("git@github.com:finos/morphir.git").unwrap();
        assert!(
            matches!(source, RemoteSource::Git { url, reference: None, subpath: None }
            if url == "git@github.com:finos/morphir.git")
        );
    }

    #[test]
    fn test_is_local() {
        let local = RemoteSource::parse("./file.json").unwrap();
        assert!(local.is_local());

        let remote = RemoteSource::parse("https://example.com/file.json").unwrap();
        assert!(!remote.is_local());
    }

    #[test]
    fn test_source_type() {
        assert_eq!(
            RemoteSource::parse("./file.json").unwrap().source_type(),
            "local"
        );
        assert_eq!(
            RemoteSource::parse("https://example.com/file.json")
                .unwrap()
                .source_type(),
            "http"
        );
        assert_eq!(
            RemoteSource::parse("github:user/repo")
                .unwrap()
                .source_type(),
            "github"
        );
        assert_eq!(
            RemoteSource::parse("gist:abc123").unwrap().source_type(),
            "gist"
        );
    }

    #[test]
    fn test_display() {
        let source = RemoteSource::parse("github:finos/morphir-examples/path@v1.0").unwrap();
        assert_eq!(
            source.to_string(),
            "github:finos/morphir-examples/path@v1.0"
        );
    }
}
