//! Strict native YAML codec for concrete Morphir IR.

use std::io::{Read, Write};

use morphir_core::format_version::SupportTable;
use morphir_core::ir::{classic, v4 as ir_v4};
use morphir_core::traversal::IrCursor;
use serde_saphyr::budget::BudgetBreach;
use serde_saphyr::options::{DuplicateKeyPolicy, MergeKeyPolicy};
use serde_saphyr::{Error, alias_limits, budget, options, ser_options};

use super::semantic::{self, SemanticFile};
use super::{
    CodecOptions, EventSink, EventSource, FormatId, IR_RECURSION_STACK_BYTES, IrCodec, IrVersion,
    SourceSpan, Stage, TransportDiagnostic,
};

mod profile;
mod v3;
mod v4;

use super::root_probe::probe_yaml_slice;
pub(crate) use profile::{
    decode_document, decode_json_value, encode_document, validate_yaml_profile,
};
use v3::V3YamlEventEncoder;
use v4::V4YamlEventEncoder;

pub(crate) const MAX_INPUT_BYTES: usize = 512 * 1024 * 1024;

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

    pub(super) fn parse_options() -> serde_saphyr::Options {
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

    pub(super) fn serializer_options() -> serde_saphyr::SerializerOptions {
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

    pub(super) fn decode_error(error: Error) -> TransportDiagnostic {
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

    pub(super) fn encode_error(error: impl std::fmt::Display) -> TransportDiagnostic {
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
        let rendered = encode_document(value)?;
        writer.write_all(&rendered).map_err(Self::encode_error)
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
        let probe = probe_yaml_slice(&input, &SupportTable::reference())?;
        stacker::grow(IR_RECURSION_STACK_BYTES, || match options.version() {
            IrVersion::V3 => {
                if probe.normalized.release.major() != 3 {
                    return Err(TransportDiagnostic::error(
                        "morphir::ir::yaml::version_mismatch",
                        Stage::Detection,
                        IrCursor::root(),
                        format!(
                            "the v3 YAML codec requires formatVersion 3, found {}",
                            probe.normalized.release
                        ),
                    ));
                }
                let file: classic::Distribution =
                    serde_saphyr::from_slice_with_options(&input, Self::parse_options())
                        .map_err(Self::decode_error)?;
                semantic::emit_classic_v3(file, sink)
            }
            IrVersion::V4 => {
                if probe.normalized.release.major() != 4 {
                    return Err(TransportDiagnostic::error(
                        "morphir::ir::yaml::version_mismatch",
                        Stage::Detection,
                        IrCursor::root(),
                        format!(
                            "the v4 YAML codec requires formatVersion 4, found {}",
                            probe.normalized.release
                        ),
                    ));
                }
                let file: ir_v4::IRFile =
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
        match options.version() {
            IrVersion::V3 => Ok(Box::new(V3YamlEventEncoder::new(writer))),
            IrVersion::V4 => Ok(Box::new(V4YamlEventEncoder::new(writer))),
        }
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
