//! Open registry for streaming IR codecs.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::Arc;

use morphir_core::traversal::SemanticEvent;

use super::{CodecOptions, FormatId, TransportDiagnostic};
use crate::ir_transport::{JsonCodec, YamlCodec};

/// Pull-based source of semantic IR events.
pub trait EventSource {
    /// Return the next event, or `None` after the distribution ends.
    fn next_event(&mut self) -> Result<Option<SemanticEvent>, TransportDiagnostic>;
}

/// Receiver for semantic IR events decoded from a physical format.
pub trait EventSink {
    /// Accept one semantic event.
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic>;

    /// Finish the distribution after all events are accepted.
    fn finish(&mut self) -> Result<(), TransportDiagnostic> {
        Ok(())
    }
}

/// Object-safe codec that maps a physical encoding to semantic events and back.
pub trait IrCodec: Send + Sync {
    /// Return the open identifier registered for this codec.
    fn format(&self) -> &FormatId;

    /// Decode one artifact into semantic events.
    fn decode(
        &self,
        reader: &mut dyn Read,
        options: &CodecOptions,
        sink: &mut dyn EventSink,
    ) -> Result<(), TransportDiagnostic>;

    /// Encode semantic events as one artifact.
    fn encode(
        &self,
        source: &mut dyn EventSource,
        writer: &mut dyn Write,
        options: &CodecOptions,
    ) -> Result<(), TransportDiagnostic>;
}

/// Registry that resolves codecs without a closed built-in format enum.
#[derive(Default)]
pub struct CodecRegistry {
    codecs: BTreeMap<FormatId, Arc<dyn IrCodec>>,
}

impl CodecRegistry {
    /// Create an empty codec registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a registry containing the JSON and YAML codec entries.
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        registry.register(Arc::new(JsonCodec::new()));
        registry.register(Arc::new(YamlCodec::new()));
        registry
    }

    /// Register or replace a codec and return the previous entry.
    pub fn register(&mut self, codec: Arc<dyn IrCodec>) -> Option<Arc<dyn IrCodec>> {
        self.codecs.insert(codec.format().clone(), codec)
    }

    /// Resolve a codec by its open format identifier.
    pub fn codec(&self, format: &FormatId) -> Option<&dyn IrCodec> {
        self.codecs.get(format).map(Arc::as_ref)
    }

    /// Iterate over registered format identifiers.
    pub fn formats(&self) -> impl Iterator<Item = &FormatId> {
        self.codecs.keys()
    }
}
