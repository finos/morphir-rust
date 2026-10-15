use super::{
    MAX_CONTEXTS, MAX_LIBRARIES, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, MAX_TOTAL_CONTEXTS,
    SERVICE_ID, ServiceError, decode::Correlation,
};
use crate::{
    local_registry::mvp::{self, RestoredSnapshot},
    provider_snapshot::{CORE_PROJECTION_CODEC, LibraryProjection},
    resolution::ReleaseId,
};
use ion_rs::{Blob, Element, Sequence, Struct, StructBuilder, v1_0::Binary};

fn base(correlation: Correlation) -> StructBuilder {
    Struct::builder()
        .with_field("invocation", correlation.invocation)
        .with_field("projectScope", correlation.scope)
        .with_field("service", SERVICE_ID)
        .with_field("profile", mvp::PROFILE)
        .with_field("profileVersion", mvp::PROFILE_VERSION)
}
fn encode(builder: StructBuilder, annotation: &str) -> Result<Vec<u8>, ServiceError> {
    let bytes = Element::from(builder.build())
        .with_annotations([annotation])
        .encode_as(Binary)
        .map_err(|_| ServiceError::ResponseLimit)?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(ServiceError::ResponseLimit);
    }
    Ok(bytes)
}
pub(super) fn capabilities(correlation: Correlation) -> Result<Vec<u8>, ServiceError> {
    encode(
        base(correlation)
            .with_field("codec", CORE_PROJECTION_CODEC)
            .with_field("maxRequestBytes", MAX_REQUEST_BYTES as i64)
            .with_field("maxResponseBytes", MAX_RESPONSE_BYTES as i64)
            .with_field("maxLibraries", MAX_LIBRARIES as i64)
            .with_field("maxContexts", MAX_CONTEXTS as i64)
            .with_field("maxTotalContexts", MAX_TOTAL_CONTEXTS as i64),
        "morphir_provider_capabilities_response_v1",
    )
}
fn release(release: &ReleaseId) -> String {
    format!(
        "{}@{}",
        release.package_path().as_str(),
        release.version().as_str()
    )
}
pub(super) fn snapshot(
    correlation: Correlation,
    snapshot: &RestoredSnapshot,
    projection: &LibraryProjection,
) -> Result<Vec<u8>, ServiceError> {
    if snapshot.libraries().len() > MAX_LIBRARIES {
        return Err(ServiceError::ResponseLimit);
    }
    if snapshot
        .libraries()
        .iter()
        .map(|library| library.context_resources().len())
        .sum::<usize>()
        > MAX_TOTAL_CONTEXTS
    {
        return Err(ServiceError::ResponseLimit);
    }
    let mut size =
        projection.core_json().len() + projection.document_metadata_json().map_or(0, <[u8]>::len);
    for library in snapshot.libraries() {
        if library.context_resources().len() > MAX_CONTEXTS {
            return Err(ServiceError::ResponseLimit);
        }
        size = size
            .checked_add(library.manifest().len())
            .and_then(|n| n.checked_add(library.payload().bytes().len()))
            .ok_or(ServiceError::ResponseLimit)?;
        for context in library.context_resources() {
            size = size
                .checked_add(context.bytes().len())
                .ok_or(ServiceError::ResponseLimit)?;
        }
        if size > MAX_RESPONSE_BYTES {
            return Err(ServiceError::ResponseLimit);
        }
    }
    let mut libraries = Sequence::builder();
    for library in snapshot.libraries() {
        let mut contexts = Sequence::builder();
        for context in library.context_resources() {
            contexts = contexts.push(
                Struct::builder()
                    .with_field("path", context.path().as_str())
                    .with_field("sha256", context.digest().to_string())
                    .with_field("bytes", Blob::from(context.bytes()))
                    .build(),
            );
        }
        libraries = libraries.push(
            Struct::builder()
                .with_field("release", release(library.release()))
                .with_field("manifest", Blob::from(library.manifest()))
                .with_field("nativePath", library.payload().path().as_str())
                .with_field("revision", library.payload().digest().to_string())
                .with_field("native", Blob::from(library.payload().bytes()))
                .with_field("contexts", ion_rs::List::from(contexts.build()))
                .build(),
        );
    }
    let metadata = projection
        .document_metadata_json()
        .map(|bytes| Element::from(Blob::from(bytes)))
        .unwrap_or_else(|| Element::from(ion_rs::Value::Null(ion_rs::IonType::Blob)));
    encode(
        base(correlation)
            .with_field("release", release(snapshot.root()))
            .with_field("policySha256", snapshot.policy_digest().to_string())
            .with_field("lockSha256", snapshot.lock_digest().to_string())
            .with_field("codec", CORE_PROJECTION_CODEC)
            .with_field("core", Blob::from(projection.core_json()))
            .with_field("metadata", metadata)
            .with_field("libraries", ion_rs::List::from(libraries.build())),
        "morphir_provider_snapshot_response_v1",
    )
}
