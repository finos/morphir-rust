//! JSON codec entry point.

use std::collections::HashSet;
use std::io::{Read, Write};

use morphir_core::ir::v4;
use morphir_core::traversal::{
    DependencyEvent, DistributionHeader, IrCursor, ModuleEvent, SemanticEvent, SemanticEventKind,
};

use super::semantic::{self, ClassicEventVisitor, SemanticFile};
use super::single_file::deserialize_classic_v3;
use super::{
    ClassicV3ModuleVisitor, CodecOptions, EventSink, EventSource, FormatId,
    IR_RECURSION_STACK_BYTES, IrCodec, IrVersion, SourceSpan, Stage, TransportDiagnostic,
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
                let mut deserializer = serde_json::Deserializer::from_reader(reader);
                let mut visitor = ClassicEventVisitor::new(sink);
                if let Err(error) = deserialize_classic_v3(&mut deserializer, &mut visitor) {
                    if let Some(diagnostic) = visitor.take_failure() {
                        return Err(diagnostic);
                    }
                    return Err(Self::decode_error(error));
                }
                deserializer.end().map_err(Self::decode_error)?;
                visitor.finish().map_err(|message| {
                    TransportDiagnostic::error(
                        "morphir::ir::json::visitor_failed",
                        Stage::Normalization,
                        IrCursor::root(),
                        message,
                    )
                    .with_guidance("verify the semantic sink and concrete v3 event order")
                })?
            }
            IrVersion::V4 => {
                let file: v4::IRFile =
                    serde_json::from_reader(reader).map_err(Self::decode_error)?;
                semantic::emit_v4(file, sink)
            }
        }
    }

    fn encoder<'writer>(
        &self,
        writer: &'writer mut dyn Write,
        options: &CodecOptions,
    ) -> Result<Box<dyn EventSink + 'writer>, TransportDiagnostic> {
        match options.version() {
            IrVersion::V3 => Ok(Box::new(V3JsonEventEncoder::new(writer))),
            IrVersion::V4 => Ok(Box::new(V4JsonEventEncoder::new(writer))),
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

struct V3JsonEventEncoder<'writer> {
    writer: &'writer mut dyn Write,
    began: bool,
    first_dependency: bool,
    modules_started: bool,
    first_module: bool,
    ended: bool,
}

impl<'writer> V3JsonEventEncoder<'writer> {
    fn new(writer: &'writer mut dyn Write) -> Self {
        Self {
            writer,
            began: false,
            first_dependency: true,
            modules_started: false,
            first_module: true,
            ended: false,
        }
    }

    fn write(&mut self, value: impl AsRef<[u8]>) -> Result<(), TransportDiagnostic> {
        self.writer
            .write_all(value.as_ref())
            .map_err(JsonCodec::encode_error)
    }

    fn write_json(&mut self, value: &impl serde::Serialize) -> Result<(), TransportDiagnostic> {
        stacker::grow(IR_RECURSION_STACK_BYTES, || {
            serde_json::to_writer(&mut self.writer, value)
        })
        .map_err(JsonCodec::encode_error)
    }

    fn start_modules(&mut self, cursor: &IrCursor) -> Result<(), TransportDiagnostic> {
        if !self.began {
            return Err(json_stream_error(
                "missing_begin",
                cursor,
                "a module appeared before the distribution header",
            ));
        }
        if !self.modules_started {
            self.write(b"],{\"modules\":[")?;
            self.modules_started = true;
        }
        Ok(())
    }
}

impl EventSink for V3JsonEventEncoder<'_> {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        if self.ended {
            return Err(json_stream_error(
                "event_after_end",
                event.cursor(),
                "an event appeared after the distribution end",
            ));
        }
        let (cursor, kind) = event.into_parts();
        match kind {
            SemanticEventKind::Begin(DistributionHeader::ClassicV3Library { package }) => {
                if self.began {
                    return Err(json_stream_error(
                        "duplicate_begin",
                        &cursor,
                        "the JSON encoder received more than one distribution header",
                    ));
                }
                self.write(b"{\"formatVersion\":3,\"distribution\":[\"Library\",")?;
                self.write_json(&package)?;
                self.write(b",[")?;
                self.began = true;
                Ok(())
            }
            SemanticEventKind::Begin(_) => Err(json_stream_error(
                "version_mismatch",
                &cursor,
                "the v3 JSON encoder received a v4 header",
            )),
            SemanticEventKind::Dependency(DependencyEvent::ClassicV3 {
                package,
                specification,
            }) => {
                if !self.began || self.modules_started {
                    return Err(json_stream_error(
                        "dependency_out_of_order",
                        &cursor,
                        "a dependency appeared outside the dependency sequence",
                    ));
                }
                if !self.first_dependency {
                    self.write(b",")?;
                }
                self.write_json(&(package, specification))?;
                self.first_dependency = false;
                Ok(())
            }
            SemanticEventKind::Dependency(_) => Err(json_stream_error(
                "version_mismatch",
                &cursor,
                "the v3 JSON encoder received a v4 dependency",
            )),
            SemanticEventKind::Module(ModuleEvent::ClassicV3(module)) => {
                self.start_modules(&cursor)?;
                if !self.first_module {
                    self.write(b",")?;
                }
                self.write_json(&module)?;
                self.first_module = false;
                Ok(())
            }
            SemanticEventKind::Module(_) => Err(json_stream_error(
                "version_mismatch",
                &cursor,
                "the v3 JSON encoder received a v4 module",
            )),
            SemanticEventKind::End => {
                self.start_modules(&cursor)?;
                self.write(b"]}]}\n")?;
                self.ended = true;
                Ok(())
            }
        }
    }

    fn finish(&mut self) -> Result<(), TransportDiagnostic> {
        if !self.ended {
            return Err(json_stream_error(
                "missing_end",
                &IrCursor::root(),
                "the event source ended before the distribution end",
            ));
        }
        self.writer.flush().map_err(JsonCodec::encode_error)
    }
}

enum V4JsonDistribution {
    Library,
    Specs,
    Application(v4::EntryPoints),
}

struct V4JsonEventEncoder<'writer> {
    writer: &'writer mut dyn Write,
    distribution: Option<V4JsonDistribution>,
    dependencies_started: bool,
    first_dependency: bool,
    modules_started: bool,
    first_module: bool,
    dependency_names: HashSet<String>,
    module_names: HashSet<String>,
    ended: bool,
}

impl<'writer> V4JsonEventEncoder<'writer> {
    fn new(writer: &'writer mut dyn Write) -> Self {
        Self {
            writer,
            distribution: None,
            dependencies_started: false,
            first_dependency: true,
            modules_started: false,
            first_module: true,
            dependency_names: HashSet::new(),
            module_names: HashSet::new(),
            ended: false,
        }
    }

    fn write(&mut self, value: impl AsRef<[u8]>) -> Result<(), TransportDiagnostic> {
        self.writer
            .write_all(value.as_ref())
            .map_err(JsonCodec::encode_error)
    }

    fn write_json(&mut self, value: &impl serde::Serialize) -> Result<(), TransportDiagnostic> {
        stacker::grow(IR_RECURSION_STACK_BYTES, || {
            serde_json::to_writer(&mut self.writer, value)
        })
        .map_err(JsonCodec::encode_error)
    }

    fn begin(
        &mut self,
        header: DistributionHeader,
        cursor: &IrCursor,
    ) -> Result<(), TransportDiagnostic> {
        if self.distribution.is_some() {
            return Err(json_stream_error(
                "duplicate_begin",
                cursor,
                "the JSON encoder received more than one distribution header",
            ));
        }
        let (format_version, package, tag, distribution) = match header {
            DistributionHeader::V4Library {
                format_version,
                package,
            } => (
                format_version,
                package,
                "Library",
                V4JsonDistribution::Library,
            ),
            DistributionHeader::V4Specs {
                format_version,
                package,
            } => (format_version, package, "Specs", V4JsonDistribution::Specs),
            DistributionHeader::V4Application {
                format_version,
                package,
                entry_points,
            } => (
                format_version,
                package,
                "Application",
                V4JsonDistribution::Application(entry_points),
            ),
            _ => {
                return Err(json_stream_error(
                    "version_mismatch",
                    cursor,
                    "the v4 JSON encoder received a Classic v3 header",
                ));
            }
        };
        self.write(b"{\"formatVersion\":")?;
        self.write_json(&format_version)?;
        self.write(format!(",\"distribution\":{{\"{tag}\":{{\"packageName\":"))?;
        self.write_json(&package)?;
        self.distribution = Some(distribution);
        Ok(())
    }

    fn dependency(
        &mut self,
        dependency: DependencyEvent,
        cursor: &IrCursor,
    ) -> Result<(), TransportDiagnostic> {
        if self.modules_started {
            return Err(json_stream_error(
                "dependency_after_module",
                cursor,
                "a dependency appeared after the first module",
            ));
        }
        let DependencyEvent::V4 {
            package,
            specification,
        } = dependency
        else {
            return Err(json_stream_error(
                "version_mismatch",
                cursor,
                "the v4 JSON encoder received a Classic v3 dependency",
            ));
        };
        if !self.dependency_names.insert(package.clone()) {
            return Err(json_stream_error(
                "duplicate_dependency",
                cursor,
                "the event stream contains a duplicate dependency name",
            ));
        }
        if !self.dependencies_started {
            self.write(b",\"dependencies\":{")?;
            self.dependencies_started = true;
        }
        if !self.first_dependency {
            self.write(b",")?;
        }
        self.write_json(&package)?;
        self.write(b":")?;
        self.write_json(&specification)?;
        self.first_dependency = false;
        Ok(())
    }

    fn start_modules(&mut self) -> Result<(), TransportDiagnostic> {
        if !self.dependencies_started {
            self.write(b",\"dependencies\":{}")?;
            self.dependencies_started = true;
        } else {
            self.write(b"}")?;
        }
        if !self.modules_started {
            let field = match self.distribution {
                Some(V4JsonDistribution::Library | V4JsonDistribution::Application(_)) => "def",
                Some(V4JsonDistribution::Specs) => "spec",
                None => {
                    return Err(json_stream_error(
                        "missing_begin",
                        &IrCursor::root(),
                        "a module appeared before the distribution header",
                    ));
                }
            };
            self.write(format!(",\"{field}\":{{\"modules\":{{"))?;
            self.modules_started = true;
        }
        Ok(())
    }

    fn module(
        &mut self,
        module: ModuleEvent,
        cursor: &IrCursor,
    ) -> Result<(), TransportDiagnostic> {
        if !self.modules_started {
            self.start_modules()?;
        }
        let (path, value, specification) = match module {
            ModuleEvent::V4Definition { path, module } => (path, Some(module), None),
            ModuleEvent::V4Specification { path, module } => (path, None, Some(module)),
            ModuleEvent::ClassicV3(_) => {
                return Err(json_stream_error(
                    "version_mismatch",
                    cursor,
                    "the v4 JSON encoder received a Classic v3 module",
                ));
            }
        };
        let matches_distribution = matches!(
            (&self.distribution, &value, &specification),
            (
                Some(V4JsonDistribution::Library | V4JsonDistribution::Application(_)),
                Some(_),
                None
            ) | (Some(V4JsonDistribution::Specs), None, Some(_))
        );
        if !matches_distribution {
            return Err(json_stream_error(
                "module_kind_mismatch",
                cursor,
                "the module event does not match the v4 distribution kind",
            ));
        }
        if !self.module_names.insert(path.clone()) {
            return Err(json_stream_error(
                "duplicate_module",
                cursor,
                "the event stream contains a duplicate module name",
            ));
        }
        if !self.first_module {
            self.write(b",")?;
        }
        self.write_json(&path)?;
        self.write(b":")?;
        match (value, specification) {
            (Some(value), None) => self.write_json(&value)?,
            (None, Some(value)) => self.write_json(&value)?,
            _ => unreachable!("module kind was validated above"),
        }
        self.first_module = false;
        Ok(())
    }

    fn end(&mut self, cursor: &IrCursor) -> Result<(), TransportDiagnostic> {
        if self.ended {
            return Err(json_stream_error(
                "duplicate_end",
                cursor,
                "the JSON encoder received more than one distribution end",
            ));
        }
        if self.distribution.is_none() {
            return Err(json_stream_error(
                "missing_begin",
                cursor,
                "the distribution ended before its header",
            ));
        }
        if !self.modules_started {
            self.start_modules()?;
        }
        self.write(b"}}")?;
        if let Some(V4JsonDistribution::Application(entry_points)) = self.distribution.take() {
            self.write(b",\"entryPoints\":")?;
            self.write_json(&entry_points)?;
        }
        self.write(b"}}}\n")?;
        self.ended = true;
        Ok(())
    }
}

impl EventSink for V4JsonEventEncoder<'_> {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        if self.ended {
            return Err(json_stream_error(
                "event_after_end",
                event.cursor(),
                "an event appeared after the distribution end",
            ));
        }
        let (cursor, kind) = event.into_parts();
        match kind {
            SemanticEventKind::Begin(header) => self.begin(header, &cursor),
            SemanticEventKind::Dependency(dependency) => self.dependency(dependency, &cursor),
            SemanticEventKind::Module(module) => self.module(module, &cursor),
            SemanticEventKind::End => self.end(&cursor),
        }
    }

    fn finish(&mut self) -> Result<(), TransportDiagnostic> {
        if !self.ended {
            return Err(json_stream_error(
                "missing_end",
                &IrCursor::root(),
                "the event source ended before the distribution end",
            ));
        }
        self.writer.flush().map_err(JsonCodec::encode_error)
    }
}

fn json_stream_error(
    suffix: &'static str,
    cursor: &IrCursor,
    message: &'static str,
) -> TransportDiagnostic {
    TransportDiagnostic::error(
        format!("morphir::ir::json::{suffix}"),
        Stage::Encoding,
        cursor.clone(),
        message,
    )
    .with_guidance("verify the semantic event order and selected concrete IR version")
}
