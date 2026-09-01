//! Parsing for local JSONL extension histories.

use crate::domain::supports_release_schema_version;
use crate::{
    CURRENT_RELEASE_SCHEMA_VERSION, DistributionError, ExtensionId, MINIMUM_RELEASE_SCHEMA_VERSION,
    ReleaseRecord, Result, SchemaVersion, Sha256Digest,
};
use serde::Deserialize;
use std::cmp::Ordering;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseSchemaEnvelope {
    schema_version: SchemaVersion,
}

/// A validated release history and the digest of its exact JSONL bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionHistory {
    extension_id: ExtensionId,
    releases: Vec<ReleaseRecord>,
    revision: Sha256Digest,
}

impl ExtensionHistory {
    /// Parse schema-versioned JSONL records and reject ambiguous versions.
    pub fn parse_jsonl(bytes: &[u8]) -> Result<Self> {
        let mut releases: Vec<ReleaseRecord> = Vec::new();
        for (line_index, line) in bytes.split(|byte| *byte == b'\n').enumerate() {
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let envelope: ReleaseSchemaEnvelope =
                serde_json::from_slice(line).map_err(|source| {
                    DistributionError::InvalidRecord {
                        line: line_index + 1,
                        source,
                    }
                })?;
            if !supports_release_schema_version(envelope.schema_version) {
                return Err(DistributionError::UnsupportedSchema {
                    line: line_index + 1,
                    version: envelope.schema_version,
                    minimum: MINIMUM_RELEASE_SCHEMA_VERSION,
                    maximum: CURRENT_RELEASE_SCHEMA_VERSION,
                });
            }
            let record: ReleaseRecord = serde_json::from_slice(line).map_err(|source| {
                DistributionError::InvalidRecord {
                    line: line_index + 1,
                    source,
                }
            })?;
            if let Some(first) = releases.first()
                && record.extension_id() != first.extension_id()
            {
                return Err(DistributionError::MixedIdentity {
                    expected: first.extension_id().to_string(),
                    actual: record.extension_id().to_string(),
                    line: line_index + 1,
                });
            }
            releases.push(record);
        }

        let extension_id = releases
            .first()
            .map(|record| record.extension_id().clone())
            .ok_or(DistributionError::EmptyHistory)?;
        let mut versions: Vec<semver::Version> = Vec::new();
        for record in &releases {
            if versions.contains(record.version()) {
                return Err(DistributionError::DuplicateVersion {
                    version: record.version().clone(),
                });
            }
            if let Some(first) = versions
                .iter()
                .find(|version| version.cmp_precedence(record.version()) == Ordering::Equal)
            {
                return Err(DistributionError::DuplicatePrecedence {
                    first: (*first).clone(),
                    second: record.version().clone(),
                });
            }
            versions.push(record.version().clone());
        }

        Ok(Self {
            extension_id,
            releases,
            revision: Sha256Digest::of_bytes(bytes),
        })
    }

    /// Return the extension identity shared by all records.
    pub fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }

    /// Return the validated releases in source order.
    pub fn releases(&self) -> &[ReleaseRecord] {
        &self.releases
    }

    /// Return the SHA-256 digest of the exact JSONL history bytes.
    pub fn revision(&self) -> &Sha256Digest {
        &self.revision
    }
}
