use crate::{digest::Digest, local_registry::mvp::SnapshotLibrary};
use morphir_core::{
    ir::{json::read_ir_file, v4},
    metadata::{ContextResources, inline_document_contexts},
};
use serde::Deserialize;
use std::fmt;

/// The exact supported conversion: native 4.1 document metadata is retained
/// separately, while metadata-free semantic definitions use canonical v4.0 JSON.
/// Node-local linked metadata is explicitly unsupported by this revision.
/// Parameter maps must already be lexical, matching the consumer's map reader.
pub const CORE_PROJECTION_CODEC: &str = "v4.1-document-metadata-to-v4.0/1";

/// A native Library cannot be represented by this consumer projection profile.
#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    /// The projection accepts exactly native 4.1.0.
    #[error("the consumer projection requires native IR 4.1.0")]
    NativeFormat,
    /// Nested linked metadata needs a separately qualified projection.
    #[error("node-local linked metadata is unsupported by this projection")]
    NodeLocalMetadata,
    /// The consumer sorts parameter maps; nonlexical order cannot be retained.
    #[error("nonlexical parameter order needs a separately qualified consumer projection")]
    ParameterOrder,
    /// An authenticated context could not be resolved from the retained inventory.
    #[error("cannot resolve native metadata from retained contexts")]
    InvalidContext,
    /// Native parsing, writing or the semantic roundtrip failed.
    #[error("native semantics cannot be retained by this consumer projection")]
    UnsupportedSemantics,
}

/// A derived core view and original document metadata bound to the native digest.
/// This does not change the native bytes or attest to another native IR codec.
pub struct LibraryProjection {
    native_digest: Digest,
    core_json: Vec<u8>,
    document_metadata_json: Option<Vec<u8>>,
}

impl LibraryProjection {
    /// Identity of the original authenticated payload, never of the core view.
    pub fn native_digest(&self) -> &Digest {
        &self.native_digest
    }
    /// Canonical v4.0 semantic JSON, with the explicit codec contract above.
    pub fn core_json(&self) -> &[u8] {
        &self.core_json
    }
    /// Original `$meta` JSON body, preserving its authored lexical bytes.
    /// Relative context locations still refer to the retained Library inventory.
    pub fn document_metadata_json(&self) -> Option<&[u8]> {
        self.document_metadata_json.as_deref()
    }
}

impl fmt::Debug for LibraryProjection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LibraryProjection")
            .field("native_digest", &self.native_digest)
            .field("core_length", &self.core_json.len())
            .field(
                "metadata_length",
                &self.document_metadata_json.as_ref().map(Vec::len),
            )
            .finish()
    }
}

#[derive(Deserialize)]
struct NativeDocument<'a> {
    #[serde(rename = "$meta", borrow)]
    metadata: Option<&'a serde_json::value::RawValue>,
}

/// Derive a separately identified semantic view from the retained native payload.
///
/// Original bytes and metadata remain available through the snapshot. This
/// profile supports document-owned 4.1 metadata only; it refuses nested linked
/// carriers rather than dropping them. It reparses the projected v4.0 view and
/// checks complete typed distribution equality before returning any projection.
/// Nonlexical parameter maps are refused because the consumer sorts such maps.
/// There is no filesystem or ambient context acquisition.
///
/// ```no_run
/// # use morphir_package::local_registry::mvp::RestoredSnapshot;
/// use morphir_package::provider_snapshot::project_library;
/// # fn inspect(snapshot: &RestoredSnapshot) -> Result<(), Box<dyn std::error::Error>> {
/// let projected = project_library(&snapshot.libraries()[0])?;
/// assert_eq!(projected.native_digest(), snapshot.libraries()[0].payload().digest());
/// # Ok(())
/// # }
/// ```
pub fn project_library(library: &SnapshotLibrary) -> Result<LibraryProjection, ProjectionError> {
    let bytes = library.payload().bytes();
    let authored: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| ProjectionError::UnsupportedSemantics)?;
    let raw: NativeDocument<'_> =
        serde_json::from_slice(bytes).map_err(|_| ProjectionError::UnsupportedSemantics)?;
    let mut resources = ContextResources::new(".");
    for context in library.context_resources() {
        resources.insert_local(context.path().as_str(), context.bytes().to_vec());
    }
    let inline = inline_document_contexts(
        &authored,
        &resources,
        Some(library.payload().path().as_str()),
    )
    .map_err(|_| ProjectionError::InvalidContext)?;
    let (native, warnings) =
        read_ir_file(&inline.to_string()).map_err(|_| ProjectionError::UnsupportedSemantics)?;
    if native.format_version != v4::FormatVersion::String("4.1.0".into()) {
        return Err(ProjectionError::NativeFormat);
    }
    if !warnings.is_empty() {
        return Err(ProjectionError::UnsupportedSemantics);
    }
    // Construct a distinct semantic artifact; never relabel the native payload.
    let core = v4::IRFile {
        format_version: v4::FormatVersion::Integer(4),
        distribution: native.distribution,
        metadata: None,
    };
    if core.has_linked_metadata() {
        return Err(ProjectionError::NodeLocalMetadata);
    }
    super::compatibility::check_parameter_order(&core)?;
    let core_json = v4::with_type_encoding(v4::TypeEncoding::Compact, || serde_json::to_vec(&core))
        .map_err(|_| ProjectionError::UnsupportedSemantics)?;
    let text =
        std::str::from_utf8(&core_json).map_err(|_| ProjectionError::UnsupportedSemantics)?;
    let (roundtrip, warnings) =
        read_ir_file(text).map_err(|_| ProjectionError::UnsupportedSemantics)?;
    if !warnings.is_empty() || roundtrip.distribution != core.distribution {
        return Err(ProjectionError::UnsupportedSemantics);
    }
    Ok(LibraryProjection {
        native_digest: library.payload().digest().clone(),
        core_json,
        document_metadata_json: raw.metadata.map(|value| value.get().as_bytes().to_vec()),
    })
}
