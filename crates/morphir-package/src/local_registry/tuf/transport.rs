use super::{AcquisitionBudget, AdmissionError, decode_profile};
use crate::local_registry::TufRole;
use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, stream};
use package_tough::{Limits, Transport, TransportError, TransportErrorKind, TransportStream};
use std::{fmt::Debug, sync::Arc};
use url::Url;

/// Protected evidence recorder, shared with the operation's admission context.
#[async_trait]
pub trait EvidenceRecorder: Debug + Send + Sync {
    /// Read remaining operation bytes and already-counted same-role inputs before
    /// acquisition. No default is safe: the protected operation owns this budget.
    async fn budget(&self, role: TufRole) -> Result<AcquisitionBudget, AdmissionError>;
    /// Admit exact evidence into the operation budget and durable marker context.
    async fn record(&self, role: TufRole, bytes: &[u8]) -> Result<(), AdmissionError>;
}

/// Bounded metadata ingress for the package TUF loader.
#[derive(Debug, Clone)]
pub struct ProfileTransport {
    inner: Box<dyn Transport>,
    recorder: Arc<dyn EvidenceRecorder>,
    metadata_base: Url,
}

impl ProfileTransport {
    /// Bind a metadata transport to its protected evidence recorder and base URL.
    pub fn new(
        inner: Box<dyn Transport>,
        recorder: Arc<dyn EvidenceRecorder>,
        metadata_base: Url,
    ) -> Result<Self, AdmissionError> {
        if metadata_base.cannot_be_a_base()
            || !metadata_base.path().ends_with('/')
            || metadata_base.query().is_some()
            || metadata_base.fragment().is_some()
        {
            return Err(AdmissionError::Profile(
                "metadata base must be a directory URL",
            ));
        }
        Ok(Self {
            inner,
            recorder,
            metadata_base,
        })
    }
}

#[async_trait]
impl Transport for ProfileTransport {
    async fn fetch(&self, url: Url) -> Result<TransportStream, TransportError> {
        let request = match MetadataRequest::parse(&self.metadata_base, &url) {
            Ok(request) => request,
            Err(error) => return Ok(error_stream(rejected(&url, error))),
        };
        let budget = match self.recorder.budget(request.role).await {
            Ok(budget) => budget,
            Err(error) => return Ok(error_stream(rejected(&url, error))),
        };
        let stream = match self.inner.fetch(url.clone()).await {
            Ok(stream) => stream,
            Err(error) if error.kind() == TransportErrorKind::FileNotFound => return Err(error),
            // Tough treats immediate root fetch errors as absence. All other errors
            // must reach its stream collector so they cannot end rotation silently.
            Err(error) => {
                return Ok(error_stream(TransportError::new_with_cause(
                    TransportErrorKind::Other,
                    &url,
                    error,
                )));
            }
        };
        let bytes = match collect_bounded(stream, request.role, budget, &url).await {
            Ok(bytes) => bytes,
            Err(error) => return Ok(error_stream(error)),
        };
        let metadata = match decode_profile(&bytes, request.role) {
            Ok(metadata) => metadata,
            Err(error) => return Ok(error_stream(rejected(&url, error))),
        };
        drop(bytes);
        if let Some(expected) = request.version
            && metadata.document()["signed"]["version"]
                .as_number()
                .map(serde_json::Number::as_str)
                != Some(expected)
        {
            return Ok(error_stream(rejected(
                &url,
                AdmissionError::Link("requested metadata version"),
            )));
        }
        if let Err(error) = self.recorder.record(request.role, metadata.bytes()).await {
            return Ok(error_stream(rejected(&url, error)));
        }
        Ok(Box::pin(stream::iter([Ok(Bytes::from(metadata.bytes))])))
    }
}

/// Inclusive package metadata limits, with one terminal root absence probe.
///
/// The host admission guard must independently reject a thirty-third accepted root
/// transition. This extra probe lets an exactly thirty-two-transition chain finish.
pub fn profile_limits() -> Limits {
    Limits {
        max_root_size: 1_048_576,
        max_timestamp_size: 1_048_576,
        max_snapshot_size: 1_048_576,
        max_targets_size: 16_777_216,
        max_root_updates: 33,
    }
}

struct MetadataRequest<'a> {
    role: TufRole,
    version: Option<&'a str>,
}

impl<'a> MetadataRequest<'a> {
    fn parse(base: &Url, url: &'a Url) -> Result<Self, AdmissionError> {
        let name = url
            .path()
            .rsplit('/')
            .next()
            .ok_or(AdmissionError::Profile("metadata filename"))?;
        if base.join(name).ok().as_ref() != Some(url) {
            return Err(AdmissionError::Profile(
                "metadata URL outside configured directory",
            ));
        }
        if name.len() > 128 {
            return Err(AdmissionError::Limit("component-bytes"));
        }
        if name == "timestamp.json" {
            return Ok(Self {
                role: TufRole::Timestamp,
                version: None,
            });
        }
        for (suffix, role) in [
            (".root.json", TufRole::Root),
            (".snapshot.json", TufRole::Snapshot),
            (".targets.json", TufRole::Targets),
        ] {
            if let Some(version) = name.strip_suffix(suffix)
                && !version.is_empty()
                && !version.starts_with('0')
                && version.bytes().all(|b| b.is_ascii_digit())
            {
                return Ok(Self {
                    role,
                    version: Some(version),
                });
            }
        }
        Err(AdmissionError::Profile("unsupported metadata filename"))
    }
}

async fn collect_bounded(
    mut stream: TransportStream,
    role: TufRole,
    mut budget: AcquisitionBudget,
    url: &Url,
) -> Result<Vec<u8>, TransportError> {
    let (maximum, resource) = match role {
        TufRole::Root => (1_048_576, "root-bytes"),
        TufRole::Timestamp => (1_048_576, "timestamp-bytes"),
        TufRole::Snapshot => (1_048_576, "snapshot-bytes"),
        TufRole::Targets => (16_777_216, "targets-bytes"),
    };
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        // An error after opening, even FileNotFound, is not evidence of absence.
        let chunk = chunk.map_err(|error| {
            TransportError::new_with_cause(TransportErrorKind::Other, url, error)
        })?;
        if chunk.len() > maximum - bytes.len() {
            return Err(rejected(url, AdmissionError::Limit(resource)));
        }
        budget
            .check_append(&bytes, &chunk)
            .map_err(|error| rejected(url, error))?;
        bytes.try_reserve_exact(chunk.len()).map_err(|error| {
            TransportError::new_with_cause(TransportErrorKind::Other, url, error)
        })?;
        bytes.extend_from_slice(&chunk);
    }
    budget
        .check_complete(&bytes)
        .map_err(|error| rejected(url, error))?;
    Ok(bytes)
}

fn rejected(url: &Url, error: AdmissionError) -> TransportError {
    TransportError::new_with_cause(TransportErrorKind::Other, url, error)
}

fn error_stream(error: TransportError) -> TransportStream {
    Box::pin(stream::iter([Err(error)]))
}
