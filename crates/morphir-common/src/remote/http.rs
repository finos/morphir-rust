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
        if let Some(ref proxy_url) = config.http_proxy
            && let Ok(proxy) = reqwest::Proxy::http(proxy_url)
        {
            builder = builder.proxy(proxy);
        }

        if let Some(ref proxy_url) = config.https_proxy
            && let Ok(proxy) = reqwest::Proxy::https(proxy_url)
        {
            builder = builder.proxy(proxy);
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
        // `ZipArchive` reads names from the central directory only, and does not check them
        // against the local headers unless asked (GHSA-m665-v7pm-7grj).
        archive.validate_header_consistency().map_err(|e| {
            RemoteSourceError::ArchiveError(format!("Inconsistent zip headers: {}", e))
        })?;

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
                if let Some(parent) = outpath.parent()
                    && !parent.exists()
                {
                    std::fs::create_dir_all(parent)?;
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

    mod extract_zip {
        use super::super::{HttpFetcher, RemoteSourceError};
        use std::fs;
        use std::io::Write;
        use std::path::Path;
        use zip::write::SimpleFileOptions;

        const CENTRAL_DIRECTORY_HEADER: &[u8] = b"PK\x01\x02";
        const CENTRAL_NAME_OFFSET: usize = 46;

        fn extract(archive: &Path, destination: &Path) -> crate::remote::error::Result<()> {
            HttpFetcher::with_defaults()
                .unwrap()
                .extract_zip(archive, destination)
        }

        fn an_archive_with_entries(path: &Path, entries: &[(&str, &[u8])]) {
            let mut writer = zip::ZipWriter::new(fs::File::create(path).unwrap());
            for (name, bytes) in entries {
                writer
                    .start_file(*name, SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }

        /// One entry whose local header says `local_name` and whose central directory says
        /// `central_name`. The names must have the same length.
        fn an_archive_whose_headers_disagree_on_the_name(
            path: &Path,
            local_name: &str,
            central_name: &str,
        ) {
            assert_eq!(local_name.len(), central_name.len());
            an_archive_with_entries(path, &[(local_name, b"payload")]);
            let mut bytes = fs::read(path).unwrap();
            let central = bytes
                .windows(CENTRAL_DIRECTORY_HEADER.len())
                .position(|window| window == CENTRAL_DIRECTORY_HEADER)
                .unwrap()
                + CENTRAL_NAME_OFFSET;
            bytes[central..central + central_name.len()].copy_from_slice(central_name.as_bytes());
            fs::write(path, bytes).unwrap();
        }

        #[test]
        fn a_well_formed_archive_is_extracted() {
            let root = tempfile::tempdir().unwrap();
            let archive = root.path().join("source.zip");
            let destination = root.path().join("out");
            fs::create_dir(&destination).unwrap();
            an_archive_with_entries(&archive, &[("dir/file.txt", b"contents")]);

            extract(&archive, &destination).unwrap();

            assert_eq!(
                fs::read(destination.join("dir/file.txt")).unwrap(),
                b"contents"
            );
        }

        #[test]
        fn an_archive_whose_headers_disagree_on_a_name_is_rejected() {
            // zip 8.6.0, and 9.2.1 without the header check, extract this under the central name.
            let root = tempfile::tempdir().unwrap();
            let archive = root.path().join("cloaked.zip");
            let destination = root.path().join("out");
            fs::create_dir(&destination).unwrap();
            an_archive_whose_headers_disagree_on_the_name(&archive, "../a.txt", "safe.txt");

            let error = extract(&archive, &destination).unwrap_err();

            assert!(matches!(error, RemoteSourceError::ArchiveError(_)));
            assert!(!destination.join("safe.txt").exists());
            assert!(!root.path().join("a.txt").exists());
        }

        #[test]
        fn an_entry_that_escapes_the_root_is_skipped() {
            let root = tempfile::tempdir().unwrap();
            let archive = root.path().join("escape.zip");
            let destination = root.path().join("out");
            fs::create_dir(&destination).unwrap();
            an_archive_with_entries(
                &archive,
                &[("../escape.txt", b"escape"), ("safe.txt", b"safe")],
            );

            extract(&archive, &destination).unwrap();

            assert!(!root.path().join("escape.txt").exists());
            assert_eq!(fs::read(destination.join("safe.txt")).unwrap(), b"safe");
        }

        #[cfg(unix)]
        #[test]
        fn unix_modes_are_applied_and_a_symlink_entry_stays_a_regular_file() {
            use std::os::unix::fs::PermissionsExt;
            let root = tempfile::tempdir().unwrap();
            let archive = root.path().join("modes.zip");
            let destination = root.path().join("out");
            fs::create_dir(&destination).unwrap();
            let mut writer = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
            writer
                .start_file(
                    "run.sh",
                    SimpleFileOptions::default().unix_permissions(0o750),
                )
                .unwrap();
            writer.write_all(b"#!/bin/sh\n").unwrap();
            writer
                .add_symlink("link", "/etc/passwd", SimpleFileOptions::default())
                .unwrap();
            writer.finish().unwrap();

            extract(&archive, &destination).unwrap();

            assert_eq!(
                fs::metadata(destination.join("run.sh"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o750
            );
            let link = fs::symlink_metadata(destination.join("link")).unwrap();
            assert!(!link.file_type().is_symlink());
            assert_eq!(fs::read(destination.join("link")).unwrap(), b"/etc/passwd");
        }
    }
}
