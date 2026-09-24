use std::io::Cursor;

use std::collections::VecDeque;

use morphir_common::ir_transport::{
    ClassicToV4, CodecOptions, EventSink, EventSource, FormatId, IonCodec, IrCodec, IrVersion,
    JsonCodec, Layout, Pipeline, TransportDiagnostic, YamlCodec,
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

/// Feeds `events` to a fresh encoder and returns the first diagnostic, from `accept` or
/// `finish`, with the bytes the encoder wrote before it.
fn encode_failure(
    codec: &dyn IrCodec,
    events: Vec<SemanticEvent>,
    format: FormatId,
) -> (TransportDiagnostic, String) {
    let mut out = Vec::new();
    let mut sink = codec.encoder(&mut out, &options(format)).unwrap();
    let mut failure = None;
    for event in events {
        if let Err(diagnostic) = sink.accept(event) {
            failure = Some(diagnostic);
            break;
        }
    }
    let diagnostic = match failure {
        Some(diagnostic) => diagnostic,
        None => sink
            .finish()
            .expect_err("the encoder accepted a module of the wrong kind"),
    };
    drop(sink);
    (diagnostic, String::from_utf8(out).unwrap())
}

/// The bytes an encoder writes for `events` without being finished.
fn partial_output(codec: &dyn IrCodec, events: &[SemanticEvent], format: FormatId) -> String {
    let mut out = Vec::new();
    let mut sink = codec.encoder(&mut out, &options(format)).unwrap();
    for event in events {
        sink.accept(event.clone()).unwrap();
    }
    drop(sink);
    String::from_utf8(out).unwrap()
}

/// The events of `events` before its first module or end.
fn before_modules(events: &[SemanticEvent]) -> Vec<SemanticEvent> {
    events
        .iter()
        .take_while(|event| !is_module(event) && !matches!(event.kind(), SemanticEventKind::End))
        .cloned()
        .collect()
}

fn decode_failure(codec: &dyn IrCodec, text: &str, format: FormatId) -> TransportDiagnostic {
    let mut sink = Collect::default();
    codec
        .decode(
            &mut Cursor::new(text.as_bytes()),
            &options(format),
            &mut sink,
        )
        .expect_err("the decoder accepted the document")
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
fn a_v3_specs_distribution_round_trips_through_json_yaml_and_ion() {
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
        (&IonCodec::new() as &dyn IrCodec, FormatId::ion()),
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
        let (diagnostic, written) = encode_failure(codec, splice(&specs, &library), format.clone());
        assert_eq!(diagnostic.code(), code, "{format}: {diagnostic}");
        // The refused module writes nothing: the output stops where the header and
        // dependencies ended.
        assert_eq!(
            written,
            partial_output(codec, &before_modules(&specs), format.clone()),
            "{format}"
        );
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
        let (diagnostic, written) = encode_failure(codec, splice(&library, &specs), format.clone());
        assert_eq!(diagnostic.code(), code, "{format}: {diagnostic}");
        // The refused module writes nothing: the output stops where the header and
        // dependencies ended.
        assert_eq!(
            written,
            partial_output(codec, &before_modules(&library), format.clone()),
            "{format}"
        );
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
    assert_eq!(expected["formatVersion"], 4, "{expected}");

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

/// A Specs body whose own module is shaped as a definition, which the Library tag would hold.
const SPECS_WITH_A_DEFINITION: &str = r#"{"formatVersion":"3.1.0","distribution":["Specs",[["my"],["pkg"]],[],{"modules":[[[["basics"]],{"access":"Public","value":{"types":[],"values":[],"doc":"Basics."}}]]}]}"#;

#[test]
fn a_specs_distribution_holding_a_module_definition_is_refused() {
    for (codec, format) in [
        (&JsonCodec::new() as &dyn IrCodec, FormatId::json()),
        (&YamlCodec::new() as &dyn IrCodec, FormatId::yaml()),
    ] {
        let diagnostic = decode_failure(codec, SPECS_WITH_A_DEFINITION, format.clone());
        assert!(
            diagnostic
                .message()
                .contains("holds module specifications, not definitions"),
            "{format}: {diagnostic}"
        );
    }
}

#[test]
fn a_lowercase_specs_tag_decodes_through_the_streaming_json_decoder() {
    let lowercase = SPECS.replace(r#"["Specs","#, r#"["specs","#);
    assert_eq!(
        events(&JsonCodec::new(), &lowercase, FormatId::json()),
        events(&JsonCodec::new(), SPECS, FormatId::json())
    );
}

#[test]
fn an_unknown_distribution_tag_names_both_kinds_in_the_streaming_json_decoder() {
    let unknown = SPECS.replace(r#"["Specs","#, r#"["Application","#);
    let diagnostic = decode_failure(&JsonCodec::new(), &unknown, FormatId::json());
    let message = diagnostic.message();
    assert!(
        message.contains("Library") && message.contains("Specs"),
        "{diagnostic}"
    );
}
