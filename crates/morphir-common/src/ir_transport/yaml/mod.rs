//! The IR YAML profile codec.
//!
//! Both directions go through the kit's own reader and canonical writer in
//! `morphir_core::ir::yaml`, so the CLI answers a YAML document exactly as the MCK adapter does:
//! one value tree, one set of diagnostic codes, one canonical spelling. What used to live here —
//! a lexical pre-scan, a `serde-saphyr` round trip, `PlainValue`'s number rewrite and two
//! hand-written streaming encoders — is gone; see `docs/spec/ir/schemas/v4/yaml-profile.md`.

use std::collections::VecDeque;
use std::io::{Read, Write};

use morphir_core::format_version::{
    FormatVersionDiagnostic, NormalizedFormatVersion, ScalarValue, SupportTable,
};
use morphir_core::ir::v4::{TypeEncoding, with_type_encoding};
use morphir_core::ir::yaml as profile;
use morphir_core::ir::{Diagnostic as CoreDiagnostic, DiagnosticCode, classic, v4 as ir_v4};
use morphir_core::traversal::{IrCursor, SemanticEvent};
use serde::Serialize;
use serde_json::Value as Json;

use super::diagnostic::{core_code_name, core_message, core_source_span, core_stage};
use super::semantic::{self, SemanticFile};
use super::{
    CodecOptions, EventSink, EventSource, FormatId, HeaderObservation, IR_RECURSION_STACK_BYTES,
    IrCodec, IrVersion, Stage, TransportDiagnostic,
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

    pub(super) fn encode_error(error: impl std::fmt::Display) -> TransportDiagnostic {
        TransportDiagnostic::error(
            "morphir::ir::yaml::encode_failed",
            Stage::Encoding,
            IrCursor::root(),
            error.to_string(),
        )
        .with_guidance("verify that the semantic event stream contains representable IR nodes")
    }
}

impl Default for YamlCodec {
    fn default() -> Self {
        Self::new()
    }
}

/// Reads `input` as one profile-conforming YAML document.
pub(crate) fn read_value(input: &[u8]) -> Result<Json, TransportDiagnostic> {
    let text = std::str::from_utf8(input).map_err(|error| {
        TransportDiagnostic::error(
            "morphir::ir::yaml::invalid_utf8",
            Stage::Syntax,
            IrCursor::root(),
            error.to_string(),
        )
        .with_guidance("encode the YAML artifact as UTF-8")
    })?;
    stacker::grow(IR_RECURSION_STACK_BYTES, || {
        profile::read(text).map_err(transport_diagnostic)
    })
}

/// The non-fatal header observations a root mapping carries, as the JSON root probe reports them.
///
/// The profile reader preserves member order, so a `formatVersion` that does not come first is as
/// answerable here as it is on the JSON path, and the shared conformance corpus pins the same
/// observation for both profiles. Replay measurements are a JSON-probe concept — the YAML reader
/// has already read the whole document — so an observation from here carries none.
pub(crate) fn header_observations(value: &Json) -> Vec<HeaderObservation> {
    let Some(members) = value.as_object() else {
        return Vec::new();
    };
    let first_is_format_version = members
        .keys()
        .next()
        .is_some_and(|first| first == "formatVersion");
    if members.contains_key("formatVersion") && !first_is_format_version {
        return vec![HeaderObservation {
            code: "format_version_not_first",
            message: "formatVersion is valid but does not appear first in the root mapping".into(),
            replay: None,
        }];
    }
    Vec::new()
}

/// Reads one profile-conforming YAML document and reports its header observations.
///
/// This is the YAML counterpart of [`probe_json_root`](super::probe_json_root)'s `observations`:
/// `IrCodec::decode` has no channel for a non-fatal observation, so a caller that wants them asks
/// for them here.
pub fn probe_yaml_header(input: &[u8]) -> Result<Vec<HeaderObservation>, TransportDiagnostic> {
    Ok(header_observations(&read_value(input)?))
}

/// Wraps one of the kit's diagnostics as a transport diagnostic.
///
/// The code is the kit's own, under `morphir::ir::yaml::`, so the CLI and the adapter name the
/// same fault the same way. A [`TransportDiagnostic`]'s cursor is a semantic [`IrCursor`] and the
/// kit's is a JSON pointer into the document, which has no semantic spelling before the document
/// is understood; the pointer therefore travels in the message and the cursor stays at the root,
/// as every other physical-syntax diagnostic in this crate does.
pub(crate) fn transport_diagnostic(diagnostic: CoreDiagnostic) -> TransportDiagnostic {
    let transport = TransportDiagnostic::error(
        format!("morphir::ir::yaml::{}", core_code_name(diagnostic.code)),
        core_stage(diagnostic.stage),
        IrCursor::root(),
        core_message(&diagnostic),
    )
    .with_guidance(guidance_for(diagnostic.code));
    match core_source_span(&diagnostic) {
        Some(span) => transport.with_source_span(span),
        None => transport,
    }
}

fn guidance_for(code: DiagnosticCode) -> &'static str {
    match code {
        DiagnosticCode::InvalidYaml => "write exactly one YAML 1.2 document",
        DiagnosticCode::UnsupportedYamlFeature => {
            "remove anchors, aliases, tags, directives and merge keys; the profile forbids them"
        }
        DiagnosticCode::DuplicateMember => "each member appears once",
        DiagnosticCode::InvalidType => "mapping keys must be strings",
        DiagnosticCode::InvalidLiteral => {
            "write decimal numbers without leading zeros; non-finite values are not representable"
        }
        DiagnosticCode::NestingTooDeep => "the document nests deeper than 1000 levels",
        _ => "correct the document for the selected concrete IR version",
    }
}

/// The diagnostic a serde failure carried, or an `invalid_type` naming what serde said.
///
/// Every v4 decoder in morphir-core smuggles one of the kit's diagnostics through the serde
/// error; the fallback is for the models still read by a derived impl (classic v3, and the
/// document-tree manifests).
fn recover(error: &serde_json::Error) -> TransportDiagnostic {
    match CoreDiagnostic::from_serde_error(error) {
        Some(diagnostic) => transport_diagnostic(diagnostic),
        None => TransportDiagnostic::error(
            "morphir::ir::yaml::invalid_type",
            Stage::Normalization,
            IrCursor::root(),
            error.to_string(),
        )
        .with_guidance("correct the document for the selected concrete IR version"),
    }
}

fn format_version_error(error: FormatVersionDiagnostic) -> TransportDiagnostic {
    // The same bare codes the JSON codec answers through the root probe, so a caller comparing
    // format-version outcomes does not have to know which profile the document was written in.
    TransportDiagnostic::error(
        error.code(),
        Stage::Detection,
        IrCursor::root(),
        error.message(),
    )
}

/// The `formatVersion` member of a root mapping, normalized and checked for support.
fn format_version_of(
    value: &Json,
    support: &SupportTable,
) -> Result<NormalizedFormatVersion, TransportDiagnostic> {
    let written = value
        .as_object()
        .and_then(|members| members.get("formatVersion"))
        .ok_or_else(|| format_version_error(FormatVersionDiagnostic::missing_format_version()))?;
    let scalar = ScalarValue::from_json(written).map_err(format_version_error)?;
    let normalized =
        NormalizedFormatVersion::from_scalar(&scalar, support).map_err(format_version_error)?;
    if !normalized.is_supported() {
        return Err(format_version_error(
            support
                .unsupported_diagnostic(&normalized.release, normalized.compatibility)
                .expect("unsupported releases produce diagnostics"),
        ));
    }
    Ok(normalized)
}

fn version_mismatch(expected: u32, found: &impl std::fmt::Display) -> TransportDiagnostic {
    TransportDiagnostic::error(
        "morphir::ir::yaml::version_mismatch",
        Stage::Detection,
        IrCursor::root(),
        format!("the v{expected} YAML codec requires formatVersion {expected}, found {found}"),
    )
}

fn encode_text<T: Serialize + ?Sized>(value: &T) -> Result<String, TransportDiagnostic> {
    stacker::grow(IR_RECURSION_STACK_BYTES, || {
        // `TypeEncoding::Compact` is the canonical spelling of a v4 type expression: a reference
        // with no arguments and no attributes is `morphir/SDK:basics#int` rather than an expanded
        // wrapper. The thread-local defaults to `Expanded`, so the canonical writer selects it,
        // as the MCK adapter's canonical path does. A classic v3 file consults it nowhere.
        let tree = with_type_encoding(TypeEncoding::Compact, || serde_json::to_value(value))
            .map_err(YamlCodec::encode_error)?;
        // `write_canonical` ends its output with exactly one `\n` and never writes a CR.
        Ok(profile::write_canonical(&tree))
    })
}

fn write_semantic_file(
    file: SemanticFile,
    writer: &mut dyn Write,
) -> Result<(), TransportDiagnostic> {
    let rendered = match file {
        SemanticFile::ClassicV3(file) => encode_text(&file)?,
        SemanticFile::V4(file) => encode_text(&file)?,
    };
    writer
        .write_all(rendered.as_bytes())
        .map_err(YamlCodec::encode_error)?;
    writer.flush().map_err(YamlCodec::encode_error)
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
        let value = read_value(&input)?;
        // The same header observations the JSON codec's root probe raises, raised on the same
        // document. Neither codec has anywhere to send a non-fatal observation from `decode` —
        // the JSON codec drops `probe.observations` here too — so a caller that wants them calls
        // `probe_yaml_header`; what matters is that the YAML path can still answer them.
        let _observations = header_observations(&value);
        let normalized = format_version_of(&value, &SupportTable::reference())?;
        stacker::grow(IR_RECURSION_STACK_BYTES, || match options.version() {
            IrVersion::V3 => {
                if normalized.release.major() != 3 {
                    return Err(version_mismatch(3, &normalized.release));
                }
                let file: classic::Distribution =
                    serde_json::from_value(value).map_err(|error| recover(&error))?;
                semantic::emit_classic_v3(file, sink)
            }
            IrVersion::V4 => {
                if normalized.release.major() != 4 {
                    return Err(version_mismatch(4, &normalized.release));
                }
                let file: ir_v4::IRFile =
                    serde_json::from_value(value).map_err(|error| recover(&error))?;
                semantic::emit_v4(file, sink)
            }
        })
    }

    fn encoder<'writer>(
        &self,
        writer: &'writer mut dyn Write,
        options: &CodecOptions,
    ) -> Result<Box<dyn EventSink + 'writer>, TransportDiagnostic> {
        Ok(Box::new(YamlEventEncoder::new(writer, options.version())))
    }

    fn encode(
        &self,
        source: &mut dyn EventSource,
        writer: &mut dyn Write,
        options: &CodecOptions,
    ) -> Result<(), TransportDiagnostic> {
        write_semantic_file(semantic::collect(source, options.version())?, writer)
    }
}

/// The push-side view of [`YamlCodec::encode`].
///
/// The canonical writer needs the whole value tree at once — it decides a sequence's style from
/// what is nested inside it — so the events are held until `finish`, which replays them through
/// the same collector `encode` uses. Buffering is what the canonical spelling costs; the JSON
/// canonical writer in the adapter pays it too. Event-order faults are therefore the collector's
/// (`morphir::ir::codec::missing_begin` and its neighbours), not a second set of rules here.
struct YamlEventEncoder<'writer> {
    writer: &'writer mut dyn Write,
    version: IrVersion,
    events: VecDeque<SemanticEvent>,
    finished: bool,
}

impl<'writer> YamlEventEncoder<'writer> {
    fn new(writer: &'writer mut dyn Write, version: IrVersion) -> Self {
        Self {
            writer,
            version,
            events: VecDeque::new(),
            finished: false,
        }
    }
}

/// Replays a buffered event stream for [`semantic::collect`].
struct BufferedSource(VecDeque<SemanticEvent>);

impl EventSource for BufferedSource {
    fn next_event(&mut self) -> Result<Option<SemanticEvent>, TransportDiagnostic> {
        Ok(self.0.pop_front())
    }
}

impl EventSink for YamlEventEncoder<'_> {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        if self.finished {
            return Err(TransportDiagnostic::error(
                "morphir::ir::codec::event_after_end",
                Stage::Encoding,
                event.cursor().clone(),
                "an event appeared after the YAML document was written",
            )
            .with_guidance("create a new encoder for each codec operation"));
        }
        self.events.push_back(event);
        Ok(())
    }

    fn finish(&mut self) -> Result<(), TransportDiagnostic> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        let mut source = BufferedSource(std::mem::take(&mut self.events));
        let file = semantic::collect(&mut source, self.version)?;
        write_semantic_file(file, self.writer)
    }
}
