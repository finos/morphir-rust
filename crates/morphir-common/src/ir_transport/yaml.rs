//! YAML codec entry point.

use std::io::{Read, Write};

use morphir_core::traversal::IrCursor;

use super::{CodecOptions, EventSink, EventSource, FormatId, IrCodec, Stage, TransportDiagnostic};

/// Built-in native YAML IR codec.
pub struct YamlCodec {
    format: FormatId,
}

impl YamlCodec {
    /// Create the built-in YAML codec.
    pub fn new() -> Self {
        Self {
            format: FormatId::yaml(),
        }
    }

    fn pending() -> TransportDiagnostic {
        TransportDiagnostic::error(
            "morphir::ir::codec::yaml_pending",
            Stage::Encoding,
            IrCursor::root(),
            "the YAML event adapter is not initialized",
        )
    }
}

impl Default for YamlCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl IrCodec for YamlCodec {
    fn format(&self) -> &FormatId {
        &self.format
    }

    fn decode(
        &self,
        _reader: &mut dyn Read,
        _options: &CodecOptions,
        _sink: &mut dyn EventSink,
    ) -> Result<(), TransportDiagnostic> {
        Err(Self::pending())
    }

    fn encode(
        &self,
        _source: &mut dyn EventSource,
        _writer: &mut dyn Write,
        _options: &CodecOptions,
    ) -> Result<(), TransportDiagnostic> {
        Err(Self::pending())
    }
}
