//! Syntax-aware JSON root probing with replayable transport.

use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use morphir_core::format_version::{
    FormatVersionDiagnostic, NormalizedFormatVersion, ScalarValue, SupportTable,
};
use tempfile::NamedTempFile;

use super::{SourceSpan, Stage, TransportDiagnostic};

const REPLAY_MEMORY_THRESHOLD: usize = 64 * 1024;

/// How a reader replays a late `formatVersion` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayKind {
    /// No replay was required because `formatVersion` appeared first.
    None,
    /// The document was replayed from an in-memory buffer.
    Memory,
    /// The document was replayed from temporary storage.
    TemporaryStorage,
}

/// Measured replay cost for lint observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayObservation {
    /// Byte offset where `formatVersion` begins.
    pub format_version_offset: usize,
    /// Total bytes scanned before semantic decoding begins.
    pub bytes_scanned: usize,
    /// Replay strategy selected by the probe.
    pub replay_kind: ReplayKind,
}

/// Non-fatal header observation emitted by the root probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderObservation {
    /// Stable warning code.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
    /// Replay measurements when available.
    pub replay: Option<ReplayObservation>,
}

/// Result of probing one JSON root mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonRootProbe {
    /// Normalized and compatibility-checked format version.
    pub normalized: NormalizedFormatVersion,
    /// Root member order discovered by the probe.
    pub member_order: Vec<String>,
    /// Non-fatal observations such as noncanonical member order.
    pub observations: Vec<HeaderObservation>,
    /// Replay measurements when a late `formatVersion` forced replay.
    pub replay: ReplayObservation,
}

/// Probed JSON input that preserves streaming when `formatVersion` appears first.
#[derive(Debug)]
pub enum ProbedJsonReader<'reader, R: Read + ?Sized + 'reader> {
    /// Prefix bytes already read chained with the remaining reader.
    Stream(PrefixedReader<'reader, R>),
    /// Replay from an in-memory buffer.
    Memory(Cursor<Vec<u8>>),
    /// Replay from a temporary file.
    Temporary(NamedTempFile),
}

/// Chains bytes already read during probing with the remaining reader.
#[derive(Debug)]
pub struct PrefixedReader<'reader, R: Read + ?Sized + 'reader> {
    prefix: Cursor<Vec<u8>>,
    reader: &'reader mut R,
}

impl<R: Read + ?Sized> Read for PrefixedReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let from_prefix = self.prefix.read(buf)?;
        if from_prefix > 0 {
            return Ok(from_prefix);
        }
        self.reader.read(buf)
    }
}

/// Probe one JSON document root and prepare replay when required.
pub fn probe_json_root<'reader, R: Read + ?Sized + 'reader>(
    reader: &'reader mut R,
    support: &SupportTable,
) -> Result<(JsonRootProbe, ProbedJsonReader<'reader, R>), TransportDiagnostic> {
    let mut buffer = Vec::new();
    let mut spill = None;
    let mut stream_after_header = false;
    let mut streaming_header = None;

    loop {
        let source = materialize_probe_source(&buffer, &mut spill)?;
        let scanner = JsonRootScanner::new(&source);
        match scanner.scan_incremental() {
            Ok(IncrementalScan::StreamingReady(header)) => {
                stream_after_header = true;
                streaming_header = Some(header);
                break;
            }
            Ok(IncrementalScan::LateHeaderPending) => break,
            Ok(IncrementalScan::Incomplete) => {}
            Err(error) => return Err(error),
        }
        if !read_probe_chunk(reader, &mut buffer, &mut spill)? {
            break;
        }
    }

    if !stream_after_header {
        while read_probe_chunk(reader, &mut buffer, &mut spill)? {}
    }

    let source = materialize_probe_source(&buffer, &mut spill)?;
    let probe = if JsonRootScanner::new(&source).scan_partial()?.is_some() {
        finish_probe(JsonRootScanner::new(&source), &source, support)?
    } else if let Some(header) = streaming_header {
        finalize_streaming_probe(header, &source, support)?
    } else {
        finish_probe(JsonRootScanner::new(&source), &source, support)?
    };
    let input = if probe.replay.replay_kind == ReplayKind::None {
        let mut prefix = buffer;
        if let Some(mut temp) = spill {
            temp.seek(SeekFrom::Start(0)).map_err(probe_io_error)?;
            temp.read_to_end(&mut prefix).map_err(probe_io_error)?;
        }
        ProbedJsonReader::Stream(PrefixedReader {
            prefix: Cursor::new(prefix),
            reader,
        })
    } else if spill.is_some() {
        let mut temp = spill.take().expect("spilled replay source");
        temp.seek(SeekFrom::Start(0)).map_err(probe_io_error)?;
        ProbedJsonReader::Temporary(temp)
    } else if source.len() <= REPLAY_MEMORY_THRESHOLD {
        ProbedJsonReader::Memory(Cursor::new(source))
    } else {
        let mut temp = NamedTempFile::new().map_err(probe_io_error)?;
        temp.write_all(&source).map_err(probe_io_error)?;
        temp.seek(SeekFrom::Start(0)).map_err(probe_io_error)?;
        ProbedJsonReader::Temporary(temp)
    };
    Ok((probe, input))
}

/// Probe one in-memory JSON document root.
pub fn probe_json_slice(
    source: &[u8],
    support: &SupportTable,
) -> Result<JsonRootProbe, TransportDiagnostic> {
    finish_probe(JsonRootScanner::new(source), source, support)
}

/// Probe one in-memory YAML document root.
pub fn probe_yaml_slice(
    source: &[u8],
    support: &SupportTable,
) -> Result<JsonRootProbe, TransportDiagnostic> {
    let text = std::str::from_utf8(source).map_err(probe_io_error)?;
    let header = if text.trim_start().starts_with('{') {
        let value = super::yaml::decode_json_value(source)?;
        yaml_root_header_from_json(&value, source.len())?
    } else {
        YamlRootScanner::new(text).scan()?
    };
    finish_probe_from_header(header, source, support)
}

fn read_probe_chunk<R: Read + ?Sized>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
    spill: &mut Option<NamedTempFile>,
) -> Result<bool, TransportDiagnostic> {
    let mut chunk = [0_u8; 4096];
    let read = reader.read(&mut chunk).map_err(probe_io_error)?;
    if read == 0 {
        return Ok(false);
    }
    append_probe_bytes(buffer, spill, &chunk[..read])?;
    Ok(true)
}

fn append_probe_bytes(
    buffer: &mut Vec<u8>,
    spill: &mut Option<NamedTempFile>,
    bytes: &[u8],
) -> Result<(), TransportDiagnostic> {
    if let Some(temp) = spill {
        temp.write_all(bytes).map_err(probe_io_error)?;
        return Ok(());
    }
    if buffer.len() + bytes.len() > REPLAY_MEMORY_THRESHOLD {
        let mut temp = NamedTempFile::new().map_err(probe_io_error)?;
        temp.write_all(buffer).map_err(probe_io_error)?;
        temp.write_all(bytes).map_err(probe_io_error)?;
        buffer.clear();
        *spill = Some(temp);
        return Ok(());
    }
    buffer.extend_from_slice(bytes);
    Ok(())
}

fn materialize_probe_source(
    buffer: &[u8],
    spill: &mut Option<NamedTempFile>,
) -> Result<Vec<u8>, TransportDiagnostic> {
    if let Some(temp) = spill {
        temp.seek(SeekFrom::Start(0)).map_err(probe_io_error)?;
        let mut source = buffer.to_vec();
        temp.read_to_end(&mut source).map_err(probe_io_error)?;
        temp.seek(SeekFrom::Start(0)).map_err(probe_io_error)?;
        Ok(source)
    } else {
        Ok(buffer.to_vec())
    }
}

fn yaml_root_header_from_json(
    value: &serde_json::Value,
    source_len: usize,
) -> Result<JsonRootHeader, TransportDiagnostic> {
    let mapping = value
        .as_object()
        .ok_or_else(|| probe_syntax_error("invalid YAML syntax"))?;

    let mut member_order = Vec::new();
    let mut format_version_count = 0_usize;
    let mut scalar = None;
    let mut saw_distribution = false;

    for (key, value) in mapping {
        member_order.push(key.clone());
        if key == "formatVersion" {
            format_version_count += 1;
            if format_version_count > 1 {
                return Err(transport_from_format_version(
                    FormatVersionDiagnostic::duplicate_format_version(),
                ));
            }
            scalar = Some(ScalarValue::from_json(value).map_err(transport_from_format_version)?);
        } else if key == "distribution" {
            saw_distribution = true;
        }
    }

    if format_version_count == 0 {
        return Err(transport_from_format_version(
            FormatVersionDiagnostic::missing_format_version(),
        ));
    }

    let replay_kind = if member_order
        .first()
        .is_some_and(|member| member == "formatVersion")
    {
        ReplayKind::None
    } else if saw_distribution {
        if source_len <= REPLAY_MEMORY_THRESHOLD {
            ReplayKind::Memory
        } else {
            ReplayKind::TemporaryStorage
        }
    } else {
        ReplayKind::None
    };

    Ok(JsonRootHeader {
        scalar,
        member_order,
        format_version_offset: 0,
        replay_kind,
        saw_distribution,
        format_version_complete: true,
    })
}

fn finalize_streaming_probe(
    header: JsonRootHeader,
    source: &[u8],
    support: &SupportTable,
) -> Result<JsonRootProbe, TransportDiagnostic> {
    let scalar = header.scalar.ok_or_else(|| {
        transport_from_format_version(FormatVersionDiagnostic::missing_format_version())
    })?;
    let normalized = NormalizedFormatVersion::from_scalar(&scalar, support)
        .map_err(transport_from_format_version)?;
    if !normalized.is_supported() {
        return Err(transport_from_format_version(
            support
                .unsupported_diagnostic(&normalized.release, normalized.compatibility)
                .expect("unsupported releases produce diagnostics"),
        ));
    }

    Ok(JsonRootProbe {
        normalized,
        member_order: header.member_order,
        observations: Vec::new(),
        replay: ReplayObservation {
            format_version_offset: header.format_version_offset,
            bytes_scanned: source.len(),
            replay_kind: ReplayKind::None,
        },
    })
}

fn finish_probe(
    scanner: JsonRootScanner<'_>,
    source: &[u8],
    support: &SupportTable,
) -> Result<JsonRootProbe, TransportDiagnostic> {
    finish_probe_from_header(scanner.scan()?, source, support)
}

fn finish_probe_from_header(
    header: JsonRootHeader,
    source: &[u8],
    support: &SupportTable,
) -> Result<JsonRootProbe, TransportDiagnostic> {
    let scalar = header.scalar.ok_or_else(|| {
        transport_from_format_version(FormatVersionDiagnostic::missing_format_version())
    })?;
    let normalized = NormalizedFormatVersion::from_scalar(&scalar, support)
        .map_err(transport_from_format_version)?;
    if !normalized.is_supported() {
        return Err(transport_from_format_version(
            support
                .unsupported_diagnostic(&normalized.release, normalized.compatibility)
                .expect("unsupported releases produce diagnostics"),
        ));
    }

    let mut observations = Vec::new();
    if header
        .member_order
        .first()
        .is_some_and(|member| member != "formatVersion")
    {
        observations.push(HeaderObservation {
            code: "format_version_not_first",
            message: "formatVersion is valid but does not appear first in the root mapping".into(),
            replay: Some(ReplayObservation {
                format_version_offset: header.format_version_offset,
                bytes_scanned: source.len(),
                replay_kind: header.replay_kind,
            }),
        });
    }

    Ok(JsonRootProbe {
        normalized,
        member_order: header.member_order,
        observations,
        replay: ReplayObservation {
            format_version_offset: header.format_version_offset,
            bytes_scanned: source.len(),
            replay_kind: header.replay_kind,
        },
    })
}

struct JsonRootHeader {
    scalar: Option<ScalarValue>,
    member_order: Vec<String>,
    format_version_offset: usize,
    replay_kind: ReplayKind,
    saw_distribution: bool,
    format_version_complete: bool,
}

enum IncrementalScan {
    Incomplete,
    StreamingReady(JsonRootHeader),
    LateHeaderPending,
}

struct JsonRootScanner<'source> {
    source: &'source [u8],
}

impl<'source> JsonRootScanner<'source> {
    fn new(source: &'source [u8]) -> Self {
        Self { source }
    }

    fn scan(self) -> Result<JsonRootHeader, TransportDiagnostic> {
        let mut header = self
            .scan_partial()?
            .ok_or_else(|| probe_syntax_error("invalid JSON syntax"))?;
        if !header.format_version_complete {
            return Err(transport_from_format_version(
                FormatVersionDiagnostic::missing_format_version(),
            ));
        }
        if header
            .member_order
            .iter()
            .filter(|member| *member == "formatVersion")
            .count()
            > 1
        {
            return Err(transport_from_format_version(
                FormatVersionDiagnostic::duplicate_format_version(),
            ));
        }
        header.replay_kind = if header
            .member_order
            .first()
            .is_some_and(|member| member == "formatVersion")
        {
            ReplayKind::None
        } else if header.saw_distribution {
            if self.source.len() <= REPLAY_MEMORY_THRESHOLD {
                ReplayKind::Memory
            } else {
                ReplayKind::TemporaryStorage
            }
        } else {
            ReplayKind::None
        };
        Ok(header)
    }

    #[allow(unused_assignments)]
    fn scan_incremental(&self) -> Result<IncrementalScan, TransportDiagnostic> {
        let mut index = 0_usize;
        skip_ws(self.source, &mut index);
        if peek(self.source, index) != Some(b'{') {
            return Ok(IncrementalScan::Incomplete);
        }
        index += 1;
        skip_ws(self.source, &mut index);

        let mut member_order = Vec::new();
        let mut scalar = None::<ScalarValue>;
        let mut format_version_offset = 0_usize;
        let mut saw_distribution = false;
        let mut format_version_complete = false;
        let mut format_version_count = 0_usize;
        let mut first_member = true;

        loop {
            skip_ws(self.source, &mut index);
            if peek(self.source, index) == Some(b'}') {
                return Ok(IncrementalScan::Incomplete);
            }
            if peek(self.source, index).is_none() {
                return Ok(IncrementalScan::Incomplete);
            }
            if !first_member {
                if peek(self.source, index) != Some(b',') {
                    return Ok(IncrementalScan::Incomplete);
                }
                index += 1;
                skip_ws(self.source, &mut index);
            }
            first_member = false;

            let key = match read_string_partial(self.source, &mut index) {
                Ok(value) => value,
                Err(PartialScanError::NeedMore) => return Ok(IncrementalScan::Incomplete),
                Err(PartialScanError::Invalid(error)) => return Err(error),
            };
            member_order.push(key.clone());
            skip_ws(self.source, &mut index);
            if peek(self.source, index) != Some(b':') {
                return Ok(IncrementalScan::Incomplete);
            }
            index += 1;
            skip_ws(self.source, &mut index);

            if key == "formatVersion" {
                format_version_count += 1;
                if format_version_count > 1 {
                    return Err(transport_from_format_version(
                        FormatVersionDiagnostic::duplicate_format_version(),
                    ));
                }
                format_version_offset = index;
                match read_scalar_partial(self.source, &mut index) {
                    Ok(value) => {
                        scalar = Some(value);
                        format_version_complete = true;
                    }
                    Err(PartialScanError::NeedMore) => return Ok(IncrementalScan::Incomplete),
                    Err(PartialScanError::Invalid(error)) => return Err(error),
                }
                if member_order
                    .first()
                    .is_some_and(|member| member == "formatVersion")
                {
                    let mut lookahead = index;
                    skip_ws(self.source, &mut lookahead);
                    if peek(self.source, lookahead) == Some(b',') {
                        lookahead += 1;
                        skip_ws(self.source, &mut lookahead);
                        match read_string_partial(self.source, &mut lookahead) {
                            Ok(next_key) if next_key == "formatVersion" => {
                                return Err(transport_from_format_version(
                                    FormatVersionDiagnostic::duplicate_format_version(),
                                ));
                            }
                            Ok(_) | Err(PartialScanError::NeedMore) => {}
                            Err(PartialScanError::Invalid(error)) => return Err(error),
                        }
                    }
                    return Ok(IncrementalScan::StreamingReady(JsonRootHeader {
                        scalar,
                        member_order,
                        format_version_offset,
                        replay_kind: ReplayKind::None,
                        saw_distribution,
                        format_version_complete,
                    }));
                } else if saw_distribution {
                    return Ok(IncrementalScan::LateHeaderPending);
                }
            } else {
                if key == "distribution" {
                    saw_distribution = true;
                }
                if skip_value_partial(self.source, &mut index).is_err() {
                    return Ok(IncrementalScan::Incomplete);
                }
                if saw_distribution && !format_version_complete {
                    return Ok(IncrementalScan::LateHeaderPending);
                }
            }
            skip_ws(self.source, &mut index);
        }
    }

    fn scan_partial(&self) -> Result<Option<JsonRootHeader>, TransportDiagnostic> {
        let mut index = 0_usize;
        skip_ws(self.source, &mut index);
        if peek(self.source, index) != Some(b'{') {
            return Ok(None);
        }
        index += 1;
        skip_ws(self.source, &mut index);

        let mut member_order = Vec::new();
        let mut format_version_count = 0_usize;
        let mut scalar = None;
        let mut format_version_offset = 0_usize;
        let mut saw_distribution = false;
        let mut first_member = true;

        loop {
            skip_ws(self.source, &mut index);
            if peek(self.source, index) == Some(b'}') {
                break;
            }
            if peek(self.source, index).is_none() {
                return Ok(None);
            }
            if !first_member {
                if peek(self.source, index) != Some(b',') {
                    return Ok(None);
                }
                index += 1;
                skip_ws(self.source, &mut index);
            }
            first_member = false;

            let key = match read_string_partial(self.source, &mut index) {
                Ok(value) => value,
                Err(PartialScanError::NeedMore) => return Ok(None),
                Err(PartialScanError::Invalid(error)) => return Err(error),
            };
            member_order.push(key.clone());
            skip_ws(self.source, &mut index);
            if peek(self.source, index) != Some(b':') {
                return Ok(None);
            }
            index += 1;
            skip_ws(self.source, &mut index);

            if key == "formatVersion" {
                format_version_count += 1;
                format_version_offset = index;
                match read_scalar_partial(self.source, &mut index) {
                    Ok(value) => {
                        scalar = Some(value);
                    }
                    Err(PartialScanError::NeedMore) => return Ok(None),
                    Err(PartialScanError::Invalid(error)) => return Err(error),
                }
            } else {
                if key == "distribution" {
                    saw_distribution = true;
                }
                if skip_value_partial(self.source, &mut index).is_err() {
                    return Ok(None);
                }
            }
            skip_ws(self.source, &mut index);
        }

        Ok(Some(JsonRootHeader {
            scalar,
            member_order,
            format_version_offset,
            replay_kind: ReplayKind::None,
            saw_distribution,
            format_version_complete: format_version_count > 0,
        }))
    }
}

fn read_string_partial(source: &[u8], index: &mut usize) -> Result<String, PartialScanError> {
    if peek(source, *index) != Some(b'"') {
        return Err(PartialScanError::Invalid(probe_syntax_error(
            "invalid JSON syntax",
        )));
    }
    let start = *index;
    *index += 1;
    let mut escaped = false;
    while *index < source.len() {
        let byte = source[*index];
        *index += 1;
        if escaped {
            escaped = false;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            continue;
        }
        if byte == b'"' {
            let quoted = std::str::from_utf8(&source[start..*index])
                .map_err(|error| PartialScanError::Invalid(probe_io_error(error)))?;
            return serde_json::from_str(quoted)
                .map_err(|_| PartialScanError::Invalid(probe_syntax_error("invalid JSON string")));
        }
    }
    Err(PartialScanError::NeedMore)
}

fn read_scalar_partial(source: &[u8], index: &mut usize) -> Result<ScalarValue, PartialScanError> {
    if peek(source, *index) == Some(b'-') {
        return Err(PartialScanError::Invalid(transport_from_format_version(
            FormatVersionDiagnostic::invalid_format_version_type(),
        )));
    }
    match peek(source, *index) {
        Some(b'"') => {
            let string = read_string_partial(source, index)?;
            ScalarValue::from_json(&serde_json::Value::String(string))
                .map_err(|error| PartialScanError::Invalid(transport_from_format_version(error)))
        }
        Some(b'0'..=b'9') => {
            let start = *index;
            while matches!(peek(source, *index), Some(b'0'..=b'9')) {
                *index += 1;
            }
            let digits = std::str::from_utf8(&source[start..*index])
                .map_err(|error| PartialScanError::Invalid(probe_io_error(error)))?;
            let integer = digits.parse::<u64>().map_err(|_| {
                PartialScanError::Invalid(transport_from_format_version(
                    FormatVersionDiagnostic::invalid_format_version_type(),
                ))
            })?;
            Ok(ScalarValue::Integer(integer))
        }
        None => Err(PartialScanError::NeedMore),
        _ => Err(PartialScanError::Invalid(transport_from_format_version(
            FormatVersionDiagnostic::invalid_format_version_type(),
        ))),
    }
}

fn skip_value_partial(source: &[u8], index: &mut usize) -> Result<(), PartialScanError> {
    match peek(source, *index) {
        Some(b'"') => {
            read_string_partial(source, index)?;
        }
        Some(b'{') => skip_object_partial(source, index)?,
        Some(b'[') => skip_array_partial(source, index)?,
        Some(b't') => consume_literal_partial(source, index, "true")?,
        Some(b'f') => consume_literal_partial(source, index, "false")?,
        Some(b'n') => consume_literal_partial(source, index, "null")?,
        Some(b'0'..=b'9') | Some(b'-') => skip_number_partial(source, index)?,
        None => return Err(PartialScanError::NeedMore),
        _ => {
            return Err(PartialScanError::Invalid(probe_syntax_error(
                "invalid JSON value",
            )));
        }
    }
    Ok(())
}

fn skip_object_partial(source: &[u8], index: &mut usize) -> Result<(), PartialScanError> {
    if peek(source, *index) != Some(b'{') {
        return Err(PartialScanError::Invalid(probe_syntax_error(
            "invalid JSON syntax",
        )));
    }
    *index += 1;
    skip_ws(source, index);
    let mut first = true;
    loop {
        skip_ws(source, index);
        if peek(source, *index) == Some(b'}') {
            *index += 1;
            return Ok(());
        }
        if peek(source, *index).is_none() {
            return Err(PartialScanError::NeedMore);
        }
        if !first {
            if peek(source, *index) != Some(b',') {
                return Err(PartialScanError::NeedMore);
            }
            *index += 1;
            skip_ws(source, index);
        }
        first = false;
        read_string_partial(source, index)?;
        skip_ws(source, index);
        if peek(source, *index) != Some(b':') {
            return Err(PartialScanError::NeedMore);
        }
        *index += 1;
        skip_ws(source, index);
        skip_value_partial(source, index)?;
    }
}

fn skip_array_partial(source: &[u8], index: &mut usize) -> Result<(), PartialScanError> {
    if peek(source, *index) != Some(b'[') {
        return Err(PartialScanError::Invalid(probe_syntax_error(
            "invalid JSON syntax",
        )));
    }
    *index += 1;
    skip_ws(source, index);
    let mut first = true;
    loop {
        skip_ws(source, index);
        if peek(source, *index) == Some(b']') {
            *index += 1;
            return Ok(());
        }
        if peek(source, *index).is_none() {
            return Err(PartialScanError::NeedMore);
        }
        if !first {
            if peek(source, *index) != Some(b',') {
                return Err(PartialScanError::NeedMore);
            }
            *index += 1;
            skip_ws(source, index);
        }
        first = false;
        skip_value_partial(source, index)?;
    }
}

fn skip_number_partial(source: &[u8], index: &mut usize) -> Result<(), PartialScanError> {
    if peek(source, *index) == Some(b'-') {
        *index += 1;
    }
    if !matches!(peek(source, *index), Some(b'0'..=b'9')) {
        return Err(PartialScanError::NeedMore);
    }
    while matches!(peek(source, *index), Some(b'0'..=b'9')) {
        *index += 1;
    }
    if peek(source, *index) == Some(b'.') {
        *index += 1;
        while matches!(peek(source, *index), Some(b'0'..=b'9')) {
            *index += 1;
        }
    }
    if matches!(peek(source, *index), Some(b'e' | b'E')) {
        *index += 1;
        if matches!(peek(source, *index), Some(b'+' | b'-')) {
            *index += 1;
        }
        while matches!(peek(source, *index), Some(b'0'..=b'9')) {
            *index += 1;
        }
    }
    Ok(())
}

fn consume_literal_partial(
    source: &[u8],
    index: &mut usize,
    literal: &str,
) -> Result<(), PartialScanError> {
    let end = *index + literal.len();
    if source.get(*index..end) != Some(literal.as_bytes()) {
        if source.len() < end {
            return Err(PartialScanError::NeedMore);
        }
        return Err(PartialScanError::Invalid(probe_syntax_error(
            "invalid JSON literal",
        )));
    }
    *index = end;
    Ok(())
}

fn skip_ws(source: &[u8], index: &mut usize) {
    while matches!(peek(source, *index), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        *index += 1;
    }
}

fn peek(source: &[u8], index: usize) -> Option<u8> {
    source.get(index).copied()
}

enum PartialScanError {
    NeedMore,
    Invalid(TransportDiagnostic),
}

fn transport_from_format_version(error: FormatVersionDiagnostic) -> TransportDiagnostic {
    TransportDiagnostic::error(
        error.code(),
        Stage::Detection,
        morphir_core::traversal::IrCursor::root(),
        error.message(),
    )
}

fn probe_io_error(error: impl std::error::Error) -> TransportDiagnostic {
    TransportDiagnostic::error(
        "morphir::ir::root_probe::io_failed",
        Stage::Detection,
        morphir_core::traversal::IrCursor::root(),
        error.to_string(),
    )
}

fn probe_syntax_error(message: &'static str) -> TransportDiagnostic {
    TransportDiagnostic::error(
        "morphir::ir::json::invalid_syntax",
        Stage::Syntax,
        morphir_core::traversal::IrCursor::root(),
        message,
    )
    .with_source_span(SourceSpan {
        offset: 0,
        length: 0,
        line: 1,
        column: 1,
    })
}

struct YamlRootScanner<'source> {
    source: &'source str,
}

impl<'source> YamlRootScanner<'source> {
    fn new(source: &'source str) -> Self {
        Self { source }
    }

    fn scan(self) -> Result<JsonRootHeader, TransportDiagnostic> {
        let mut member_order = Vec::new();
        let mut format_version_count = 0_usize;
        let mut scalar = None;
        let mut format_version_offset = 0_usize;
        let mut saw_distribution = false;
        let mut byte_offset = 0_usize;

        for (line_index, line) in self.source.split_inclusive('\n').enumerate() {
            let line_without_ending = line.trim_end_matches(['\r', '\n']);
            let trimmed = line_without_ending.trim_start();
            let indent = line_without_ending.len() - trimmed.len();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                byte_offset += line.len();
                continue;
            }
            if trimmed == "---" {
                byte_offset += line.len();
                continue;
            }
            if indent != 0 {
                byte_offset += line.len();
                continue;
            }

            let Some((raw_key, value)) = split_yaml_mapping_line(trimmed) else {
                byte_offset += line.len();
                continue;
            };
            let key = normalize_yaml_key(raw_key);
            member_order.push(key.to_owned());

            if key == "formatVersion" {
                format_version_count += 1;
                if format_version_count > 1 {
                    let column = line_without_ending
                        .find(':')
                        .map(|index| index + 1)
                        .unwrap_or(1);
                    return Err(transport_from_format_version(
                        FormatVersionDiagnostic::duplicate_format_version(),
                    )
                    .with_source_span(SourceSpan {
                        offset: byte_offset,
                        length: line_without_ending.len(),
                        line: line_index + 1,
                        column,
                    })
                    .with_guidance("remove the repeated formatVersion member"));
                }
                format_version_offset =
                    byte_offset + line_without_ending.find(':').unwrap_or(0) + 1;
                scalar = Some(parse_yaml_scalar_value(value)?);
            } else if key == "distribution" {
                saw_distribution = true;
            }
            byte_offset += line.len();
            let _ = line_index;
        }

        if format_version_count == 0 {
            return Err(transport_from_format_version(
                FormatVersionDiagnostic::missing_format_version(),
            ));
        }

        let replay_kind = if member_order
            .first()
            .is_some_and(|member| member == "formatVersion")
        {
            ReplayKind::None
        } else if saw_distribution {
            if self.source.len() <= REPLAY_MEMORY_THRESHOLD {
                ReplayKind::Memory
            } else {
                ReplayKind::TemporaryStorage
            }
        } else {
            ReplayKind::None
        };

        Ok(JsonRootHeader {
            scalar,
            member_order,
            format_version_offset,
            replay_kind,
            saw_distribution,
            format_version_complete: true,
        })
    }
}

fn split_yaml_mapping_line(line: &str) -> Option<(&str, &str)> {
    let colon = line.find(':')?;
    let key = line[..colon].trim();
    let value = strip_yaml_inline_comment(line[colon + 1..].trim());
    if key.is_empty() {
        return None;
    }
    Some((key, value))
}

fn strip_yaml_inline_comment(value: &str) -> &str {
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
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
            '#' if yaml_token_boundary(value[..index].chars().next_back()) => {
                return value[..index].trim_end();
            }
            _ => {}
        }
    }
    value
}

fn yaml_token_boundary(character: Option<char>) -> bool {
    character.is_none_or(|character| {
        character.is_whitespace() || matches!(character, ':' | ',' | '[' | ']' | '{' | '}' | '-')
    })
}

fn normalize_yaml_key(key: &str) -> &str {
    let key = key.trim();
    if key.len() >= 2
        && ((key.starts_with('\'') && key.ends_with('\''))
            || (key.starts_with('"') && key.ends_with('"')))
    {
        return &key[1..key.len() - 1];
    }
    key
}

fn parse_yaml_scalar_value(value: &str) -> Result<ScalarValue, TransportDiagnostic> {
    let value = value.trim();
    if value.is_empty() {
        return Err(transport_from_format_version(
            FormatVersionDiagnostic::missing_format_version(),
        ));
    }
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        let string = value[1..value.len() - 1].replace("\\\"", "\"");
        return ScalarValue::from_json(&serde_json::Value::String(string))
            .map_err(transport_from_format_version);
    }
    if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
        let string = value[1..value.len() - 1].replace("\\'", "'");
        return ScalarValue::from_json(&serde_json::Value::String(string))
            .map_err(transport_from_format_version);
    }
    if value.starts_with('-') {
        return Err(transport_from_format_version(
            FormatVersionDiagnostic::invalid_format_version_type(),
        ));
    }
    if value.chars().all(|ch| ch.is_ascii_digit()) {
        let integer = value.parse::<u64>().map_err(|_| {
            transport_from_format_version(FormatVersionDiagnostic::invalid_format_version_type())
        })?;
        return Ok(ScalarValue::Integer(integer));
    }
    ScalarValue::from_json(&serde_json::Value::String(value.to_owned()))
        .map_err(transport_from_format_version)
}

/// Convert one header observation into a transport warning diagnostic.
pub fn observation_diagnostic(observation: &HeaderObservation) -> TransportDiagnostic {
    let mut diagnostic = TransportDiagnostic::warning(
        observation.code,
        Stage::Detection,
        morphir_core::traversal::IrCursor::root(),
        &observation.message,
    );
    if let Some(replay) = observation.replay {
        diagnostic = diagnostic.with_guidance(format!(
            "formatVersion offset={} bytes_scanned={} replay={:?}",
            replay.format_version_offset, replay.bytes_scanned, replay.replay_kind
        ));
    }
    diagnostic
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_core::format_version::ReleaseTriplet;

    #[test]
    fn canonical_json_header_streams_without_replay() {
        let source =
            br#"{"formatVersion":3,"distribution":["Library",[[["example"]]],[],{"modules":[]}]}"#;
        let mut reader = &source[..];
        let (probe, input) = probe_json_root(&mut reader, &SupportTable::reference()).unwrap();
        assert_eq!(probe.normalized.release, ReleaseTriplet::new(3, 0, 0));
        assert_eq!(probe.replay.replay_kind, ReplayKind::None);
        assert!(probe.observations.is_empty());
        assert!(matches!(input, ProbedJsonReader::Stream(_)));
    }

    #[test]
    fn noncanonical_json_header_replays_from_memory() {
        let source =
            br#"{"distribution":["Library",[[["example"]]],[],{"modules":[]}],"formatVersion":3}"#;
        let mut reader = &source[..];
        let (probe, input) = probe_json_root(&mut reader, &SupportTable::reference()).unwrap();
        assert_eq!(probe.replay.replay_kind, ReplayKind::Memory);
        assert_eq!(probe.observations[0].code, "format_version_not_first");
        assert!(matches!(input, ProbedJsonReader::Memory(_)));
    }

    #[test]
    fn unsupported_revision_fails_before_replay() {
        let source = br#"{"formatVersion":"3.1.0","distribution":[]}"#;
        let error = probe_json_root(&mut &source[..], &SupportTable::reference()).unwrap_err();
        assert_eq!(error.code(), "unsupported_format_version_revision");
    }

    #[test]
    fn duplicate_format_version_is_detected_during_streaming_probe() {
        let source = br#"{"formatVersion":3,"formatVersion":3,"distribution":["Library",[[["example"]]],[],{"modules":[]}]}"#;
        let mut reader = &source[..];
        let error = probe_json_root(&mut reader, &SupportTable::reference())
            .expect_err("duplicate formatVersion");
        assert_eq!(error.code(), "duplicate_format_version");
    }

    #[test]
    fn yaml_format_version_with_inline_comment_is_recognized() {
        let source = b"formatVersion: 4 # current format\ndistribution:\n  Specs:\n    packageName: example\n    modules: {}\n";
        let probe = probe_yaml_slice(source, &SupportTable::reference()).expect("yaml probe");
        assert_eq!(probe.normalized.release, ReleaseTriplet::new(4, 0, 0));
    }
}
