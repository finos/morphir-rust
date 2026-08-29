//! Integration tests for format-version-aware transport decoding.

use morphir_common::ir_transport::{
    CodecOptions, EventSink, FormatId, IrCodec, IrVersion, JsonCodec, Layout, YamlCodec,
};

struct NullSink;

impl EventSink for NullSink {
    fn accept(
        &mut self,
        _event: morphir_core::traversal::SemanticEvent,
    ) -> Result<(), morphir_common::ir_transport::TransportDiagnostic> {
        Ok(())
    }

    fn finish(&mut self) -> Result<(), morphir_common::ir_transport::TransportDiagnostic> {
        Ok(())
    }
}

#[test]
fn json_v3_codec_accepts_supported_release_string_spelling() {
    let source = br#"{"formatVersion":"3.0.0","distribution":["Library",[["example"]],[],{"modules":[]}]}"#;
    let mut reader = &source[..];
    let mut sink = NullSink;
    JsonCodec::new()
        .decode(
            &mut reader,
            &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json()),
            &mut sink,
        )
        .expect("v3 JSON decode");
}

#[test]
fn yaml_v3_codec_accepts_supported_release_string_spelling() {
    let source = b"formatVersion: \"3.0.0\"\ndistribution:\n  - Library\n  - - - example\n  - []\n  - modules: []\n";
    let mut reader = &source[..];
    let mut sink = NullSink;
    YamlCodec::new()
        .decode(
            &mut reader,
            &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::yaml()),
            &mut sink,
        )
        .expect("v3 YAML decode");
}
