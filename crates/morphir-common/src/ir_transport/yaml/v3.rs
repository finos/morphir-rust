//! Push encoder for concrete Classic v3 YAML.

use std::io::Write;

use morphir_core::traversal::{
    DependencyEvent, DistributionHeader, IrCursor, ModuleEvent, SemanticEvent, SemanticEventKind,
};
use serde::Serialize;

use super::{YamlCodec, encode_document, stream_event_error};
use crate::ir_transport::{EventSink, TransportDiagnostic};

pub(super) struct V3YamlEventEncoder<'writer> {
    writer: &'writer mut dyn Write,
    began: bool,
    dependencies_started: bool,
    modules_started: bool,
    first_module: bool,
    ended: bool,
}

impl<'writer> V3YamlEventEncoder<'writer> {
    pub(super) fn new(writer: &'writer mut dyn Write) -> Self {
        Self {
            writer,
            began: false,
            dependencies_started: false,
            modules_started: false,
            first_module: true,
            ended: false,
        }
    }

    fn write(&mut self, value: impl AsRef<[u8]>) -> Result<(), TransportDiagnostic> {
        self.writer
            .write_all(value.as_ref())
            .map_err(YamlCodec::encode_error)
    }

    fn write_item(
        &mut self,
        value: &impl Serialize,
        indent: usize,
    ) -> Result<(), TransportDiagnostic> {
        let rendered =
            String::from_utf8(encode_document(value)?).map_err(YamlCodec::encode_error)?;
        let mut lines = rendered.trim_end_matches('\n').lines();
        let first = lines.next().ok_or_else(|| {
            YamlCodec::encode_error("a Classic v3 sequence item encoded as an empty document")
        })?;
        let padding = " ".repeat(indent);
        self.write(&padding)?;
        self.write(b"- ")?;
        self.write(first)?;
        self.write(b"\n")?;
        for line in lines {
            self.write(&padding)?;
            self.write(b"  ")?;
            self.write(line)?;
            self.write(b"\n")?;
        }
        Ok(())
    }

    fn start_modules(&mut self, cursor: &IrCursor) -> Result<(), TransportDiagnostic> {
        if !self.began {
            return Err(stream_event_error(
                "missing_begin",
                cursor,
                "a module appeared before the distribution header",
            ));
        }
        if !self.dependencies_started {
            self.write(b"  - []\n")?;
            self.dependencies_started = true;
        }
        if !self.modules_started {
            self.write(b"  - modules:\n")?;
            self.modules_started = true;
        }
        Ok(())
    }
}

impl EventSink for V3YamlEventEncoder<'_> {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        if self.ended {
            return Err(stream_event_error(
                "event_after_end",
                event.cursor(),
                "an event appeared after the distribution end",
            ));
        }
        let (cursor, kind) = event.into_parts();
        match kind {
            SemanticEventKind::Begin(DistributionHeader::ClassicV3Library { package }) => {
                if self.began {
                    return Err(stream_event_error(
                        "duplicate_begin",
                        &cursor,
                        "the YAML encoder received more than one distribution header",
                    ));
                }
                self.write(b"formatVersion: 3\ndistribution:\n  - Library\n")?;
                self.write_item(&package, 2)?;
                self.began = true;
                Ok(())
            }
            SemanticEventKind::Begin(_) => Err(stream_event_error(
                "version_mismatch",
                &cursor,
                "the v3 YAML encoder received a v4 header",
            )),
            SemanticEventKind::Dependency(DependencyEvent::ClassicV3 {
                package,
                specification,
            }) => {
                if !self.began || self.modules_started {
                    return Err(stream_event_error(
                        "dependency_out_of_order",
                        &cursor,
                        "a dependency appeared outside the dependency sequence",
                    ));
                }
                if !self.dependencies_started {
                    self.write(b"  -\n")?;
                    self.dependencies_started = true;
                }
                self.write_item(&(package, specification), 4)
            }
            SemanticEventKind::Dependency(_) => Err(stream_event_error(
                "version_mismatch",
                &cursor,
                "the v3 YAML encoder received a v4 dependency",
            )),
            SemanticEventKind::Module(ModuleEvent::ClassicV3(module)) => {
                self.start_modules(&cursor)?;
                self.write_item(&module, 6)?;
                self.first_module = false;
                Ok(())
            }
            SemanticEventKind::Module(_) => Err(stream_event_error(
                "version_mismatch",
                &cursor,
                "the v3 YAML encoder received a v4 module",
            )),
            SemanticEventKind::End => {
                if !self.dependencies_started {
                    self.write(b"  - []\n")?;
                    self.dependencies_started = true;
                }
                if !self.modules_started {
                    self.write(b"  - modules: []\n")?;
                    self.modules_started = true;
                } else if self.first_module {
                    self.write(b"      []\n")?;
                }
                self.ended = true;
                Ok(())
            }
        }
    }

    fn finish(&mut self) -> Result<(), TransportDiagnostic> {
        if !self.ended {
            return Err(stream_event_error(
                "missing_end",
                &IrCursor::root(),
                "the event source ended before the distribution end",
            ));
        }
        self.writer.flush().map_err(YamlCodec::encode_error)
    }
}
