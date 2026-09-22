//! Source identities within one compilation unit. These operations perform no I/O.
//!
//! ```
//! use morphir_extension_sdk::CompileRequest;
//! let request: CompileRequest = serde_json::from_value(serde_json::json!({
//!     "languageId": "python",
//!     "package": {"name": "acme/example", "exposedModules": ["domain.models"]},
//!     "sources": {
//!         "root": "file:///project/src",
//!         "documents": [{"uri": "file:///project/src/domain/models.py", "languageId": "python",
//!                        "version": 1, "text": ""}]
//!     },
//!     "options": {"irVersion": "4", "typesOnly": false}
//! })).unwrap();
//! assert_eq!(request.source_paths().unwrap()[0].as_str(), "domain/models.py");
//! ```

use crate::{CompileRequest, SourceSet};
use percent_encoding::percent_decode_str;
use std::collections::BTreeSet;
use url::Url;

/// Invalid source context supplied at the MEP boundary.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SourceContextError(String);

/// Absolute location against which a compilation's document identities are resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRoot(String);

impl SourceRoot {
    /// Parse an absolute hierarchical URI or legacy absolute filesystem path.
    pub fn parse(value: &str) -> Result<Self, SourceContextError> {
        let value = normalize(value)?;
        if !absolute(&value) {
            return Err(SourceContextError("sources.root must be absolute".into()));
        }
        Ok(Self(value))
    }

    /// The normalized location, without query or fragment metadata.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A nonempty relative document path within its compilation unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcePath(String);

impl SourcePath {
    /// Return the decoded, slash-separated path, including its file extension.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl SourceSet {
    /// Validate the root, if one was supplied.
    pub fn root(&self) -> Result<Option<SourceRoot>, SourceContextError> {
        self.root.as_deref().map(SourceRoot::parse).transpose()
    }
}

impl CompileRequest {
    /// Resolve and validate source identities in document order.
    ///
    /// Relative paths retain their nesting. Multiple absolute documents require
    /// a root; one absolute document without a root retains the legacy basename
    /// behavior. Original document URIs remain unchanged for diagnostics.
    pub fn source_paths(&self) -> Result<Vec<SourcePath>, SourceContextError> {
        let root = self.sources.root()?;
        let mut seen = BTreeSet::new();
        self.sources
            .documents
            .iter()
            .map(|document| {
                let normalized = normalize(&document.uri)?;
                let relative = if absolute(&normalized) {
                    match &root {
                        Some(root) => normalized
                            .strip_prefix(&format!(
                                "{}/",
                                root.as_str().strip_suffix('/').unwrap_or(root.as_str())
                            ))
                            .ok_or_else(|| {
                                SourceContextError("Source document is outside sources.root".into())
                            })?,
                        None if self.sources.documents.len() == 1 => {
                            normalized.rsplit('/').next().unwrap_or("")
                        }
                        None => {
                            return Err(SourceContextError(
                                "Absolute source URIs require sources.root for multiple modules"
                                    .into(),
                            ));
                        }
                    }
                } else {
                    &normalized
                };
                if relative.is_empty() || relative.split('/').any(str::is_empty) {
                    return Err(SourceContextError(
                        "Source document must identify a nonempty relative file path".into(),
                    ));
                }
                if !seen.insert(relative.to_owned()) {
                    return Err(SourceContextError(format!(
                        "Duplicate source document identity: {relative}"
                    )));
                }
                Ok(SourcePath(relative.to_owned()))
            })
            .collect()
    }
}

fn absolute(value: &str) -> bool {
    value.starts_with('/') || value.contains(':')
}

fn normalize(value: &str) -> Result<String, SourceContextError> {
    let raw = value
        .split(['?', '#'])
        .next()
        .unwrap_or(value)
        .replace('\\', "/");
    // URL parsing erases dot segments, so reject them before parsing.
    decode_path(&raw)?;
    let windows_drive =
        raw.as_bytes().get(1) == Some(&b':') && raw.as_bytes().get(2) == Some(&b'/');
    if !windows_drive && raw.contains(':') {
        let url = Url::parse(&raw)
            .map_err(|error| SourceContextError(format!("Invalid source URI: {error}")))?;
        if url.cannot_be_a_base() {
            return Err(SourceContextError(
                "Source URI must have a hierarchical path".into(),
            ));
        }
        Ok(format!(
            "{}://{}{}",
            url.scheme(),
            url.authority(),
            decode_path(url.path())?
        ))
    } else {
        decode_path(&raw)
    }
}

fn decode_path(path: &str) -> Result<String, SourceContextError> {
    for (index, byte) in path.bytes().enumerate() {
        if byte == b'%'
            && !path
                .as_bytes()
                .get(index + 1..index + 3)
                .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
        {
            return Err(SourceContextError(
                "Source path contains invalid percent encoding".into(),
            ));
        }
    }
    path.split('/')
        .map(|segment| {
            let decoded = percent_decode_str(segment).decode_utf8().map_err(|_| {
                SourceContextError("Source path contains invalid UTF-8 encoding".into())
            })?;
            if matches!(decoded.as_ref(), "." | "..") || decoded.contains(['/', '\\', '\0']) {
                return Err(SourceContextError(
                    "Source path contains a dot segment or encoded separator".into(),
                ));
            }
            Ok(decoded.into_owned())
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("/"))
}
