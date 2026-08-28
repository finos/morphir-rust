//! Orthogonal options for IR codecs and storage layouts.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// Error returned when a codec or vocabulary identifier is invalid.
#[derive(Debug, Error, Clone, Eq, PartialEq)]
#[error(
    "identifier must start with an ASCII letter and contain only lowercase letters, digits, '-' or '_'"
)]
pub struct IdentifierError;

fn validate_identifier(value: &str) -> Result<(), IdentifierError> {
    let mut characters = value.chars();
    if !characters
        .next()
        .is_some_and(|value| value.is_ascii_lowercase())
        || !characters.all(|value| {
            value.is_ascii_lowercase() || value.is_ascii_digit() || matches!(value, '-' | '_')
        })
    {
        return Err(IdentifierError);
    }
    Ok(())
}

/// Open identifier for a physical IR serialization format.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FormatId(String);

impl FormatId {
    /// Create and validate a format identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_identifier(&value)?;
        Ok(Self(value))
    }

    /// Return the built-in JSON format identifier.
    pub fn json() -> Self {
        Self("json".to_owned())
    }

    /// Return the built-in YAML format identifier.
    pub fn yaml() -> Self {
        Self("yaml".to_owned())
    }

    /// Return the identifier text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for FormatId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for FormatId {
    type Err = IdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

/// Open identifier for a serialization vocabulary or presentation style.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VocabularyId(String);

impl VocabularyId {
    /// Create and validate a vocabulary identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_identifier(&value)?;
        Ok(Self(value))
    }

    /// Return the preferred readable vocabulary identifier.
    pub fn readable() -> Self {
        Self("readable".to_owned())
    }

    /// Return the explicit structural vocabulary identifier.
    pub fn structural() -> Self {
        Self("structural".to_owned())
    }

    /// Return the identifier text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VocabularyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for VocabularyId {
    type Err = IdentifierError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

/// Concrete Morphir IR version selected for a codec operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IrVersion {
    /// Concrete Morphir IR version 3.
    V3,
    /// Concrete Morphir IR version 4.
    V4,
}

/// Physical organization of a serialized distribution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Layout {
    /// Store one distribution in one file or stream.
    SingleFile,
    /// Store a distribution as a tree of logical documents.
    DocumentTree,
}

/// Policy used while normalizing physical syntax into semantic IR.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NormalizationPolicy {
    /// Reject ambiguous, lossy, and unsupported spellings.
    #[default]
    Strict,
}

/// Independent choices that configure an IR codec operation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct CodecOptions {
    version: IrVersion,
    layout: Layout,
    format: FormatId,
    vocabulary: VocabularyId,
    normalization: NormalizationPolicy,
}

impl CodecOptions {
    /// Create options with the readable vocabulary and strict normalization.
    pub fn new(version: IrVersion, layout: Layout, format: FormatId) -> Self {
        Self {
            version,
            layout,
            format,
            vocabulary: VocabularyId::readable(),
            normalization: NormalizationPolicy::Strict,
        }
    }

    /// Select a vocabulary without changing version, layout, or format.
    pub fn with_vocabulary(mut self, vocabulary: VocabularyId) -> Self {
        self.vocabulary = vocabulary;
        self
    }

    /// Return the selected IR version.
    pub fn version(&self) -> IrVersion {
        self.version
    }

    /// Return the selected storage layout.
    pub fn layout(&self) -> Layout {
        self.layout
    }

    /// Return the selected serialization format.
    pub fn format(&self) -> &FormatId {
        &self.format
    }

    /// Return the selected serialization vocabulary.
    pub fn vocabulary(&self) -> &VocabularyId {
        &self.vocabulary
    }

    /// Return the normalization policy.
    pub fn normalization(&self) -> NormalizationPolicy {
        self.normalization
    }
}
