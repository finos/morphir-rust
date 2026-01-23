//! HTTP/HTTPS fetching for remote sources.

use crate::remote::cache::SourceCache;
use crate::remote::config::NetworkConfig;
use crate::remote::error::{RemoteSourceError, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// HTTP client for fetching remote sources.
pub struct HttpFetcher {
    /// Network configuration.
    #[allow(dead_code)]
    config: NetworkConfig,

    /// reqwest client.
    client: reqwest::blocking::Client,
}

impl HttpFetcher {
    /// Create a new HTTP fetcher with the given configuration.
    pub fn new(config: NetworkConfig) -> Result<Self> {
        let mut builder = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .redirect(reqwest::redirect::Policy::limited(
                config.max_redirects as usize,
            ));

        // Set user agent
        if let Some(ref ua) = config.user_agent {
            builder = builder.user_agent(ua);
        } else {
            builder = builder.user_agent(format!("morphir-cli/{}", env!("CARGO_PKG_VERSION")));
        }

        // Set proxies
        if let Some(ref proxy_url) = config.http_proxy {
            if let Ok(proxy) = reqwest::Proxy::http(proxy_url) {
                builder = builder.proxy(proxy);
            }
        }

        if let Some(ref proxy_url) = config.https_proxy {
            if let Ok(proxy) = reqwest::Proxy::https(proxy_url) {
                builder = builder.proxy(proxy);
            }
        }

        let client = builder.build().map_err(|e| {
            RemoteSourceError::NetworkError(format!("Failed to create HTTP client: {}", e))
        })?;

        Ok(Self { config, client })
    }

    /// Create a new HTTP fetcher with default configuration.
    pub fn with_defaults() -> Result<Self> {
        Self::new(NetworkConfig::default())
    }

    /// Fetch a URL and return the response body as bytes.
    pub fn fetch_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|e| RemoteSourceError::NetworkError(format!("Request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(RemoteSourceError::HttpError {
                status: response.status().as_u16(),
                message: response.status().to_string(),
            });
        }

        response
            .bytes()
            .map(|b| b.to_vec())
            .map_err(|e| RemoteSourceError::NetworkError(format!("Failed to read response: {}", e)))
    }

    /// Fetch a URL and save to a file.
    pub fn fetch_to_file(&self, url: &str, dest: &PathBuf) -> Result<()> {
        let bytes = self.fetch_bytes(url)?;

        // Create parent directories
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(dest, bytes)?;
        Ok(())
    }

    /// Fetch a URL to cache, handling archives if needed.
    pub fn fetch_to_cache(
        &self,
        url: &str,
        subpath: Option<&str>,
        cache: &mut SourceCache,
        source: &crate::remote::source::RemoteSource,
    ) -> Result<PathBuf> {
        let bytes = self.fetch_bytes(url)?;

        // Check if this is an archive
        let is_archive = url.ends_with(".zip")
            || url.ends_with(".tar.gz")
            || url.ends_with(".tgz")
            || url.ends_with(".tar");

        if is_archive {
            // Extract to temp, then cache the extracted content
            let temp_dir = tempfile::tempdir()?;
            let archive_path = temp_dir.path().join("archive");
            std::fs::write(&archive_path, &bytes)?;

            let extract_dir = temp_dir.path().join("extracted");
            std::fs::create_dir_all(&extract_dir)?;

            self.extract_archive(&archive_path, &extract_dir, url)?;

            // If subpath specified, cache only that part
            let content_path = if let Some(sub) = subpath {
                let sub_path = extract_dir.join(sub);
                if !sub_path.exists() {
                    return Err(RemoteSourceError::PathNotFound {
                        path: sub.to_string(),
                        location: url.to_string(),
                    });
                }
                sub_path
            } else {
                extract_dir
            };

            cache.put(source, &content_path)
        } else {
            // Cache the file directly
            cache.put_bytes(source, &bytes)
        }
    }

    /// Extract an archive to a destination directory.
    fn extract_archive(&self, archive_path: &PathBuf, dest: &PathBuf, url: &str) -> Result<()> {
        if url.ends_with(".zip") {
            self.extract_zip(archive_path, dest)
        } else if url.ends_with(".tar.gz") || url.ends_with(".tgz") {
            self.extract_tar_gz(archive_path, dest)
        } else if url.ends_with(".tar") {
            self.extract_tar(archive_path, dest)
        } else {
            Err(RemoteSourceError::ArchiveError(format!(
                "Unknown archive format: {}",
                url
            )))
        }
    }

    /// Extract a zip archive.
    fn extract_zip(&self, archive_path: &Path, dest: &Path) -> Result<()> {
        let file = std::fs::File::open(archive_path)?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| RemoteSourceError::ArchiveError(format!("Failed to open zip: {}", e)))?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| {
                RemoteSourceError::ArchiveError(format!("Failed to read zip entry: {}", e))
            })?;

            let outpath = match file.enclosed_name() {
                Some(path) => dest.join(path),
                None => continue,
            };

            if file.is_dir() {
                std::fs::create_dir_all(&outpath)?;
            } else {
                if let Some(parent) = outpath.parent() {
                    if !parent.exists() {
                        std::fs::create_dir_all(parent)?;
                    }
                }

                let mut outfile = std::fs::File::create(&outpath)?;
                std::io::copy(&mut file, &mut outfile)?;
            }

            // Set permissions on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode))?;
                }
            }
        }

        Ok(())
    }

    /// Extract a tar.gz archive.
    fn extract_tar_gz(&self, archive_path: &PathBuf, dest: &PathBuf) -> Result<()> {
        let file = std::fs::File::open(archive_path)?;
        let gz = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(gz);

        archive.unpack(dest).map_err(|e| {
            RemoteSourceError::ArchiveError(format!("Failed to extract tar.gz: {}", e))
        })?;

        Ok(())
    }

    /// Extract a tar archive.
    fn extract_tar(&self, archive_path: &PathBuf, dest: &PathBuf) -> Result<()> {
        let file = std::fs::File::open(archive_path)?;
        let mut archive = tar::Archive::new(file);

        archive.unpack(dest).map_err(|e| {
            RemoteSourceError::ArchiveError(format!("Failed to extract tar: {}", e))
        })?;

        Ok(())
    }
}

/// Fetch a GitHub Gist.
pub fn fetch_gist(
    http: &HttpFetcher,
    id: &str,
    revision: Option<&str>,
    filename: Option<&str>,
    cache: &mut SourceCache,
    source: &crate::remote::source::RemoteSource,
) -> Result<PathBuf> {
    // Build the API URL
    let api_url = if let Some(rev) = revision {
        format!("https://api.github.com/gists/{}/{}", id, rev)
    } else {
        format!("https://api.github.com/gists/{}", id)
    };

    // Fetch gist metadata
    let response = http
        .client
        .get(&api_url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .map_err(|e| RemoteSourceError::NetworkError(format!("Gist API request failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(RemoteSourceError::HttpError {
            status: response.status().as_u16(),
            message: format!("Gist not found: {}", id),
        });
    }

    let gist: serde_json::Value = response.json().map_err(|e| {
        RemoteSourceError::NetworkError(format!("Failed to parse gist response: {}", e))
    })?;

    // Get the files
    let files = gist
        .get("files")
        .and_then(|f: &serde_json::Value| f.as_object())
        .ok_or_else(|| RemoteSourceError::NotFound(format!("Gist {} has no files", id)))?;

    if let Some(target_filename) = filename {
        // Fetch a specific file
        let file_info =
            files
                .get(target_filename)
                .ok_or_else(|| RemoteSourceError::PathNotFound {
                    path: target_filename.to_string(),
                    location: format!("gist:{}", id),
                })?;

        let raw_url = file_info
            .get("raw_url")
            .and_then(|u: &serde_json::Value| u.as_str())
            .ok_or_else(|| RemoteSourceError::NotFound("File has no raw_url".to_string()))?;

        let bytes = http.fetch_bytes(raw_url)?;
        cache.put_bytes(source, &bytes)
    } else {
        // Fetch all files
        let temp_dir = tempfile::tempdir()?;

        for (name, file_info) in files {
            if let Some(raw_url) = file_info
                .get("raw_url")
                .and_then(|u: &serde_json::Value| u.as_str())
            {
                let bytes = http.fetch_bytes(raw_url)?;
                let file_path = temp_dir.path().join(name);
                std::fs::write(&file_path, bytes)?;
            }
        }

        cache.put(source, temp_dir.path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_fetcher_creation() {
        let fetcher = HttpFetcher::with_defaults();
        assert!(fetcher.is_ok());
    }

    // Note: Additional tests would require a mock HTTP server
}
