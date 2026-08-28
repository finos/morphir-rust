//! Strict native YAML codec for concrete Morphir IR.

use std::collections::HashSet;
use std::io::{Read, Write};

use morphir_core::ir::{classic, v4};
use morphir_core::traversal::IrCursor;
use morphir_core::traversal::{
    DependencyEvent, DistributionHeader, ModuleEvent, SemanticEvent, SemanticEventKind,
};
use serde_saphyr::budget::BudgetBreach;
use serde_saphyr::options::{DuplicateKeyPolicy, MergeKeyPolicy};
use serde_saphyr::{Error, alias_limits, budget, options, ser_options};

use super::semantic::{self, SemanticFile};
use super::{
    CodecOptions, EventSink, EventSource, FormatId, IR_RECURSION_STACK_BYTES, IrCodec, IrVersion,
    SourceSpan, Stage, TransportDiagnostic,
};

const MAX_INPUT_BYTES: usize = 512 * 1024 * 1024;

/// Built-in native YAML IR codec.
pub struct YamlCodec {
    format: FormatId,
}

impl YamlCodec {
    /// Create the built-in YAML codec.
    pub fn new() -> Self {
        Self {
            format: FormatId::yaml(),
        }
    }

    fn parse_options() -> serde_saphyr::Options {
        options! {
            budget: budget! {
                max_reader_input_bytes: Some(MAX_INPUT_BYTES),
                max_events: 50_000_000,
                max_aliases: 0,
                max_anchors: 0,
                max_depth: 512,
                max_documents: 1,
                max_nodes: 20_000_000,
                max_total_scalar_bytes: MAX_INPUT_BYTES,
                max_merge_keys: 0,
            },
            duplicate_keys: DuplicateKeyPolicy::Error,
            merge_keys: MergeKeyPolicy::Error,
            alias_limits: alias_limits! {
                max_total_replayed_events: 0,
                max_replay_stack_depth: 0,
                max_alias_expansions_per_anchor: 0,
            },
            emit_comments: false,
            strict_booleans: true,
            legacy_octal_numbers: false,
            reject_non_finite_typeless_float: true,
            with_snippet: false,
        }
    }

    fn serializer_options() -> serde_saphyr::SerializerOptions {
        ser_options! {
            indent_step: 2,
            compact_list_indent: false,
            empty_as_braces: true,
            tagged_enums: false,
            quote_all: false,
        }
    }

    fn read_input(reader: &mut dyn Read) -> Result<Vec<u8>, TransportDiagnostic> {
        let mut input = Vec::new();
        reader
            .take((MAX_INPUT_BYTES as u64) + 1)
            .read_to_end(&mut input)
            .map_err(|error| {
                TransportDiagnostic::error(
                    "morphir::ir::yaml::read_failed",
                    Stage::Syntax,
                    IrCursor::root(),
                    error.to_string(),
                )
                .with_guidance("verify that the input is readable UTF-8 YAML")
            })?;
        if input.len() > MAX_INPUT_BYTES {
            return Err(TransportDiagnostic::error(
                "morphir::ir::yaml::input_budget_exceeded",
                Stage::Syntax,
                IrCursor::root(),
                format!("YAML input exceeds the {MAX_INPUT_BYTES}-byte safety budget"),
            )
            .with_guidance(
                "split the artifact into a document tree or raise the configured budget",
            ));
        }
        Ok(input)
    }

    fn decode_error(error: Error) -> TransportDiagnostic {
        let source_span = error.location().map(|location| {
            let span = location.span();
            SourceSpan {
                offset: span.byte_offset().unwrap_or(span.offset()) as usize,
                length: span.byte_len().unwrap_or(span.len()) as usize,
                line: location.line() as usize,
                column: location.column() as usize,
            }
        });
        let (code, stage, guidance) = match error.without_snippet() {
            Error::DuplicateMappingKey { .. } => (
                "morphir::ir::yaml::duplicate_key",
                Stage::Syntax,
                "remove the repeated mapping key",
            ),
            Error::MultipleDocuments { .. }
            | Error::Budget {
                breach: BudgetBreach::Documents { .. },
                ..
            } => (
                "morphir::ir::yaml::multiple_documents",
                Stage::Syntax,
                "store exactly one IR document in each YAML artifact",
            ),
            Error::MergeKeyNotAllowed { .. } => (
                "morphir::ir::yaml::merge_key_not_allowed",
                Stage::Syntax,
                "expand the merge explicitly as ordinary mapping entries",
            ),
            Error::NonFiniteFloat { .. } => (
                "morphir::ir::yaml::non_finite_number",
                Stage::Normalization,
                "use a finite number representable by the concrete IR literal",
            ),
            Error::AliasReplayCounterOverflow { .. }
            | Error::AliasReplayLimitExceeded { .. }
            | Error::AliasExpansionLimitExceeded { .. }
            | Error::AliasReplayStackDepthExceeded { .. }
            | Error::AliasError { .. }
            | Error::Budget {
                breach: BudgetBreach::Aliases { .. } | BudgetBreach::Anchors { .. },
                ..
            } => (
                "morphir::ir::yaml::alias_not_allowed",
                Stage::Syntax,
                "expand anchors and aliases into explicit YAML nodes",
            ),
            Error::TaggedScalarCannotDeserializeIntoString { .. }
            | Error::TaggedEnumMismatch { .. } => (
                "morphir::ir::yaml::unsupported_tag",
                Stage::Syntax,
                "replace YAML semantic tags with the explicit structural vocabulary",
            ),
            Error::QuotingRequired { .. } | Error::InvalidBooleanStrict { .. } => (
                "morphir::ir::yaml::ambiguous_scalar",
                Stage::Normalization,
                "quote the scalar or use the explicit structural vocabulary",
            ),
            Error::Budget { .. } | Error::IOError { .. } => (
                "morphir::ir::yaml::budget_exceeded",
                Stage::Syntax,
                "simplify the YAML artifact or use a document-tree layout",
            ),
            _ => (
                "morphir::ir::yaml::invalid_ir",
                Stage::Normalization,
                "correct the YAML structure for the selected concrete IR version",
            ),
        };
        let diagnostic = TransportDiagnostic::error(
            code,
            stage,
            IrCursor::root(),
            error.without_snippet().to_string(),
        )
        .with_guidance(guidance);
        match source_span {
            Some(span) => diagnostic.with_source_span(span),
            None => diagnostic,
        }
    }

    fn encode_error(error: impl std::fmt::Display) -> TransportDiagnostic {
        TransportDiagnostic::error(
            "morphir::ir::yaml::encode_failed",
            Stage::Encoding,
            IrCursor::root(),
            error.to_string(),
        )
        .with_guidance("verify that the semantic event stream contains representable IR nodes")
    }

    fn write_yaml(
        writer: &mut dyn Write,
        value: &impl serde::Serialize,
    ) -> Result<(), TransportDiagnostic> {
        let mut rendered = serde_saphyr::to_string_with_options(value, Self::serializer_options())
            .map_err(Self::encode_error)?;
        rendered = rendered.replace("\r\n", "\n");
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        writer
            .write_all(rendered.as_bytes())
            .map_err(Self::encode_error)
    }
}

impl Default for YamlCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl IrCodec for YamlCodec {
    fn format(&self) -> &FormatId {
        &self.format
    }

    fn decode(
        &self,
        reader: &mut dyn Read,
        options: &CodecOptions,
        sink: &mut dyn EventSink,
    ) -> Result<(), TransportDiagnostic> {
        let input = Self::read_input(reader)?;
        validate_yaml_profile(&input)?;
        stacker::grow(IR_RECURSION_STACK_BYTES, || match options.version() {
            IrVersion::V3 => {
                let file: classic::Distribution =
                    serde_saphyr::from_slice_with_options(&input, Self::parse_options())
                        .map_err(Self::decode_error)?;
                semantic::emit_classic_v3(file, sink)
            }
            IrVersion::V4 => {
                let file: v4::IRFile =
                    serde_saphyr::from_slice_with_options(&input, Self::parse_options())
                        .map_err(Self::decode_error)?;
                semantic::emit_v4(file, sink)
            }
        })
    }

    fn encoder<'writer>(
        &self,
        writer: &'writer mut dyn Write,
        options: &CodecOptions,
    ) -> Result<Box<dyn EventSink + 'writer>, TransportDiagnostic> {
        if options.version() != IrVersion::V4 {
            return Err(TransportDiagnostic::error(
                "morphir::ir::yaml::streaming_v3_encoder_unsupported",
                Stage::Encoding,
                IrCursor::root(),
                "the push-based YAML encoder currently targets concrete IR v4",
            )
            .with_guidance("use the pull-based v3 encoder or select v4 output"));
        }
        Ok(Box::new(V4YamlEventEncoder::new(writer)))
    }

    fn encode(
        &self,
        source: &mut dyn EventSource,
        writer: &mut dyn Write,
        options: &CodecOptions,
    ) -> Result<(), TransportDiagnostic> {
        match semantic::collect(source, options.version())? {
            SemanticFile::ClassicV3(file) => Self::write_yaml(writer, &file),
            SemanticFile::V4(file) => Self::write_yaml(writer, &file),
        }
    }
}

enum V4YamlDistribution {
    Library,
    Specs,
    Application(v4::EntryPoints),
}

struct V4YamlEventEncoder<'writer> {
    writer: &'writer mut dyn Write,
    distribution: Option<V4YamlDistribution>,
    dependencies_started: bool,
    modules_started: bool,
    dependency_names: HashSet<String>,
    module_names: HashSet<String>,
    ended: bool,
}

impl<'writer> V4YamlEventEncoder<'writer> {
    fn new(writer: &'writer mut dyn Write) -> Self {
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

fn stream_event_error(
    suffix: &'static str,
    cursor: &IrCursor,
    message: &'static str,
) -> TransportDiagnostic {
    TransportDiagnostic::error(
        format!("morphir::ir::yaml::{suffix}"),
        Stage::Encoding,
        cursor.clone(),
        message,
    )
    .with_guidance("verify the semantic event order and selected concrete IR version")
}

fn validate_yaml_profile(input: &[u8]) -> Result<(), TransportDiagnostic> {
    let source = std::str::from_utf8(input).map_err(|error| {
        TransportDiagnostic::error(
            "morphir::ir::yaml::invalid_utf8",
            Stage::Syntax,
            IrCursor::root(),
            error.to_string(),
        )
        .with_guidance("encode the YAML artifact as UTF-8")
    })?;
    let mut saw_content = false;
    let mut block_scalar_parent_indent = None;
    let mut offset = 0;
    for (line_index, line) in source.split_inclusive('\n').enumerate() {
        let line_without_ending = line.trim_end_matches(['\r', '\n']);
        let trimmed = line_without_ending.trim_start();
        let indent = line_without_ending.len() - trimmed.len();
        let column = indent + 1;
        if trimmed.is_empty() {
            offset += line.len();
            continue;
        }
        if block_scalar_parent_indent.is_some_and(|parent| indent > parent) {
            offset += line.len();
            continue;
        }
        block_scalar_parent_indent = None;
        if trimmed.starts_with('#') {
            offset += line.len();
            continue;
        }
        if trimmed == "---" {
            if saw_content {
                return Err(profile_diagnostic(
                    "morphir::ir::yaml::multiple_documents",
                    Stage::Syntax,
                    "a YAML IR artifact contains more than one document",
                    "store exactly one IR document in each YAML artifact",
                    offset + column - 1,
                    3,
                    line_index + 1,
                    column,
                ));
            }
            offset += line.len();
            continue;
        }
        saw_content = true;
        if let Some((token_column, code, stage, message, guidance)) = forbidden_token(trimmed) {
            return Err(profile_diagnostic(
                code,
                stage,
                message,
                guidance,
                offset + column - 1 + token_column,
                1,
                line_index + 1,
                column + token_column,
            ));
        }
        if is_block_scalar_header(trimmed) {
            block_scalar_parent_indent = Some(indent);
        }
        offset += line.len();
    }
    Ok(())
}

pub(crate) fn decode_json_value(input: &[u8]) -> Result<serde_json::Value, TransportDiagnostic> {
    validate_yaml_profile(input)?;
    serde_saphyr::from_slice_with_options(input, YamlCodec::parse_options())
        .map_err(YamlCodec::decode_error)
}

fn is_block_scalar_header(line: &str) -> bool {
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    let mut comment_start = line.len();
    for (index, character) in line.char_indices() {
        if double_quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                double_quoted = false;
            }
            continue;
        }
        if single_quoted {
            if character == '\'' {
                single_quoted = false;
            }
            continue;
        }
        match character {
            '\'' => single_quoted = true,
            '"' => double_quoted = true,
            '#' if token_boundary(line[..index].chars().next_back()) => {
                comment_start = index;
                break;
            }
            _ => {}
        }
    }
    let token = line[..comment_start]
        .split_whitespace()
        .next_back()
        .unwrap_or_default();
    let mut characters = token.chars();
    matches!(characters.next(), Some('|' | '>'))
        && characters.all(|character| matches!(character, '+' | '-' | '1'..='9'))
}

fn forbidden_token(line: &str) -> Option<(usize, &'static str, Stage, &'static str, &'static str)> {
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    let characters = line.char_indices().collect::<Vec<_>>();
    for (position, (index, character)) in characters.iter().copied().enumerate() {
        if double_quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                double_quoted = false;
            }
            continue;
        }
        if single_quoted {
            if character == '\'' {
                if characters
                    .get(position + 1)
                    .is_some_and(|(_, next)| *next == '\'')
                {
                    continue;
                }
                single_quoted = false;
            }
            continue;
        }
        match character {
            '\'' => single_quoted = true,
            '"' => double_quoted = true,
            '#' if token_boundary(line[..index].chars().next_back()) => break,
            '&' | '*' if token_boundary(line[..index].chars().next_back()) => {
                return Some((
                    index,
                    "morphir::ir::yaml::alias_not_allowed",
                    Stage::Syntax,
                    "YAML anchors and aliases are disabled by the native IR profile",
                    "expand anchors and aliases into explicit YAML nodes",
                ));
            }
            '!' if token_boundary(line[..index].chars().next_back()) => {
                return Some((
                    index,
                    "morphir::ir::yaml::unsupported_tag",
                    Stage::Syntax,
                    "YAML semantic tags are not part of the native IR vocabulary",
                    "replace the tag with an explicit structural mapping",
                ));
            }
            '<' if line[index..].starts_with("<<:")
                && token_boundary(line[..index].chars().next_back()) =>
            {
                return Some((
                    index,
                    "morphir::ir::yaml::merge_key_not_allowed",
                    Stage::Syntax,
                    "YAML merge keys are disabled by the native IR profile",
                    "expand the merge as ordinary mapping entries",
                ));
            }
            _ => {}
        }
    }

    for token in line.split(|character: char| {
        character.is_whitespace() || matches!(character, ':' | ',' | '[' | ']' | '{' | '}')
    }) {
        let lowercase = token.to_ascii_lowercase();
        if matches!(lowercase.as_str(), ".inf" | "+.inf" | "-.inf" | ".nan") {
            let index = line.find(token).unwrap_or_default();
            return Some((
                index,
                "morphir::ir::yaml::non_finite_number",
                Stage::Normalization,
                "non-finite YAML numbers cannot represent concrete IR literals losslessly",
                "use a finite number representable by the concrete IR literal",
            ));
        }
        if looks_like_timestamp(token) {
            let index = line.find(token).unwrap_or_default();
            return Some((
                index,
                "morphir::ir::yaml::ambiguous_scalar",
                Stage::Normalization,
                "an unquoted timestamp-like scalar is ambiguous in the native IR profile",
                "quote the value to store it as a string",
            ));
        }
    }
    None
}

fn token_boundary(character: Option<char>) -> bool {
    character.is_none_or(|character| {
        character.is_whitespace() || matches!(character, ':' | ',' | '[' | ']' | '{' | '}' | '-')
    })
}

fn looks_like_timestamp(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() == 10
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

#[allow(clippy::too_many_arguments)]
fn profile_diagnostic(
    code: &'static str,
    stage: Stage,
    message: &'static str,
    guidance: &'static str,
    offset: usize,
    length: usize,
    line: usize,
    column: usize,
) -> TransportDiagnostic {
    TransportDiagnostic::error(code, stage, IrCursor::root(), message)
        .with_guidance(guidance)
        .with_source_span(SourceSpan {
            offset,
            length,
            line,
            column,
        })
}
