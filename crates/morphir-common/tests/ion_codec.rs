use std::io::Cursor;

use morphir_common::ir_transport::{
    CodecOptions, CodecRegistry, EventSink, FormatId, IonCodec, IrCodec, IrVersion, JsonCodec,
    Layout, TransportDiagnostic,
};
use morphir_core::traversal::SemanticEvent;

const V3_JSON: &str = r#"{
  "formatVersion": 3,
  "distribution": ["Library", [["example"]], [], {"modules": []}]
}"#;

const V3_EMPTY_ION: &str = r#"
morphir::{
  ionVersion: "0.1.0-draft.1",
  formatVersion: "3.0.0",
  kind: library,
  packageName: "example",
}
morphir_footer::{}
"#;

#[derive(Default)]
struct CollectingSink(Vec<SemanticEvent>);

impl EventSink for CollectingSink {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        self.0.push(event);
        Ok(())
    }
}

fn decode(
    codec: &dyn IrCodec,
    input: &str,
    options: &CodecOptions,
) -> Result<Vec<SemanticEvent>, TransportDiagnostic> {
    let mut reader = Cursor::new(input.as_bytes());
    let mut sink = CollectingSink::default();
    codec.decode(&mut reader, options, &mut sink)?;
    Ok(sink.0)
}

#[test]
fn an_empty_v3_library_datagram_matches_the_json_ir() {
    let ion = decode(
        &IonCodec::new(),
        V3_EMPTY_ION,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::ion()),
    )
    .unwrap();
    let json = decode(
        &JsonCodec::new(),
        V3_JSON,
        &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json()),
    )
    .unwrap();

    assert_eq!(ion, json);
}

#[test]
fn the_builtin_registry_resolves_ion() {
    let registry = CodecRegistry::with_builtins();

    assert_eq!(
        registry.codec(&FormatId::ion()).unwrap().format(),
        &FormatId::ion()
    );
}
