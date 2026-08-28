//! JSON codec entry point.

use std::io::{Read, Write};

use morphir_core::traversal::IrCursor;

use super::{CodecOptions, EventSink, EventSource, FormatId, IrCodec, Stage, TransportDiagnostic};

/// Built-in JSON IR codec.
pub struct JsonCodec {
    format: FormatId,
}

impl JsonCodec {
    /// Create the built-in JSON codec.
    pub fn new() -> Self {
        Self {
            format: FormatId::json(),
        }
    }

    fn pending() -> TransportDiagnostic {
        TransportDiagnostic::error(
            "morphir::ir::codec::json_pending",
            Stage::Encoding,
            IrCursor::root(),
            "the JSON event adapter is not initialized",
        )
    }
}

impl Default for JsonCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl IrCodec for JsonCodec {
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
