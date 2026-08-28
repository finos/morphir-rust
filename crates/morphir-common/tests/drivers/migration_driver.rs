//! Driver for format-neutral streaming migration scenarios.

use std::io::Cursor;

use morphir_common::ir_transport::{
    ClassicToV4, CodecOptions, EventSink, FormatId, IrCodec, IrVersion, JsonCodec, Layout,
    Pipeline, Retention, TransportDiagnostic, YamlCodec,
};
use morphir_core::migration::MigrationOptions;
use morphir_core::traversal::{DistributionHeader, SemanticEvent, SemanticEventKind};

#[derive(Debug, Default)]
pub struct MigrationDriver {
    input: Vec<u8>,
    output: Vec<u8>,
    retention: Option<Retention>,
    report_publishable: Option<bool>,
    failure: Option<TransportDiagnostic>,
}

impl MigrationDriver {
    pub fn given_classic_v3_json(&mut self) {
        self.input = a_classic_v3_distribution().into_bytes();
    }

    pub fn when_streaming_to_native_v4_yaml(&mut self) {
        self.failure = None;
        self.output.clear();

        let transform = ClassicToV4::new(MigrationOptions::default());
        let report = transform.report_handle();
        let mut pipeline = Pipeline::new().with_transform(transform);
        self.retention = Some(pipeline.retention());
        let input_options = CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json());
        let output_options = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::yaml());
        let mut output = Vec::new();

        let result = (|| {
            let mut encoder = YamlCodec::new().encoder(&mut output, &output_options)?;
            {
                let mut sink = pipeline.sink(encoder.as_mut())?;
                JsonCodec::new().decode(
                    &mut Cursor::new(&self.input),
                    &input_options,
                    &mut sink,
                )?;
            }
            drop(encoder);
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.output = output;
                self.report_publishable = report.get().map(|value| value.can_publish());
            }
            Err(error) => self.failure = Some(error),
        }
    }

    pub fn assert_concrete_v4_yaml(&self) {
        self.assert_succeeded();
        let source = std::str::from_utf8(&self.output).expect("migration output should be UTF-8");
        assert!(source.starts_with("formatVersion: 4\n"));
        assert!(!source.trim_start().starts_with('{'));

        let mut sink = V4HeaderSink::default();
        YamlCodec::new()
            .decode(
                &mut Cursor::new(&self.output),
                &CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::yaml()),
                &mut sink,
            )
            .expect("migration output should decode as concrete v4 YAML");
        assert!(sink.saw_v4_header);
    }

    pub fn assert_module_bounded(&self) {
        self.assert_succeeded();
        assert_eq!(self.retention, Some(Retention::Module));
    }

    pub fn assert_report_publishable(&self) {
        self.assert_succeeded();
        assert_eq!(self.report_publishable, Some(true));
    }

    fn assert_succeeded(&self) {
        assert!(
            self.failure.is_none(),
            "streaming migration failed: {:?}",
            self.failure
        );
    }
}

#[derive(Default)]
struct V4HeaderSink {
    saw_v4_header: bool,
}

impl EventSink for V4HeaderSink {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        if matches!(
            event.kind(),
            SemanticEventKind::Begin(DistributionHeader::V4Library { .. })
        ) {
            self.saw_v4_header = true;
        }
        Ok(())
    }
}

fn a_classic_v3_distribution() -> String {
    r#"{
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
    }"#
    .to_owned()
}
