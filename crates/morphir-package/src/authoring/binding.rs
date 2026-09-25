//! Publication-time binding of metadata node identities to verified IR bytes.

use super::AuthoredLibrary;
use morphir_core::metadata::{GraphIndex, GraphMapError};
use morphir_core::node_address::{
    ArtifactRevision, ArtifactSelector, NodeCatalog, NodeIndex, NodeResolutionError, NodeUri,
    NodeUriError, Sha256Digest,
};
use std::collections::HashMap;

/// A missing or mismatched node provider prevents publication.
#[derive(Debug, thiserror::Error)]
pub enum BindingError {
    #[error("the containing archive has invalid V4 IR")]
    InvalidArchive,
    #[error("the external provider has invalid V4 IR")]
    InvalidProvider,
    #[error("the same external package was supplied more than once")]
    DuplicateProvider,
    #[error("a publication node URI needs a verified package provider")]
    UnavailableProvider,
    #[error(transparent)]
    Index(#[from] NodeResolutionError),
    #[error(transparent)]
    Uri(#[from] NodeUriError),
}

/// One archive's own index and exact, caller-authenticated external snapshots.
///
/// A digest only checks bytes. The caller must acquire and authenticate each
/// external snapshot through its trust path before adding it here. No network
/// or ambient workspace lookup occurs during binding.
pub struct PublicationBindings {
    catalog: NodeCatalog,
    self_artifact: ArtifactSelector,
    providers: HashMap<ArtifactSelector, Sha256Digest>,
}

impl PublicationBindings {
    /// Start with already-verified bundle bytes. Self references remain
    /// unpinned because their final revision includes the references themselves.
    pub fn new(library: &AuthoredLibrary) -> Result<Self, BindingError> {
        let text =
            std::str::from_utf8(library.ir_bytes()).map_err(|_| BindingError::InvalidArchive)?;
        let (file, _) =
            morphir_core::ir::json::read_ir_file(text).map_err(|_| BindingError::InvalidArchive)?;
        let index = NodeIndex::v4_file(&file).map_err(|_| BindingError::InvalidArchive)?;
        let self_artifact = index
            .addresses()
            .next()
            .map(|uri| uri.artifact().clone())
            .ok_or(BindingError::InvalidArchive)?;
        let mut catalog = NodeCatalog::new();
        catalog.add_current(index);
        Ok(Self {
            catalog,
            self_artifact,
            providers: HashMap::new(),
        })
    }

    /// Register exact acquired V4 bytes against an independently authenticated
    /// digest. The indexed package identity comes from those bytes, not a label.
    pub fn add_v4_provider(
        &mut self,
        bytes: &[u8],
        expected: &Sha256Digest,
    ) -> Result<(), BindingError> {
        let text = std::str::from_utf8(bytes).map_err(|_| BindingError::InvalidProvider)?;
        let (file, _) = morphir_core::ir::json::read_ir_file(text)
            .map_err(|_| BindingError::InvalidProvider)?;
        let index = NodeIndex::v4_file(&file).map_err(|_| BindingError::InvalidProvider)?;
        let artifact = index
            .addresses()
            .next()
            .map(|uri| uri.artifact().clone())
            .ok_or(BindingError::InvalidProvider)?;
        if self.providers.contains_key(&artifact) {
            return Err(BindingError::DuplicateProvider);
        }
        self.catalog.add_v4_json_snapshot(bytes, Some(expected))?;
        self.providers.insert(artifact, expected.clone());
        Ok(())
    }

    /// Check an unpinned guard against the actual provider before adding its
    /// revision. Explicit pins stay explicit, even for the archive's package.
    pub fn bind_uri(&self, uri: &NodeUri) -> Result<NodeUri, BindingError> {
        if !matches!(uri.artifact(), ArtifactSelector::Package(_)) {
            return Err(BindingError::UnavailableProvider);
        }
        match uri.revision() {
            ArtifactRevision::Pinned(_) => {
                self.catalog.resolve(uri)?;
                Ok(uri.clone())
            }
            ArtifactRevision::Current if uri.artifact() == &self.self_artifact => {
                self.catalog.resolve(uri)?;
                Ok(uri.clone())
            }
            ArtifactRevision::Current => {
                let digest = self
                    .providers
                    .get(uri.artifact())
                    .ok_or(BindingError::UnavailableProvider)?;
                let checked = NodeUri::new(
                    uri.artifact().clone(),
                    uri.format(),
                    uri.root().clone(),
                    uri.steps().to_vec(),
                    ArtifactRevision::Pinned(digest.clone()),
                    uri.guard().cloned(),
                )?;
                self.catalog.resolve(&checked)?;
                Ok(NodeUri::new(
                    uri.artifact().clone(),
                    uri.format(),
                    uri.root().clone(),
                    uri.steps().to_vec(),
                    ArtifactRevision::Pinned(digest.clone()),
                    None,
                )?)
            }
        }
    }

    /// Rebind facts, node-local carriers, and detailed source selectors as one
    /// graph update. JSON data values are retained byte-for-byte as values.
    pub fn bind_graph(
        &self,
        graph: &GraphIndex,
    ) -> Result<GraphIndex, GraphMapError<BindingError>> {
        graph.try_map_node_uris(|uri| self.bind_uri(uri))
    }
}
