use std::io::Cursor;

use std::collections::VecDeque;

use morphir_common::ir_transport::{
    ClassicToV4, CodecOptions, EventSink, EventSource, FormatId, IrCodec, IrVersion, JsonCodec,
    Layout, Pipeline, TransportDiagnostic, YamlCodec,
};
use morphir_core::ir::classic;
use morphir_core::migration::{MigrationOptions, migrate_distribution};
use morphir_core::traversal::{ModuleEvent, SemanticEvent, SemanticEventKind};

const SPECS: &str = r#"{"formatVersion":"3.1.0","distribution":["Specs",[["my"],["pkg"]],[[[["morphir"],["s","d","k"]],{"modules":[]}]],{"modules":[[[["basics"]],{"types":[[["int"],{"doc":"","value":["OpaqueTypeSpecification",[]]}]],"values":[],"doc":"Basics."}]]}]}"#;

const LIBRARY_3_1_0: &str = r#"{"formatVersion":"3.1.0","distribution":["Library",[["my"],["pkg"]],[],{"modules":[[[["basics"]],{"access":"Public","value":{"types":[],"values":[],"doc":"Basics."}}]]}]}"#;

#[derive(Default)]
struct Collect(Vec<SemanticEvent>);

impl EventSink for Collect {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        self.0.push(event);
        Ok(())
    }
}

fn options(format: FormatId) -> CodecOptions {
    CodecOptions::new(IrVersion::V3, Layout::SingleFile, format)
}

fn events(codec: &dyn IrCodec, text: &str, format: FormatId) -> Vec<SemanticEvent> {
    let mut sink = Collect::default();
    codec
        .decode(
            &mut Cursor::new(text.as_bytes()),
            &options(format),
            &mut sink,
        )
        .unwrap();
    sink.0
}

fn text(codec: &dyn IrCodec, events: Vec<SemanticEvent>, format: FormatId) -> String {
    let mut out = Vec::new();
    let mut sink = codec.encoder(&mut out, &options(format)).unwrap();
    for event in events {
        sink.accept(event).unwrap();
    }
    sink.finish().unwrap();
    drop(sink);
    String::from_utf8(out).unwrap()
}

/// Feeds `events` to a fresh encoder and returns the first diagnostic, from `accept` or `finish`.
fn encode_failure(
    codec: &dyn IrCodec,
    events: Vec<SemanticEvent>,
    format: FormatId,
) -> TransportDiagnostic {
    let mut out = Vec::new();
    let mut sink = codec.encoder(&mut out, &options(format)).unwrap();
    for event in events {
        if let Err(diagnostic) = sink.accept(event) {
            return diagnostic;
        }
    }
    sink.finish()
        .expect_err("the encoder accepted a module of the wrong kind")
}

fn is_module(event: &SemanticEvent) -> bool {
    matches!(event.kind(), SemanticEventKind::Module(_))
}

/// Replaces the module events of `header_from` with the module events of `modules_from`.
fn splice(header_from: &[SemanticEvent], modules_from: &[SemanticEvent]) -> Vec<SemanticEvent> {
    let mut spliced: Vec<SemanticEvent> = header_from
        .iter()
        .filter(|event| !is_module(event) && !matches!(event.kind(), SemanticEventKind::End))
        .cloned()
        .collect();
    spliced.extend(
        modules_from
            .iter()
            .filter(|event| is_module(event))
            .cloned(),
    );
    spliced.push(header_from.last().unwrap().clone());
    spliced
}

#[test]
fn a_v3_specs_distribution_round_trips_through_json_and_yaml() {
    let original = events(&JsonCodec::new(), SPECS, FormatId::json());
    assert!(
        original.iter().any(|event| matches!(
            event.kind(),
            SemanticEventKind::Module(ModuleEvent::ClassicV3Specification { .. })
        )),
        "{original:?}"
    );
    for (codec, format) in [
        (&JsonCodec::new() as &dyn IrCodec, FormatId::json()),
        (&YamlCodec::new() as &dyn IrCodec, FormatId::yaml()),
    ] {
        let written = text(codec, original.clone(), format.clone());
        assert_eq!(
            events(codec, &written, format.clone()),
            original,
            "{format}: {written}"
        );
    }
    let json = text(&JsonCodec::new(), original, FormatId::json());
    assert!(
        json.contains(r#""formatVersion":"3.1.0""#) && json.contains(r#""Specs""#),
        "{json}"
    );
}

#[test]
fn a_v3_library_read_as_3_1_0_is_written_back_as_3() {
    let read = events(&JsonCodec::new(), LIBRARY_3_1_0, FormatId::json());
    let json = text(&JsonCodec::new(), read, FormatId::json());
    assert!(
        json.starts_with(r#"{"formatVersion":3,"distribution":["Library","#),
        "{json}"
    );
    assert!(!json.contains("3.1.0"), "{json}");
}

#[test]
fn a_definition_module_under_a_specs_header_is_a_module_kind_mismatch() {
    let specs = events(&JsonCodec::new(), SPECS, FormatId::json());
    let library = events(&JsonCodec::new(), LIBRARY_3_1_0, FormatId::json());
    for (codec, format, code) in [
        (
            &JsonCodec::new() as &dyn IrCodec,
            FormatId::json(),
            "morphir::ir::json::module_kind_mismatch",
        ),
        (
            &YamlCodec::new() as &dyn IrCodec,
            FormatId::yaml(),
            "morphir::ir::codec::module_kind_mismatch",
        ),
    ] {
        let diagnostic = encode_failure(codec, splice(&specs, &library), format.clone());
        assert_eq!(diagnostic.code(), code, "{format}: {diagnostic}");
    }
}

#[test]
fn a_specification_module_under_a_library_header_is_a_module_kind_mismatch() {
    let specs = events(&JsonCodec::new(), SPECS, FormatId::json());
    let library = events(&JsonCodec::new(), LIBRARY_3_1_0, FormatId::json());
    for (codec, format, code) in [
        (
            &JsonCodec::new() as &dyn IrCodec,
            FormatId::json(),
            "morphir::ir::json::module_kind_mismatch",
        ),
        (
            &YamlCodec::new() as &dyn IrCodec,
            FormatId::yaml(),
            "morphir::ir::codec::module_kind_mismatch",
        ),
    ] {
        let diagnostic = encode_failure(codec, splice(&library, &specs), format.clone());
        assert_eq!(diagnostic.code(), code, "{format}: {diagnostic}");
    }
}

struct Queue(VecDeque<SemanticEvent>);

impl EventSource for Queue {
    fn next_event(&mut self) -> Result<Option<SemanticEvent>, TransportDiagnostic> {
        Ok(self.0.pop_front())
    }
}

#[test]
fn streaming_migration_of_a_v3_specs_distribution_matches_the_typed_migration() {
    let original: classic::Distribution = serde_json::from_str(SPECS).unwrap();
    let typed = migrate_distribution(&original, MigrationOptions::default()).unwrap();
    let expected = serde_json::to_value(&typed.value).unwrap();
    assert_eq!(expected["formatVersion"], "4.0.0", "{expected}");

    let transform = ClassicToV4::new(MigrationOptions::default());
    let report = transform.report_handle();
    let mut pipeline = Pipeline::new().with_transform(transform);
    let mut source = Queue(events(&JsonCodec::new(), SPECS, FormatId::json()).into());
    let mut out = Vec::new();
    {
        let v4_options = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::json());
        let mut encoder = JsonCodec::new().encoder(&mut out, &v4_options).unwrap();
        pipeline.run(&mut source, encoder.as_mut()).unwrap();
    }
    let streamed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(streamed, expected);
    assert!(report.get().unwrap().can_publish());
}
