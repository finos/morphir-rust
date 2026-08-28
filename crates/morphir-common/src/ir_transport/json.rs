//! JSON codec entry point.

use std::io::{Read, Write};

use morphir_core::ir::{classic, v4};
use morphir_core::traversal::IrCursor;

use super::semantic::{self, SemanticFile};
use super::{
    CodecOptions, EventSink, EventSource, FormatId, IrCodec, IrVersion, SourceSpan, Stage,
    TransportDiagnostic,
};

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

    fn decode_error(error: serde_json::Error) -> TransportDiagnostic {
        TransportDiagnostic::error(
            "morphir::ir::json::invalid_syntax",
            Stage::Syntax,
            IrCursor::root(),
            error.to_string(),
        )
        .with_source_span(SourceSpan {
            offset: 0,
            length: 0,
            line: error.line(),
            column: error.column(),
        })
        .with_guidance("correct the JSON syntax or select the actual input format")
    }

    fn encode_error(error: impl std::fmt::Display) -> TransportDiagnostic {
        TransportDiagnostic::error(
            "morphir::ir::json::encode_failed",
            Stage::Encoding,
            IrCursor::root(),
            error.to_string(),
        )
        .with_guidance("verify that the semantic event stream contains representable IR nodes")
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
        reader: &mut dyn Read,
        options: &CodecOptions,
        sink: &mut dyn EventSink,
    ) -> Result<(), TransportDiagnostic> {
        match options.version() {
            IrVersion::V3 => {
                let file: classic::Distribution =
                    serde_json::from_reader(reader).map_err(Self::decode_error)?;
                semantic::emit_classic_v3(file, sink)
            }
            IrVersion::V4 => {
                let file: v4::IRFile =
                    serde_json::from_reader(reader).map_err(Self::decode_error)?;
                semantic::emit_v4(file, sink)
            }
        }
    }

    fn encode(
        &self,
        source: &mut dyn EventSource,
        writer: &mut dyn Write,
        options: &CodecOptions,
    ) -> Result<(), TransportDiagnostic> {
        let file = semantic::collect(source, options.version())?;
        match file {
            SemanticFile::ClassicV3(file) => {
                serde_json::to_writer(&mut *writer, &file).map_err(Self::encode_error)?;
            }
            SemanticFile::V4(file) => {
                serde_json::to_writer(&mut *writer, &file).map_err(Self::encode_error)?;
            }
        }
        writer.write_all(b"\n").map_err(Self::encode_error)
    }
}
