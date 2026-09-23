//! Amazon Ion codec for a single-file Morphir IR distribution.
//!
//! `ionVersion` is the spelling contract. A missing value means the latest
//! version this reader implements. `formatVersion` selects the IR.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};

use ion_rs::{Element, IonType, Symbol};
use morphir_core::ir::classic;
use morphir_core::traversal::IrCursor;

use super::semantic;
use super::{
    CodecOptions, EventSink, EventSource, FormatId, IrCodec, IrVersion, Stage, TransportDiagnostic,
};

/// Spelling contract implemented by this reader. Drafts match only this string.
const ION_CONTRACT: &str = "0.1.0-draft.1";

const HEADER_MEMBERS: &[&str] = &[
    "critical",
    "formatVersion",
    "ionVersion",
    "kind",
    "packageName",
];

/// Built-in Ion IR codec.
pub struct IonCodec {
    format: FormatId,
}

impl IonCodec {
    /// Create the built-in Ion codec.
    pub fn new() -> Self {
        Self {
            format: FormatId::ion(),
        }
    }

    fn error(code: &'static str, stage: Stage, message: impl Into<String>) -> TransportDiagnostic {
        TransportDiagnostic::error(code, stage, IrCursor::root(), message)
    }
}

impl Default for IonCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl IrCodec for IonCodec {
    fn format(&self) -> &FormatId {
        &self.format
    }

    fn decode(
        &self,
        reader: &mut dyn Read,
        options: &CodecOptions,
        sink: &mut dyn EventSink,
    ) -> Result<(), TransportDiagnostic> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).map_err(|error| {
            IonCodec::error(
                "morphir::ir::ion::read_failed",
                Stage::Syntax,
                error.to_string(),
            )
        })?;
        let values = Element::read_all(&bytes).map_err(|error| {
            IonCodec::error(
                "morphir::ir::ion::invalid_syntax",
                Stage::Syntax,
                error.to_string(),
            )
            .with_guidance("correct the Ion syntax or select the actual input format")
        })?;
        let header = values.get(0).ok_or_else(|| {
            IonCodec::error(
                "morphir::ir::ion::unexpected_value",
                Stage::Detection,
                "an Ion IR document starts with morphir::",
            )
        })?;
        expect_marker(header, "morphir")?;
        let header_fields = struct_fields(header, "morphir")?;
        match values.get(1) {
            None => {}
            Some(footer) if values.len() == 2 => {
                expect_marker(footer, "morphir_footer")?;
                let footer_fields = struct_fields(footer, "morphir_footer")?;
                if !footer_fields.is_empty() {
                    return Err(IonCodec::error(
                        "morphir::ir::ion::unexpected_member",
                        Stage::Normalization,
                        "morphir_footer has no members",
                    ));
                }
            }
            Some(_) => {
                return Err(IonCodec::error(
                    "morphir::ir::ion::unexpected_value",
                    Stage::Detection,
                    format!(
                        "a record is one morphir value and an empty datagram ends with morphir_footer, found {} top-level values",
                        values.len()
                    ),
                ));
            }
        }
        let package = decode_v3_library_header(&header_fields, options.version())?;
        semantic::emit_classic_v3(package, sink)
    }

    fn encode(
        &self,
        source: &mut dyn EventSource,
        writer: &mut dyn Write,
        options: &CodecOptions,
    ) -> Result<(), TransportDiagnostic> {
        match semantic::collect(source, options.version())? {
            semantic::SemanticFile::ClassicV3(distribution) => {
                write_empty_v3_library(distribution, writer)
            }
            semantic::SemanticFile::V4(_) => Err(IonCodec::error(
                "morphir::ir::ion::encode_unsupported",
                Stage::Encoding,
                "the Ion codec encodes formatVersion 3 only",
            )),
        }
    }
}

fn write_empty_v3_library(
    distribution: classic::Distribution,
    writer: &mut dyn Write,
) -> Result<(), TransportDiagnostic> {
    if distribution.format_version != 3 {
        return Err(IonCodec::error(
            "morphir::ir::ion::version_mismatch",
            Stage::Encoding,
            format!(
                "the v3 Ion writer received formatVersion {}",
                distribution.format_version
            ),
        ));
    }
    let classic::DistributionBody::Library(package, dependencies, definition) =
        distribution.distribution;
    if !dependencies.is_empty() || !definition.modules.is_empty() {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_node",
            Stage::Encoding,
            "the Ion writer encodes an empty v3 library",
        ));
    }
    let header = Element::from(ion_rs::ion_struct! {
        "ionVersion": ION_CONTRACT,
        "formatVersion": "3.0.0",
        "kind": Element::symbol("library"),
        "packageName": canonical_package(&package),
    })
    .with_annotations(["morphir"]);
    let footer =
        Element::from(ion_rs::Struct::builder().build()).with_annotations(["morphir_footer"]);
    let text: String = ion_rs::ion_seq![header, footer]
        .encode_as(ion_rs::v1_0::Text.with_format(ion_rs::TextFormat::Pretty))
        .map_err(|error| {
            IonCodec::error(
                "morphir::ir::ion::encode_failed",
                Stage::Encoding,
                error.to_string(),
            )
        })?;
    writer.write_all(text.as_bytes()).map_err(|error| {
        IonCodec::error(
            "morphir::ir::ion::encode_failed",
            Stage::Encoding,
            error.to_string(),
        )
    })?;
    if !text.ends_with('\n') {
        writer.write_all(b"\n").map_err(|error| {
            IonCodec::error(
                "morphir::ir::ion::encode_failed",
                Stage::Encoding,
                error.to_string(),
            )
        })?;
    }
    Ok(())
}

fn canonical_package(path: &classic::Path) -> String {
    path.segments
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("/")
}

fn decode_v3_library_header(
    fields: &BTreeMap<&str, &Element>,
    selected: IrVersion,
) -> Result<classic::Distribution, TransportDiagnostic> {
    reject_critical_unknowns(fields)?;
    accept_ion_version(optional_string(fields, "ionVersion")?)?;
    let format_version = required_string(fields, "formatVersion")?;
    let major = accept_format_version(format_version, selected)?;
    if selected != IrVersion::V3 {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_version",
            Stage::Normalization,
            "the Ion codec decodes formatVersion 3 only",
        ));
    }
    let kind = required_text(fields, "kind")?;
    if kind != "library" {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_kind",
            Stage::Normalization,
            format!("a v3 Ion distribution has kind library, found {kind}"),
        ));
    }
    let package_name = required_string(fields, "packageName")?;
    let package = classic_path(package_name)?;
    Ok(classic::Distribution {
        format_version: major,
        distribution: classic::DistributionBody::Library(
            package,
            Vec::new(),
            classic::PackageDefinition::<classic::Attrs, classic::Type<classic::Attrs>> {
                modules: Vec::new(),
            },
        ),
    })
}

fn accept_ion_version(text: Option<&str>) -> Result<(), TransportDiagnostic> {
    let Some(text) = text else {
        return Ok(());
    };
    let version = semver::Version::parse(text).map_err(|error| {
        IonCodec::error(
            "morphir::ir::ion::unsupported_version",
            Stage::Detection,
            format!("ionVersion '{text}' is not a SemVer version: {error}"),
        )
    })?;
    let supported = semver::Version::parse(ION_CONTRACT).expect("ion contract version parses");
    if version != supported || version.to_string() != text {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_version",
            Stage::Detection,
            format!("this reader implements ionVersion {ION_CONTRACT}, found {text}"),
        )
        .with_guidance("omit ionVersion or set it to the implemented draft"));
    }
    Ok(())
}

fn accept_format_version(text: &str, selected: IrVersion) -> Result<u32, TransportDiagnostic> {
    let version = semver::Version::parse(text).map_err(|error| {
        IonCodec::error(
            "morphir::ir::ion::unsupported_version",
            Stage::Detection,
            format!("formatVersion '{text}' is not a SemVer version: {error}"),
        )
    })?;
    if version.to_string() != text || !version.pre.is_empty() || !version.build.is_empty() {
        return Err(IonCodec::error(
            "morphir::ir::ion::unsupported_version",
            Stage::Detection,
            format!("formatVersion must be a canonical release such as 3.0.0, found {text}"),
        ));
    }
    let major = match selected {
        IrVersion::V3 => 3,
        IrVersion::V4 => 4,
    };
    if version.major != major || version.minor != 0 {
        return Err(IonCodec::error(
            "morphir::ir::ion::version_mismatch",
            Stage::Detection,
            format!(
                "the selected {:?} codec accepts formatVersion {major}.0.x, found {text}",
                selected
            ),
        ));
    }
    Ok(u32::try_from(major).expect("IR major fits in u32"))
}

fn reject_critical_unknowns(fields: &BTreeMap<&str, &Element>) -> Result<(), TransportDiagnostic> {
    let Some(critical) = fields.get("critical") else {
        return Ok(());
    };
    let names = text_list(critical, "critical")?;
    let known: BTreeSet<&str> = HEADER_MEMBERS.iter().copied().collect();
    for name in names {
        if !known.contains(name.as_str()) {
            return Err(IonCodec::error(
                "morphir::ir::ion::critical_member",
                Stage::Normalization,
                format!("critical names '{name}', which this reader does not understand"),
            ));
        }
    }
    Ok(())
}

fn classic_path(canonical: &str) -> Result<classic::Path, TransportDiagnostic> {
    if canonical.is_empty() || canonical.starts_with('/') || canonical.ends_with('/') {
        return Err(IonCodec::error(
            "morphir::ir::ion::invalid_name",
            Stage::Normalization,
            format!("packageName '{canonical}' is not a canonical package name"),
        ));
    }
    let mut segments = Vec::new();
    for segment in canonical.split('/') {
        if segment.is_empty() {
            return Err(IonCodec::error(
                "morphir::ir::ion::invalid_name",
                Stage::Normalization,
                format!("packageName '{canonical}' is not a canonical package name"),
            ));
        }
        let name = classic::Name::from_str(segment);
        if name.words.is_empty() {
            return Err(IonCodec::error(
                "morphir::ir::ion::invalid_name",
                Stage::Normalization,
                format!("packageName '{canonical}' is not a canonical package name"),
            ));
        }
        segments.push(name);
    }
    Ok(classic::Path::new(segments))
}

fn expect_marker(element: &Element, expected: &str) -> Result<(), TransportDiagnostic> {
    let mut names = Vec::new();
    for symbol in element.annotations().iter() {
        names.push(symbol_text(symbol)?);
    }
    if names != [expected] {
        return Err(IonCodec::error(
            "morphir::ir::ion::unexpected_value",
            Stage::Detection,
            format!(
                "expected {expected}::, found {}",
                display_annotations(&names)
            ),
        ));
    }
    Ok(())
}

fn struct_fields<'a>(
    element: &'a Element,
    marker: &str,
) -> Result<BTreeMap<&'a str, &'a Element>, TransportDiagnostic> {
    if element.is_null() || element.ion_type() != IonType::Struct {
        return Err(IonCodec::error(
            "morphir::ir::ion::unexpected_value",
            Stage::Detection,
            format!("{marker}:: is a struct"),
        ));
    }
    let value = element.as_struct().expect("ion type checked");
    let mut fields = BTreeMap::new();
    for (symbol, field) in value.fields() {
        let name = symbol_text(symbol)?;
        if fields.insert(name, field).is_some() {
            return Err(IonCodec::error(
                "morphir::ir::ion::duplicate_field",
                Stage::Normalization,
                format!("{marker} repeats field '{name}'"),
            ));
        }
    }
    Ok(fields)
}

fn symbol_text(symbol: &Symbol) -> Result<&str, TransportDiagnostic> {
    symbol.text().ok_or_else(|| {
        IonCodec::error(
            "morphir::ir::ion::invalid_symbol",
            Stage::Syntax,
            "an Ion symbol with no text cannot name a Morphir member",
        )
    })
}

fn optional_string<'a>(
    fields: &BTreeMap<&str, &'a Element>,
    name: &str,
) -> Result<Option<&'a str>, TransportDiagnostic> {
    match fields.get(name) {
        None => Ok(None),
        Some(element) => Ok(Some(require_string(element, name)?)),
    }
}

fn required_string<'a>(
    fields: &BTreeMap<&str, &'a Element>,
    name: &str,
) -> Result<&'a str, TransportDiagnostic> {
    let element = required_field(fields, name)?;
    require_string(element, name)
}

fn required_text<'a>(
    fields: &BTreeMap<&str, &'a Element>,
    name: &str,
) -> Result<&'a str, TransportDiagnostic> {
    let element = required_field(fields, name)?;
    if let Some(text) = element.as_string() {
        return Ok(text);
    }
    if let Some(symbol) = element.as_symbol() {
        return symbol_text(symbol);
    }
    Err(IonCodec::error(
        "morphir::ir::ion::invalid_member",
        Stage::Normalization,
        format!("{name} is a string or a symbol"),
    ))
}

fn required_field<'a>(
    fields: &BTreeMap<&str, &'a Element>,
    name: &str,
) -> Result<&'a Element, TransportDiagnostic> {
    fields.get(name).copied().ok_or_else(|| {
        IonCodec::error(
            "morphir::ir::ion::missing_member",
            Stage::Normalization,
            format!("{name} is required"),
        )
    })
}

fn require_string<'a>(element: &'a Element, name: &str) -> Result<&'a str, TransportDiagnostic> {
    element.as_string().ok_or_else(|| {
        IonCodec::error(
            "morphir::ir::ion::invalid_member",
            Stage::Normalization,
            format!("{name} is a string"),
        )
    })
}

fn text_list(element: &Element, name: &str) -> Result<Vec<String>, TransportDiagnostic> {
    let Some(list) = element.as_list() else {
        return Err(IonCodec::error(
            "morphir::ir::ion::invalid_member",
            Stage::Normalization,
            format!("{name} is a list"),
        ));
    };
    let mut texts = Vec::new();
    for item in list.iter() {
        let text = if let Some(text) = item.as_string() {
            text
        } else if let Some(symbol) = item.as_symbol() {
            symbol_text(symbol)?
        } else {
            return Err(IonCodec::error(
                "morphir::ir::ion::invalid_member",
                Stage::Normalization,
                format!("{name} contains a string or a symbol"),
            ));
        };
        texts.push(text.to_owned());
    }
    Ok(texts)
}

fn display_annotations(names: &[&str]) -> String {
    if names.is_empty() {
        "no annotation".to_owned()
    } else {
        names.join("::")
    }
}
