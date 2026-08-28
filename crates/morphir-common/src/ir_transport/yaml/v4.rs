//! Push encoder for concrete v4 YAML.

use std::collections::HashSet;
use std::io::Write;

use morphir_core::ir::v4;
use morphir_core::traversal::{
    DependencyEvent, DistributionHeader, IrCursor, ModuleEvent, SemanticEvent, SemanticEventKind,
};

use super::{YamlCodec, stream_event_error};
use crate::ir_transport::{EventSink, TransportDiagnostic};

enum V4YamlDistribution {
    Library,
    Specs,
    Application(v4::EntryPoints),
}

pub(super) struct V4YamlEventEncoder<'writer> {
    writer: &'writer mut dyn Write,
    distribution: Option<V4YamlDistribution>,
    dependencies_started: bool,
    modules_started: bool,
    dependency_names: HashSet<String>,
    module_names: HashSet<String>,
    ended: bool,
}

impl<'writer> V4YamlEventEncoder<'writer> {
    pub(super) fn new(writer: &'writer mut dyn Write) -> Self {
        Self {
            writer,
            distribution: None,
            dependencies_started: false,
            modules_started: false,
            dependency_names: HashSet::new(),
            module_names: HashSet::new(),
            ended: false,
        }
    }

    fn write(&mut self, value: impl AsRef<[u8]>) -> Result<(), TransportDiagnostic> {
        self.writer
            .write_all(value.as_ref())
            .map_err(YamlCodec::encode_error)
    }

    fn inline(value: &impl serde::Serialize) -> Result<String, TransportDiagnostic> {
        let rendered = serde_saphyr::to_string_with_options(value, YamlCodec::serializer_options())
            .map_err(YamlCodec::encode_error)?;
        let rendered = rendered.trim_end_matches(['\r', '\n']);
        if rendered.contains('\n') {
            return Err(YamlCodec::encode_error(
                "a mapping key or header scalar required multiple YAML lines",
            ));
        }
        Ok(rendered.to_owned())
    }

    fn write_indented(
        &mut self,
        value: &impl serde::Serialize,
        indent: usize,
    ) -> Result<(), TransportDiagnostic> {
        let rendered = serde_saphyr::to_string_with_options(value, YamlCodec::serializer_options())
            .map_err(YamlCodec::encode_error)?
            .replace("\r\n", "\n");
        let padding = " ".repeat(indent);
        for line in rendered.trim_end_matches('\n').lines() {
            self.write(&padding)?;
            self.write(line)?;
            self.write(b"\n")?;
        }
        Ok(())
    }

    fn begin(
        &mut self,
        header: DistributionHeader,
        cursor: &IrCursor,
    ) -> Result<(), TransportDiagnostic> {
        if self.distribution.is_some() {
            return Err(stream_event_error(
                "duplicate_begin",
                cursor,
                "the YAML encoder received more than one distribution header",
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
                V4YamlDistribution::Library,
            ),
            DistributionHeader::V4Specs {
                format_version,
                package,
            } => (format_version, package, "Specs", V4YamlDistribution::Specs),
            DistributionHeader::V4Application {
                format_version,
                package,
                entry_points,
            } => (
                format_version,
                package,
                "Application",
                V4YamlDistribution::Application(entry_points),
            ),
            _ => {
                return Err(stream_event_error(
                    "version_mismatch",
                    cursor,
                    "the v4 YAML encoder received a Classic v3 header",
                ));
            }
        };
        let version = Self::inline(&format_version)?;
        let package = Self::inline(&package)?;
        self.write(format!(
            "formatVersion: {version}\ndistribution:\n  {tag}:\n    packageName: {package}\n"
        ))?;
        self.distribution = Some(distribution);
        Ok(())
    }

    fn dependency(
        &mut self,
        dependency: DependencyEvent,
        cursor: &IrCursor,
    ) -> Result<(), TransportDiagnostic> {
        if self.modules_started {
            return Err(stream_event_error(
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
            return Err(stream_event_error(
                "version_mismatch",
                cursor,
                "the v4 YAML encoder received a Classic v3 dependency",
            ));
        };
        if !self.dependency_names.insert(package.clone()) {
            return Err(stream_event_error(
                "duplicate_dependency",
                cursor,
                "the event stream contains a duplicate dependency name",
            ));
        }
        if !self.dependencies_started {
            self.write(b"    dependencies:\n")?;
            self.dependencies_started = true;
        }
        let package = Self::inline(&package)?;
        self.write(format!("      {package}:\n"))?;
        self.write_indented(&specification, 8)
    }

    fn start_modules(&mut self) -> Result<(), TransportDiagnostic> {
        if !self.dependencies_started {
            self.write(b"    dependencies: {}\n")?;
            self.dependencies_started = true;
        }
        if !self.modules_started {
            let field = match self.distribution {
                Some(V4YamlDistribution::Library | V4YamlDistribution::Application(_)) => "def",
                Some(V4YamlDistribution::Specs) => "spec",
                None => {
                    return Err(stream_event_error(
                        "missing_begin",
                        &IrCursor::root(),
                        "a module appeared before the distribution header",
                    ));
                }
            };
            self.write(format!("    {field}:\n      modules:\n"))?;
            self.modules_started = true;
        }
        Ok(())
    }

    fn module(
        &mut self,
        module: ModuleEvent,
        cursor: &IrCursor,
    ) -> Result<(), TransportDiagnostic> {
        self.start_modules()?;
        let (path, value, specification) = match module {
            ModuleEvent::V4Definition { path, module } => (path, Some(module), None),
            ModuleEvent::V4Specification { path, module } => (path, None, Some(module)),
            ModuleEvent::ClassicV3(_) => {
                return Err(stream_event_error(
                    "version_mismatch",
                    cursor,
                    "the v4 YAML encoder received a Classic v3 module",
                ));
            }
        };
        let matches_distribution = matches!(
            (&self.distribution, &value, &specification),
            (
                Some(V4YamlDistribution::Library | V4YamlDistribution::Application(_)),
                Some(_),
                None
            ) | (Some(V4YamlDistribution::Specs), None, Some(_))
        );
        if !matches_distribution {
            return Err(stream_event_error(
                "module_kind_mismatch",
                cursor,
                "the module event does not match the v4 distribution kind",
            ));
        }
        if !self.module_names.insert(path.clone()) {
            return Err(stream_event_error(
                "duplicate_module",
                cursor,
                "the event stream contains a duplicate module name",
            ));
        }
        let path = Self::inline(&path)?;
        self.write(format!("        {path}:\n"))?;
        match (value, specification) {
            (Some(value), None) => self.write_indented(&value, 10),
            (None, Some(value)) => self.write_indented(&value, 10),
            _ => unreachable!("module kind was validated above"),
        }
    }

    fn end(&mut self, cursor: &IrCursor) -> Result<(), TransportDiagnostic> {
        if self.ended {
            return Err(stream_event_error(
                "duplicate_end",
                cursor,
                "the YAML encoder received more than one distribution end",
            ));
        }
        if self.distribution.is_none() {
            return Err(stream_event_error(
                "missing_begin",
                cursor,
                "the distribution ended before its header",
            ));
        }
        self.start_modules()?;
        if self.module_names.is_empty() {
            // Replace the open mapping with an explicit empty mapping entry.
            self.write(b"          {}\n")?;
        }
        if let Some(V4YamlDistribution::Application(entry_points)) = self.distribution.take() {
            if entry_points.is_empty() {
                self.write(b"    entryPoints: {}\n")?;
            } else {
                self.write(b"    entryPoints:\n")?;
                self.write_indented(&entry_points, 6)?;
            }
        }
        self.ended = true;
        Ok(())
    }
}

impl EventSink for V4YamlEventEncoder<'_> {
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
            SemanticEventKind::Begin(header) => self.begin(header, &cursor),
            SemanticEventKind::Dependency(dependency) => self.dependency(dependency, &cursor),
            SemanticEventKind::Module(module) => self.module(module, &cursor),
            SemanticEventKind::End => self.end(&cursor),
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
