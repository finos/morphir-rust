use std::cell::Cell;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::rc::Rc;

use morphir_common::ir_transport::{
    ClassicToV4, CodecOptions, EventSink, EventSource, EventTransform, FormatId, IrCodec,
    IrVersion, JsonCodec, Layout, Pipeline, Retention, TransportDiagnostic, YamlCodec,
};
use morphir_core::ir::classic;
use morphir_core::migration::{MigrationOptions, migrate_distribution};
use morphir_core::traversal::{IrCursor, SemanticEvent, SemanticEventKind};

struct QueueSource(VecDeque<SemanticEvent>);

impl EventSource for QueueSource {
    fn next_event(&mut self) -> Result<Option<SemanticEvent>, TransportDiagnostic> {
        Ok(self.0.pop_front())
    }
}

#[derive(Default)]
struct CollectingSink {
    events: Vec<SemanticEvent>,
    finished: bool,
}

impl EventSink for CollectingSink {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        self.events.push(event);
        Ok(())
    }

    fn finish(&mut self) -> Result<(), TransportDiagnostic> {
        self.finished = true;
        Ok(())
    }
}

struct Identity(Retention);

impl EventTransform for Identity {
    fn retention(&self) -> Retention {
        self.0
    }

    fn transform(
        &mut self,
        event: SemanticEvent,
        emit: &mut dyn FnMut(SemanticEvent) -> Result<(), TransportDiagnostic>,
    ) -> Result<(), TransportDiagnostic> {
        emit(event)
    }
}

struct Duplicate;

impl EventTransform for Duplicate {
    fn retention(&self) -> Retention {
        Retention::Event
    }

    fn transform(
        &mut self,
        event: SemanticEvent,
        emit: &mut dyn FnMut(SemanticEvent) -> Result<(), TransportDiagnostic>,
    ) -> Result<(), TransportDiagnostic> {
        emit(event.clone())?;
        emit(event)
    }
}

fn end_event() -> SemanticEvent {
    SemanticEvent::new(IrCursor::root(), SemanticEventKind::End)
}

#[test]
fn pipeline_composes_zero_one_or_many_event_transforms() {
    let mut pipeline = Pipeline::new()
        .with_transform(Identity(Retention::Event))
        .with_transform(Duplicate);
    let mut source = QueueSource([end_event()].into());
    let mut sink = CollectingSink::default();

    pipeline.run(&mut source, &mut sink).unwrap();

    assert_eq!(sink.events, vec![end_event(), end_event()]);
    assert!(sink.finished);
}

#[test]
fn pipeline_reports_the_largest_declared_retention() {
    let pipeline = Pipeline::new()
        .with_transform(Identity(Retention::Event))
        .with_transform(Identity(Retention::Module))
        .with_transform(Identity(Retention::Definition));

    assert_eq!(pipeline.retention(), Retention::Module);
    assert!(pipeline.require_bounded().is_ok());
}

#[test]
fn whole_distribution_retention_is_rejected_before_source_io() {
    let pipeline = Pipeline::new().with_transform(Identity(Retention::Distribution));

    let diagnostic = pipeline.require_bounded().unwrap_err();
    assert_eq!(
        diagnostic.code(),
        "morphir::ir::pipeline::whole_distribution_required"
    );
}

fn decode_json(input: &str, version: IrVersion) -> Vec<SemanticEvent> {
    let codec = JsonCodec::new();
    let options = CodecOptions::new(version, Layout::SingleFile, FormatId::json());
    let mut reader = std::io::Cursor::new(input.as_bytes());
    let mut sink = CollectingSink::default();
    codec.decode(&mut reader, &options, &mut sink).unwrap();
    sink.events
}

#[test]
fn classic_to_v4_is_a_module_bounded_semantic_transform() {
    let source_json = r#"{
      "formatVersion": 3,
      "distribution": [
        "Library",
        [["example"]],
        [],
        {"modules": [
          [
            [["orders"]],
            {
              "access": "Public",
              "value": {"types": [], "values": [], "doc": null}
            }
          ]
        ]}
      ]
    }"#;
    let original: classic::Distribution = serde_json::from_str(source_json).unwrap();
    let migrated = migrate_distribution(&original, MigrationOptions::default()).unwrap();
    let expected_json = serde_json::to_string(&migrated.value).unwrap();
    let expected = decode_json(&expected_json, IrVersion::V4);

    let transform = ClassicToV4::new(MigrationOptions::default());
    let report = transform.report_handle();
    let mut pipeline = Pipeline::new().with_transform(transform);
    let mut source = QueueSource(decode_json(source_json, IrVersion::V3).into());
    let mut sink = CollectingSink::default();

    assert_eq!(pipeline.retention(), Retention::Module);
    pipeline.run(&mut source, &mut sink).unwrap();
    assert_eq!(sink.events, expected);
    assert!(report.get().unwrap().can_publish());
}

struct ChunkedReader<R> {
    inner: R,
    bytes_read: Rc<Cell<usize>>,
}

impl<R: Read> Read for ChunkedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let length = buffer.len().min(1024);
        let read = self.inner.read(&mut buffer[..length])?;
        self.bytes_read.set(self.bytes_read.get() + read);
        Ok(read)
    }
}

struct FirstModuleObserver {
    bytes_read: Rc<Cell<usize>>,
    first_module_offset: Rc<Cell<usize>>,
}

impl EventTransform for FirstModuleObserver {
    fn retention(&self) -> Retention {
        Retention::Event
    }

    fn transform(
        &mut self,
        event: SemanticEvent,
        emit: &mut dyn FnMut(SemanticEvent) -> Result<(), TransportDiagnostic>,
    ) -> Result<(), TransportDiagnostic> {
        if matches!(event.kind(), SemanticEventKind::Module(_))
            && self.first_module_offset.get() == 0
        {
            self.first_module_offset.set(self.bytes_read.get());
        }
        emit(event)
    }
}

fn large_classic_v3_source(module_count: usize, module_padding: usize) -> Vec<u8> {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ClassicFile {
        format_version: u32,
        distribution: serde_json::Value,
    }

    let modules = (0..module_count)
        .map(|index| {
            serde_json::json!([
                [["module", index.to_string()]],
                {
                    "access": "Public",
                    "value": {
                        "types": [],
                        "values": [],
                        "doc": "x".repeat(module_padding),
                    }
                }
            ])
        })
        .collect::<Vec<_>>();
    serde_json::to_vec(&ClassicFile {
        format_version: 3,
        distribution: serde_json::json!([
            "Library",
            [["streaming"], ["fixture"]],
            [],
            { "modules": modules }
        ]),
    })
    .unwrap()
}

#[test]
fn lcr_scale_json_to_yaml_releases_modules_before_end_of_input() {
    let input = large_classic_v3_source(84, 42_000);
    let bytes_read = Rc::new(Cell::new(0));
    let first_module_offset = Rc::new(Cell::new(0));
    let mut reader = ChunkedReader {
        inner: std::io::Cursor::new(input.as_slice()),
        bytes_read: bytes_read.clone(),
    };
    let input_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json());
    let output_options = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::yaml());
    let mut output = Vec::new();
    let yaml = YamlCodec::new();
    let mut encoder = yaml.encoder(&mut output, &output_options).unwrap();
    let mut pipeline = Pipeline::new()
        .with_transform(FirstModuleObserver {
            bytes_read,
            first_module_offset: first_module_offset.clone(),
        })
        .with_transform(ClassicToV4::new(MigrationOptions::default()));
    {
        let mut sink = pipeline.sink(encoder.as_mut()).unwrap();
        JsonCodec::new()
            .decode(&mut reader, &input_options, &mut sink)
            .unwrap();
    }
    drop(encoder);

    assert_eq!(pipeline.retention(), Retention::Module);
    assert!(first_module_offset.get() < input.len() / 10);
    assert!(
        std::str::from_utf8(&output)
            .unwrap()
            .starts_with("formatVersion: 4")
    );
    let mut output_reader = std::io::Cursor::new(output.as_slice());
    let mut output_sink = CollectingSink::default();
    yaml.decode(&mut output_reader, &output_options, &mut output_sink)
        .unwrap();
    assert_eq!(
        output_sink
            .events
            .iter()
            .filter(|event| matches!(event.kind(), SemanticEventKind::Module(_)))
            .count(),
        84
    );
}

#[test]
fn real_lcr_v3_migrates_to_native_yaml_incrementally() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../morphir-tests/tests/features/lcr_v3.json");
    let input_length = path.metadata().unwrap().len() as usize;
    let bytes_read = Rc::new(Cell::new(0));
    let first_module_offset = Rc::new(Cell::new(0));
    let mut reader = ChunkedReader {
        inner: File::open(path).unwrap(),
        bytes_read: bytes_read.clone(),
    };
    let input_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json());
    let output_options = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::yaml());
    let mut output = Vec::new();
    let yaml = YamlCodec::new();
    let mut encoder = yaml.encoder(&mut output, &output_options).unwrap();
    let mut pipeline = Pipeline::new()
        .with_transform(FirstModuleObserver {
            bytes_read,
            first_module_offset: first_module_offset.clone(),
        })
        .with_transform(ClassicToV4::new(MigrationOptions::default()));
    {
        let mut sink = pipeline.sink(encoder.as_mut()).unwrap();
        JsonCodec::new()
            .decode(&mut reader, &input_options, &mut sink)
            .unwrap();
    }
    drop(encoder);

    assert!(first_module_offset.get() < input_length / 10);
    assert!(output.starts_with(b"formatVersion: 4"));
    let mut output_reader = std::io::Cursor::new(output);
    let mut output_sink = CollectingSink::default();
    yaml.decode(&mut output_reader, &output_options, &mut output_sink)
        .unwrap();
    assert!(
        output_sink
            .events
            .iter()
            .any(|event| { matches!(event.kind(), SemanticEventKind::Module(_)) })
    );
}

#[test]
fn streaming_json_encoder_produces_a_complete_v4_document() {
    let input = large_classic_v3_source(3, 128);
    let input_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json());
    let output_options = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::json());
    let mut reader = std::io::Cursor::new(input);
    let mut output = Vec::new();
    let json = JsonCodec::new();
    let mut encoder = json.encoder(&mut output, &output_options).unwrap();
    let mut pipeline =
        Pipeline::new().with_transform(ClassicToV4::new(MigrationOptions::default()));
    {
        let mut sink = pipeline.sink(encoder.as_mut()).unwrap();
        JsonCodec::new()
            .decode(&mut reader, &input_options, &mut sink)
            .unwrap();
    }
    drop(encoder);

    let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(value["formatVersion"], 4);
    let mut output_reader = std::io::Cursor::new(output);
    let mut output_sink = CollectingSink::default();
    json.decode(&mut output_reader, &output_options, &mut output_sink)
        .unwrap();
    assert_eq!(
        output_sink
            .events
            .iter()
            .filter(|event| matches!(event.kind(), SemanticEventKind::Module(_)))
            .count(),
        3
    );
}

#[test]
fn streaming_yaml_encoder_handles_an_empty_module_set() {
    let input = large_classic_v3_source(0, 0);
    let input_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json());
    let output_options = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::yaml());
    let mut reader = std::io::Cursor::new(input);
    let mut output = Vec::new();
    let yaml = YamlCodec::new();
    let mut encoder = yaml.encoder(&mut output, &output_options).unwrap();
    let mut pipeline =
        Pipeline::new().with_transform(ClassicToV4::new(MigrationOptions::default()));
    {
        let mut sink = pipeline.sink(encoder.as_mut()).unwrap();
        JsonCodec::new()
            .decode(&mut reader, &input_options, &mut sink)
            .unwrap();
    }
    drop(encoder);

    let mut output_reader = std::io::Cursor::new(output);
    let mut output_sink = CollectingSink::default();
    yaml.decode(&mut output_reader, &output_options, &mut output_sink)
        .unwrap();
    assert_eq!(
        output_sink
            .events
            .iter()
            .filter(|event| matches!(event.kind(), SemanticEventKind::Module(_)))
            .count(),
        0
    );
}
