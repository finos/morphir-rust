use std::io::{Read, Write};
use std::sync::Arc;

use morphir_common::ir_transport::{
    CodecOptions, CodecRegistry, EventSink, EventSource, FormatId, IrCodec, IrVersion, Layout,
    Stage, TransportDiagnostic, VocabularyId,
};
use morphir_core::traversal::{CursorSegment, IrCursor};

struct TestCodec {
    format: FormatId,
}

impl IrCodec for TestCodec {
    fn format(&self) -> &FormatId {
        &self.format
    }

    fn decode(
        &self,
        _reader: &mut dyn Read,
        _options: &CodecOptions,
        _sink: &mut dyn EventSink,
    ) -> Result<(), TransportDiagnostic> {
        Ok(())
    }

    fn encode(
        &self,
        source: &mut dyn EventSource,
        _writer: &mut dyn Write,
        _options: &CodecOptions,
    ) -> Result<(), TransportDiagnostic> {
        while source.next_event()?.is_some() {}
        Ok(())
    }
}

#[test]
fn codec_options_keep_version_layout_format_and_vocabulary_independent() {
    let options = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::yaml())
        .with_vocabulary(VocabularyId::readable());

    assert_eq!(options.version(), IrVersion::V4);
    assert_eq!(options.layout(), Layout::SingleFile);
    assert_eq!(options.format(), &FormatId::yaml());
    assert_eq!(options.vocabulary(), &VocabularyId::readable());
}

#[test]
fn registry_accepts_a_codec_without_changing_builtin_dispatch() {
    let mut registry = CodecRegistry::with_builtins();
    let toml = FormatId::new("toml").unwrap();

    assert!(registry.codec(&FormatId::json()).is_some());
    assert!(registry.codec(&FormatId::yaml()).is_some());
    assert!(registry.codec(&toml).is_none());

    registry.register(Arc::new(TestCodec {
        format: toml.clone(),
    }));

    assert_eq!(registry.codec(&toml).unwrap().format(), &toml);
}

#[test]
fn transport_diagnostics_include_stage_cursor_and_guidance() {
    let cursor = IrCursor::root()
        .child(CursorSegment::Module("orders".into()))
        .child(CursorSegment::Value("total".into()));
    let diagnostic = TransportDiagnostic::error(
        "morphir::ir::normalize::ambiguous_yaml",
        Stage::Normalization,
        cursor.clone(),
        "ambiguous YAML value",
    )
    .with_guidance("use the explicit structural form");

    assert_eq!(diagnostic.stage(), Stage::Normalization);
    assert_eq!(diagnostic.cursor(), &cursor);
    assert_eq!(
        diagnostic.guidance(),
        Some("use the explicit structural form")
    );
}
