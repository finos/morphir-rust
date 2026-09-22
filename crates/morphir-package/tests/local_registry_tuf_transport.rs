use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, stream};
use morphir_package::local_registry::{
    TufRole,
    tuf::{AcquisitionBudget, AdmissionError, EvidenceRecorder, ProfileTransport, profile_limits},
};
use package_tough::{IntoVec, Transport, TransportError, TransportErrorKind, TransportStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use url::Url;

#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod mothers;

#[derive(Debug, Clone)]
enum Reply {
    Chunks(Vec<Result<Bytes, TransportErrorKind>>),
    Immediate(TransportErrorKind),
}

#[derive(Debug, Clone)]
struct Source {
    reply: Reply,
    fetched: Arc<AtomicUsize>,
    polled: Arc<AtomicUsize>,
}

impl Source {
    fn new(reply: Reply) -> Self {
        Self {
            reply,
            fetched: Arc::new(AtomicUsize::new(0)),
            polled: Arc::new(AtomicUsize::new(0)),
        }
    }
    fn bytes(bytes: Vec<u8>) -> Self {
        Self::new(Reply::Chunks(vec![Ok(Bytes::from(bytes))]))
    }
}

#[async_trait]
impl Transport for Source {
    async fn fetch(&self, url: Url) -> Result<TransportStream, TransportError> {
        self.fetched.fetch_add(1, Ordering::SeqCst);
        match &self.reply {
            Reply::Immediate(kind) => Err(TransportError::new(*kind, url)),
            Reply::Chunks(chunks) => {
                let polled = self.polled.clone();
                Ok(Box::pin(stream::iter(chunks.clone()).map(move |item| {
                    polled.fetch_add(1, Ordering::SeqCst);
                    item.map_err(|kind| TransportError::new(kind, &url))
                })))
            }
        }
    }
}

#[derive(Debug, Default)]
struct Recorder {
    calls: Mutex<Vec<(String, Vec<u8>)>>,
    deny: bool,
    allowance: Option<AcquisitionBudget>,
    budget_calls: Mutex<Vec<String>>,
    deny_budget: bool,
}

#[async_trait]
impl EvidenceRecorder for Recorder {
    async fn budget(&self, role: TufRole) -> Result<AcquisitionBudget, AdmissionError> {
        self.budget_calls.lock().unwrap().push(format!("{role:?}"));
        if self.deny_budget {
            return Err(AdmissionError::Context);
        }
        self.allowance
            .clone()
            .map_or_else(|| AcquisitionBudget::new(268_435_456, vec![]), Ok)
    }
    async fn record(&self, role: TufRole, bytes: &[u8]) -> Result<(), AdmissionError> {
        self.calls
            .lock()
            .unwrap()
            .push((format!("{role:?}"), bytes.to_vec()));
        if self.deny {
            Err(AdmissionError::Context)
        } else {
            Ok(())
        }
    }
}

fn base() -> Url {
    Url::parse("file:///registry/metadata/").unwrap()
}

fn wrapper(source: Source, recorder: Arc<Recorder>) -> ProfileTransport {
    ProfileTransport::new(Box::new(source), recorder, base()).unwrap()
}

async fn stream_error(transport: &ProfileTransport, name: &str) -> TransportError {
    let stream = transport
        .fetch(base().join(name).unwrap())
        .await
        .expect("security failures must reach Tough through the stream");
    match stream.into_vec().await {
        Err(error) => error,
        Ok(_) => panic!("invalid evidence must release no bytes"),
    }
}

#[tokio::test]
async fn records_exact_validated_evidence_before_exposing_any_bytes() {
    let bytes = mothers::root(2, 1);
    let source = Source::new(Reply::Chunks(
        bytes
            .chunks(7)
            .map(|b| Ok(Bytes::copy_from_slice(b)))
            .collect(),
    ));
    let recorder = Arc::new(Recorder::default());
    let transport = wrapper(source, recorder.clone());
    let stream = transport
        .fetch(base().join("2.root.json").unwrap())
        .await
        .unwrap();
    assert_eq!(
        *recorder.calls.lock().unwrap(),
        [("Root".into(), bytes.clone())]
    );
    assert_eq!(stream.into_vec().await.unwrap(), bytes);
}

#[tokio::test]
async fn wrong_profile_and_requested_version_fail_without_recording() {
    let mut value: serde_json::Value = serde_json::from_slice(&mothers::root(2, 1)).unwrap();
    value["signed"]["spec_version"] = serde_json::json!("1.0.35");
    for bytes in [serde_json::to_vec(&value).unwrap(), mothers::root(3, 1)] {
        let recorder = Arc::new(Recorder::default());
        let transport = wrapper(Source::bytes(bytes), recorder.clone());
        assert_eq!(
            stream_error(&transport, "2.root.json").await.kind(),
            TransportErrorKind::Other
        );
        assert!(recorder.calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn rejects_excess_stream_before_collecting_later_chunks_or_recording() {
    let mut boundary = mothers::root(2, 1);
    boundary.resize(1_048_576, b' ');
    let source = Source::new(Reply::Chunks(vec![
        Ok(boundary.into()),
        Ok(Bytes::from_static(b" ")),
        Ok(Bytes::from_static(b"never read")),
    ]));
    let recorder = Arc::new(Recorder::default());
    let transport = wrapper(source.clone(), recorder.clone());
    assert_eq!(
        stream_error(&transport, "2.root.json").await.kind(),
        TransportErrorKind::Other
    );
    assert_eq!(source.polled.load(Ordering::SeqCst), 2);
    assert!(recorder.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn accepts_inclusive_document_boundaries_and_large_exact_version_names() {
    for (name, mut bytes, maximum) in [
        ("2.root.json", mothers::root(2, 1), 1_048_576),
        ("1.targets.json", mothers::targets(1, 1), 16_777_216),
    ] {
        bytes.resize(maximum, b' ');
        let transport = wrapper(Source::bytes(bytes.clone()), Arc::new(Recorder::default()));
        assert_eq!(
            transport
                .fetch(base().join(name).unwrap())
                .await
                .unwrap()
                .into_vec()
                .await
                .unwrap(),
            bytes
        );
    }
    let version = "184467440737095516160001";
    let mut body = mothers::targets_body(1);
    body["version"] = serde_json::from_str(version).unwrap();
    let bytes = serde_json::to_vec(&serde_json::json!({"signed": body, "signatures": []})).unwrap();
    let transport = wrapper(Source::bytes(bytes.clone()), Arc::new(Recorder::default()));
    assert_eq!(
        transport
            .fetch(base().join(&format!("{version}.targets.json")).unwrap())
            .await
            .unwrap()
            .into_vec()
            .await
            .unwrap(),
        bytes
    );
}

#[tokio::test]
async fn preserves_immediate_absence_but_converts_other_root_errors_to_stream_errors() {
    for kind in [
        TransportErrorKind::Other,
        TransportErrorKind::UnsupportedUrlScheme,
    ] {
        let transport = wrapper(
            Source::new(Reply::Immediate(kind)),
            Arc::new(Recorder::default()),
        );
        assert_eq!(
            stream_error(&transport, "2.root.json").await.kind(),
            TransportErrorKind::Other
        );
    }
    let transport = wrapper(
        Source::new(Reply::Immediate(TransportErrorKind::FileNotFound)),
        Arc::new(Recorder::default()),
    );
    match transport.fetch(base().join("2.root.json").unwrap()).await {
        Err(error) => assert_eq!(error.kind(), TransportErrorKind::FileNotFound),
        Ok(_) => panic!("immediate absence should remain an absence error"),
    }
}

#[tokio::test]
async fn acquired_truncation_is_not_treated_as_missing_successor() {
    let source = Source::new(Reply::Chunks(vec![
        Ok(Bytes::from_static(b"{")),
        Err(TransportErrorKind::FileNotFound),
    ]));
    let recorder = Arc::new(Recorder::default());
    let transport = wrapper(source, recorder.clone());
    assert_eq!(
        stream_error(&transport, "2.root.json").await.kind(),
        TransportErrorKind::Other
    );
    assert!(recorder.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn recorder_denial_prevents_any_bytes_from_reaching_tough() {
    let recorder = Arc::new(Recorder {
        deny: true,
        ..Recorder::default()
    });
    let transport = wrapper(Source::bytes(mothers::root(2, 1)), recorder.clone());
    let mut stream = transport
        .fetch(base().join("2.root.json").unwrap())
        .await
        .unwrap();
    assert_eq!(
        stream.next().await.unwrap().unwrap_err().kind(),
        TransportErrorKind::Other
    );
    assert!(stream.next().await.is_none());
    assert_eq!(recorder.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn rejects_non_metadata_and_escaped_urls_before_access() {
    let source = Source::bytes(mothers::root(2, 1));
    let transport = wrapper(source.clone(), Arc::new(Recorder::default()));
    for name in [
        "root.json",
        "0.root.json",
        "02.root.json",
        "2e0.root.json",
        "%32.root.json",
        "2.root.json?x=1",
        "2.root.json#fragment",
        "nested/2.root.json",
        "../2.root.json",
        "https://other.invalid/2.root.json",
        "records/a.json",
        "2.delegated.json",
    ] {
        assert_eq!(
            stream_error(&transport, name).await.kind(),
            TransportErrorKind::Other,
            "{name}"
        );
    }
    assert_eq!(source.fetched.load(Ordering::SeqCst), 0);
}

#[test]
fn rejects_non_directory_bases_and_uses_profile_loader_limits() {
    for base in [
        "file:///registry/metadata",
        "file:///registry/metadata/?x=1",
        "file:///registry/metadata/#x",
        "mailto:someone@example.com",
    ] {
        assert!(
            ProfileTransport::new(
                Box::new(Source::bytes(vec![])),
                Arc::new(Recorder::default()),
                Url::parse(base).unwrap()
            )
            .is_err()
        );
    }
    let limits = profile_limits();
    assert_eq!(limits.max_root_size, 1_048_576);
    assert_eq!(limits.max_timestamp_size, 1_048_576);
    assert_eq!(limits.max_snapshot_size, 1_048_576);
    assert_eq!(limits.max_targets_size, 16_777_216);
    assert_eq!(limits.max_root_updates, 33);
}

#[tokio::test]
async fn admits_timestamp_and_snapshot_with_the_expected_recorded_roles() {
    let targets = mothers::targets(1, 1);
    let snapshot = mothers::snapshot(&targets, 1);
    let timestamp = mothers::sign(
        serde_json::json!({
            "_type":"timestamp", "spec_version":"1.0.36", "version":1,
            "expires":"2100-01-01T00:00:00Z", "meta":{"snapshot.json":mothers::meta(&snapshot,1)}
        }),
        &[(1, mothers::key(1))],
    );
    for (name, bytes, role) in [
        ("timestamp.json", timestamp, "Timestamp"),
        ("1.snapshot.json", snapshot, "Snapshot"),
    ] {
        let recorder = Arc::new(Recorder::default());
        let transport = wrapper(Source::bytes(bytes.clone()), recorder.clone());
        let stream = transport.fetch(base().join(name).unwrap()).await.unwrap();
        assert_eq!(recorder.calls.lock().unwrap()[0].0, role);
        assert_eq!(stream.into_vec().await.unwrap(), bytes);
    }
}

#[tokio::test]
async fn metadata_component_budget_accepts_128_and_rejects_129_before_access() {
    for length in [128, 129] {
        let version = "1".repeat(length - ".targets.json".len());
        let mut body = mothers::targets_body(1);
        body["version"] = serde_json::from_str(&version).unwrap();
        let bytes =
            serde_json::to_vec(&serde_json::json!({"signed":body, "signatures":[]})).unwrap();
        let source = Source::bytes(bytes);
        let transport = wrapper(source.clone(), Arc::new(Recorder::default()));
        let stream = transport
            .fetch(base().join(&format!("{version}.targets.json")).unwrap())
            .await
            .unwrap();
        assert_eq!(stream.into_vec().await.is_ok(), length == 128);
        assert_eq!(
            source.fetched.load(Ordering::SeqCst),
            usize::from(length == 128)
        );
    }
}

fn limited_recorder(remaining: usize, previous: Vec<Vec<u8>>) -> Arc<Recorder> {
    Arc::new(Recorder {
        allowance: Some(AcquisitionBudget::new(remaining, previous).unwrap()),
        ..Recorder::default()
    })
}

fn assert_aggregate_limit(error: &TransportError) {
    use std::error::Error;
    assert!(
        matches!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<AdmissionError>()),
            Some(AdmissionError::Limit("metadata-bytes"))
        ),
        "{error:?}"
    );
}

#[tokio::test]
async fn aggregate_excess_stops_at_first_provable_chunk_without_reading_remainder() {
    let bytes = mothers::root(2, 1);
    let source = Source::new(Reply::Chunks(vec![
        Ok(Bytes::copy_from_slice(&bytes[..8])),
        Ok(Bytes::copy_from_slice(&bytes[8..9])),
        Ok(Bytes::copy_from_slice(&bytes[9..])),
    ]));
    let recorder = limited_recorder(8, vec![]);
    let transport = wrapper(source.clone(), recorder.clone());
    assert_aggregate_limit(&stream_error(&transport, "2.root.json").await);
    assert_eq!(source.polled.load(Ordering::SeqCst), 2);
    assert!(recorder.calls.lock().unwrap().is_empty());
    assert_eq!(*recorder.budget_calls.lock().unwrap(), ["Root"]);
}

#[tokio::test]
async fn exact_retry_is_allowed_with_no_remaining_logical_bytes() {
    let bytes = mothers::root(2, 1);
    let source = Source::new(Reply::Chunks(
        bytes
            .chunks(13)
            .map(|part| Ok(Bytes::copy_from_slice(part)))
            .collect(),
    ));
    let recorder = limited_recorder(0, vec![mothers::root(1, 1), bytes.clone()]);
    let transport = wrapper(source, recorder.clone());
    let stream = transport
        .fetch(base().join("2.root.json").unwrap())
        .await
        .unwrap();
    assert_eq!(stream.into_vec().await.unwrap(), bytes);
    assert_eq!(recorder.calls.lock().unwrap().len(), 1);
    assert_eq!(*recorder.budget_calls.lock().unwrap(), ["Root"]);
}

#[tokio::test]
async fn changed_retry_stops_when_it_diverges_from_every_counted_input() {
    let bytes = mothers::root(2, 1);
    let source = Source::new(Reply::Chunks(vec![
        Ok(Bytes::copy_from_slice(&bytes[..1])),
        Ok(Bytes::from_static(b"\t")),
        Ok(Bytes::copy_from_slice(&bytes[2..])),
    ]));
    let recorder = limited_recorder(0, vec![bytes]);
    let transport = wrapper(source.clone(), recorder.clone());
    assert_aggregate_limit(&stream_error(&transport, "2.root.json").await);
    assert_eq!(source.polled.load(Ordering::SeqCst), 2);
    assert!(recorder.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn retry_cannot_splice_prefixes_of_different_counted_inputs() {
    let bytes = mothers::root(2, 1);
    let mut other = bytes.clone();
    other[..2].copy_from_slice(b" \t");
    let source = Source::new(Reply::Chunks(vec![
        Ok(Bytes::copy_from_slice(&bytes[..1])),
        Ok(Bytes::from_static(b"\t")),
        Ok(Bytes::copy_from_slice(&bytes[2..])),
    ]));
    let recorder = limited_recorder(0, vec![bytes, other]);
    let transport = wrapper(source.clone(), recorder.clone());
    assert_aggregate_limit(&stream_error(&transport, "2.root.json").await);
    assert_eq!(source.polled.load(Ordering::SeqCst), 2);
    assert!(recorder.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn valid_but_shorter_prefix_is_distinct_evidence_at_end_of_stream() {
    let bytes = mothers::root(2, 1);
    let mut previous = bytes.clone();
    previous.extend_from_slice(b"   ");
    let recorder = limited_recorder(0, vec![previous]);
    let transport = wrapper(Source::bytes(bytes), recorder.clone());
    assert_aggregate_limit(&stream_error(&transport, "2.root.json").await);
    assert!(recorder.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn inclusive_remaining_budget_accepts_distinct_complete_evidence() {
    let bytes = mothers::root(2, 1);
    let recorder = limited_recorder(bytes.len(), vec![]);
    let transport = wrapper(Source::bytes(bytes.clone()), recorder);
    let stream = transport
        .fetch(base().join("2.root.json").unwrap())
        .await
        .unwrap();
    assert_eq!(stream.into_vec().await.unwrap(), bytes);
}

#[tokio::test]
async fn denied_budget_prevents_underlying_access() {
    let source = Source::bytes(mothers::root(2, 1));
    let recorder = Arc::new(Recorder {
        deny_budget: true,
        ..Recorder::default()
    });
    let transport = wrapper(source.clone(), recorder.clone());
    assert_eq!(
        stream_error(&transport, "2.root.json").await.kind(),
        TransportErrorKind::Other
    );
    assert_eq!(source.fetched.load(Ordering::SeqCst), 0);
    assert_eq!(*recorder.budget_calls.lock().unwrap(), ["Root"]);
    assert!(recorder.calls.lock().unwrap().is_empty());
}

#[test]
fn acquisition_budget_cannot_enlarge_profile_bounds() {
    assert!(AcquisitionBudget::new(268_435_456, vec![]).is_ok());
    assert!(AcquisitionBudget::new(268_435_457, vec![]).is_err());
    assert!(AcquisitionBudget::new(0, vec![vec![b' '; 16_777_217]]).is_err());
}
