//! Git operations for remote sources.
//!
//! This module uses the git CLI rather than a native Rust git library
//! for simplicity and to leverage the user's existing git configuration
//! (credentials, SSH keys, etc.).

use crate::remote::cache::SourceCache;
use crate::remote::error::{RemoteSourceError, Result};
use crate::remote::source::{GitRef, RemoteSource};
use std::path::PathBuf;
use std::process::Command;

/// Git operations handler.
pub struct GitFetcher {
    /// Whether to use shallow clones.
    shallow: bool,

    /// Clone depth for shallow clones.
    depth: u32,
}

impl GitFetcher {
    /// Create a new git fetcher.
    pub fn new() -> Self {
        Self {
            shallow: true,
            depth: 1,
        }
    }

    /// Create a git fetcher with full clone (not shallow).
    pub fn with_full_clone() -> Self {
        Self {
            shallow: false,
            depth: 0,
        }
    }

    /// Check if git is available.
    pub fn is_available(&self) -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Clone a git repository.
    pub fn clone_repo(
        &self,
        url: &str,
        reference: Option<&GitRef>,
        dest: &PathBuf,
    ) -> Result<()> {
        if !self.is_available() {
            return Err(RemoteSourceError::GitError(
                "git command not found. Please install git.".to_string(),
            ));
        }

        let mut cmd = Command::new("git");
        cmd.arg("clone");

        // Add shallow clone options if enabled and not cloning a specific commit
        if self.shallow {
            let can_shallow = match reference {
                Some(GitRef::Commit(_)) => false, // Need full clone for specific commit
                _ => true,
            };

            if can_shallow {
                cmd.arg("--depth").arg(self.depth.to_string());
            }
        }

        // Add branch/tag if specified
        if let Some(ref_) = reference {
            match ref_ {
                GitRef::Branch(name) | GitRef::Tag(name) => {
                    cmd.arg("--branch").arg(name);
                }
                GitRef::Commit(_) => {
                    // We'll checkout the commit after cloning
                }
            }
        }

        // Suppress progress output
        cmd.arg("--quiet");

        // Add URL and destination
        cmd.arg(url);
        cmd.arg(dest);

        let output = cmd
            .output()
            .map_err(|e| RemoteSourceError::GitError(format!("Failed to run git clone: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RemoteSourceError::GitError(format!(
                "git clone failed: {}",
                stderr.trim()
            )));
        }

        // Checkout specific commit if specified
        if let Some(GitRef::Commit(sha)) = reference {
            self.checkout(dest, sha)?;
        }

        Ok(())
    }

    /// Checkout a specific commit/branch/tag.
    fn checkout(&self, repo_path: &PathBuf, reference: &str) -> Result<()> {
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["checkout", "--quiet", reference])
            .output()
            .map_err(|e| RemoteSourceError::GitError(format!("Failed to run git checkout: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RemoteSourceError::GitError(format!(
                "git checkout failed: {}",
                stderr.trim()
            )));
        }

        Ok(())
    }

    /// Clone a repository to cache.
    pub fn clone_to_cache(
        &self,
        url: &str,
        reference: Option<&GitRef>,
        subpath: Option<&str>,
        cache: &mut SourceCache,
        source: &RemoteSource,
    ) -> Result<PathBuf> {
        let temp_dir = tempfile::tempdir()?;
        let clone_path = temp_dir.path().join("repo");

        self.clone_repo(url, reference, &clone_path)?;

        // Determine what to cache
        let content_path = if let Some(sub) = subpath {
            let sub_path = clone_path.join(sub);
            if !sub_path.exists() {
                return Err(RemoteSourceError::PathNotFound {
                    path: sub.to_string(),
                    location: url.to_string(),
                });
            }
            sub_path
        } else {
            clone_path
        };

        cache.put(source, &content_path)
    }

    /// Clone a GitHub repository using the shorthand.
    pub fn clone_github(
        &self,
        owner: &str,
        repo: &str,
        reference: Option<&GitRef>,
        subpath: Option<&str>,
        cache: &mut SourceCache,
        source: &RemoteSource,
    ) -> Result<PathBuf> {
        let url = format!("https://github.com/{}/{}.git", owner, repo);
        self.clone_to_cache(&url, reference, subpath, cache, source)
    }

    /// Fetch the latest changes for an existing clone.
    pub fn fetch(&self, repo_path: &PathBuf) -> Result<()> {
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["fetch", "--quiet"])
            .output()
            .map_err(|e| RemoteSourceError::GitError(format!("Failed to run git fetch: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RemoteSourceError::GitError(format!(
                "git fetch failed: {}",
                stderr.trim()
            )));
        }

        Ok(())
    }

    /// Pull the latest changes for an existing clone.
    pub fn pull(&self, repo_path: &PathBuf) -> Result<()> {
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["pull", "--quiet"])
            .output()
            .map_err(|e| RemoteSourceError::GitError(format!("Failed to run git pull: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RemoteSourceError::GitError(format!(
                "git pull failed: {}",
                stderr.trim()
            )));
        }

        Ok(())
    }

    /// Get the current commit SHA.
    pub fn current_commit(&self, repo_path: &PathBuf) -> Result<String> {
        let output = Command::new("git")
            .current_dir(repo_path)
            .args(["rev-parse", "HEAD"])
            .output()
            .map_err(|e| RemoteSourceError::GitError(format!("Failed to run git rev-parse: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RemoteSourceError::GitError(format!(
                "git rev-parse failed: {}",
                stderr.trim()
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

impl Default for GitFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_fetcher_creation() {
        let fetcher = GitFetcher::new();
        assert!(fetcher.shallow);
        assert_eq!(fetcher.depth, 1);
    }

    #[test]
    fn test_git_available() {
        let fetcher = GitFetcher::new();
        // Git should be available in most development environments
        // This is a soft test - it won't fail if git isn't installed
        let _available = fetcher.is_available();
    }

    // Note: Additional tests would require a real git repository
    // or a mock git command, which is complex to set up
}
