//! Morphir IR v4 name model and canonical encodings.
//!
//! A [`Name`] is a sequence of [`Segment`]s. A segment is a lowercase alphanumeric
//! token that is either an ordinary word or an initialism. Marking an initialism as
//! a property of the word, rather than fragmenting it into single-letter words as
//! v1 through v3 did, lets each backend apply its own convention: a Go backend
//! renders `HTMLParser` and a Rust backend renders `HtmlParser` from one IR.
//!
//! Two canonical string encodings are implemented. [`CANONICAL_STYLE`] selects the
//! one that is written; decoding always accepts both. The two syntaxes are
//! disjoint, so a union decoder is unambiguous, and flipping the constant is
//! therefore backward compatible for every reader.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Stems that Windows reserves as device names. The reservation applies with any
/// extension, so `aux.type.json` cannot be created.
const RESERVED_DEVICE_STEMS: &[&str] = &[
    "con", "prn", "aux", "nul", "com0", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "lpt0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// One component of a [`Name`]. The text is always stored lowercase; the variant
/// carries the intent that rendering acts on.
#[derive(Debug, Clone, PartialEq, Eq, Hash, JsonSchema)]
pub enum Segment {
    /// An ordinary word, such as `user`.
    Word(String),
    /// An initialism, such as `usd`, which renders as `USD` or `Usd` depending on
    /// the target convention.
    Initialism(String),
}

/// Why a [`Segment`] cannot appear in a [`Name`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentError {
    /// The segment carries no text.
    Empty,
    /// The segment carries a character outside `[a-z0-9]`.
    InvalidCharacter { text: String },
    /// The initialism carries no letter. The canonical encoding uppercases an
    /// initialism, and uppercasing digits is a no-op, so a digits-only
    /// initialism is indistinguishable from the word with the same text and
    /// cannot survive a round trip.
    DigitOnlyInitialism { text: String },
}

impl fmt::Display for SegmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SegmentError::Empty => write!(f, "a name segment cannot be empty"),
            SegmentError::InvalidCharacter { text } => {
                write!(f, "name segment {text:?} is not lowercase alphanumeric")
            }
            SegmentError::DigitOnlyInitialism { text } => write!(
                f,
                "initialism {text:?} has no letter, so it cannot be distinguished from a word"
            ),
        }
    }
}

impl std::error::Error for SegmentError {}

impl Segment {
    /// The lowercase text of this segment, without its class.
    pub fn text(&self) -> &str {
        match self {
            Segment::Word(text) | Segment::Initialism(text) => text,
        }
    }

    pub fn is_initialism(&self) -> bool {
        matches!(self, Segment::Initialism(_))
    }

    /// Check the invariant this type documents but cannot enforce, because the
    /// variants are publicly constructible.
    pub fn validate(&self) -> Result<(), SegmentError> {
        let text = self.text();
        if text.is_empty() {
            return Err(SegmentError::Empty);
        }
        if !text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            return Err(SegmentError::InvalidCharacter {
                text: text.to_string(),
            });
        }
        if self.is_initialism() && !text.chars().any(|c| c.is_ascii_lowercase()) {
            return Err(SegmentError::DigitOnlyInitialism {
                text: text.to_string(),
            });
        }
        Ok(())
    }

    fn titled(&self) -> String {
        let text = self.text();
        let mut chars = text.chars();
        match chars.next() {
            None => String::new(),
            Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        }
    }
}

/// Which canonical string encoding to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameStyle {
    /// An initialism is an uppercase segment: `value-in-USD`.
    Uppercase,
    /// An initialism is preceded by a doubled separator: `value-in--usd`.
    /// Keeps every canonical name inside `[a-z0-9/-]`.
    DoubledHyphen,
}

/// The encoding written by [`Name::to_canonical_string`] and by `Serialize`.
///
/// This is deliberately a compile-time constant rather than a runtime setting.
/// If the encoding were selectable at run time, two producers configured
/// differently would emit different bytes for the same name and nothing
/// downstream could detect that the two artifacts describe the same thing.
pub const CANONICAL_STYLE: NameStyle = NameStyle::Uppercase;

#[derive(Debug, Clone, PartialEq, Eq, Hash, JsonSchema)]
#[schemars(with = "String")]
pub struct Name {
    segments: Vec<Segment>,
}

impl Name {
    /// Build a Name from legacy v1 through v3 words, applying the run-collapsing
    /// rule: a maximal run of two or more single-letter words becomes one
    /// initialism, and a run of one stays a word. The run-of-one rule is what
    /// keeps a single-letter type variable a word rather than an initialism.
    pub fn new(words: &[&str]) -> Self {
        Self::from_words(words.iter().map(|word| word.to_string()))
    }

    /// Build a Name from legacy words. See [`Name::new`] for the collapsing rule.
    pub fn from_words<I, S>(words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut segments = Vec::new();
        let mut run: Vec<String> = Vec::new();

        fn flush(run: &mut Vec<String>, segments: &mut Vec<Segment>) {
            match run.len() {
                0 => {}
                1 => segments.push(Segment::Word(run.remove(0))),
                _ => segments.push(Segment::Initialism(run.concat())),
            }
            run.clear();
        }

        for word in words {
            let word: String = word.into();
            let word = word.to_lowercase();
            // Only a run of single *letters* collapses. A single digit stays a
            // word: the canonical encoding uppercases an initialism, which does
            // nothing to digits, so a digits-only initialism would decode back as
            // a word and silently change identity.
            let is_single_letter = word.chars().count() == 1
                && word.chars().all(|character| character.is_ascii_lowercase());
            if is_single_letter {
                run.push(word);
            } else {
                flush(&mut run, &mut segments);
                segments.push(Segment::Word(word));
            }
        }
        flush(&mut run, &mut segments);

        Name { segments }
    }

    /// Build a Name from segments, checking each one.
    ///
    /// `Segment`'s variants are publicly constructible, so nothing stops a caller
    /// from building `Segment::Word("USD".into())`. Validating here means every
    /// `Name` holds the invariant however its segments were made.
    pub fn from_segments(segments: Vec<Segment>) -> Result<Self, SegmentError> {
        for segment in &segments {
            segment.validate()?;
        }
        Ok(Name { segments })
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// The legacy v1 through v3 word list. An initialism explodes back into single
    /// letters, so `from_words` and `words` round-trip for every name a legacy
    /// document can express.
    pub fn words(&self) -> Vec<String> {
        let mut words = Vec::new();
        for segment in &self.segments {
            match segment {
                Segment::Word(text) => words.push(text.clone()),
                Segment::Initialism(text) => {
                    words.extend(text.chars().map(|c| c.to_string()));
                }
            }
        }
        words
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Parse permissively. The canonical encodings are tried first, then
    /// `snake_case`, `kebab-case`, `camelCase` and `PascalCase`.
    pub fn from(name: &str) -> Self {
        if name.is_empty() {
            return Name {
                segments: Vec::new(),
            };
        }
        if let Ok(parsed) = Self::from_canonical_string(name) {
            return parsed;
        }

        let mut segments = Vec::new();
        for chunk in name.split(['_', '-', '/', '.', ':', ' ']) {
            if chunk.is_empty() {
                continue;
            }
            segments.extend(split_camel_case(chunk));
        }
        Name { segments }
    }

    /// Render using [`CANONICAL_STYLE`].
    pub fn to_canonical_string(&self) -> String {
        self.to_canonical_string_in(CANONICAL_STYLE)
    }

    pub fn to_canonical_string_in(&self, style: NameStyle) -> String {
        match style {
            NameStyle::Uppercase => self
                .segments
                .iter()
                .map(|segment| match segment {
                    Segment::Word(text) => text.clone(),
                    Segment::Initialism(text) => text.to_ascii_uppercase(),
                })
                .collect::<Vec<_>>()
                .join("-"),
            NameStyle::DoubledHyphen => {
                let mut out = String::new();
                for (index, segment) in self.segments.iter().enumerate() {
                    if segment.is_initialism() {
                        out.push_str("--");
                    } else if index > 0 {
                        out.push('-');
                    }
                    out.push_str(segment.text());
                }
                out
            }
        }
    }

    /// Parse a canonical name in either encoding.
    ///
    /// The two syntaxes are disjoint: a doubled separator requires an empty
    /// segment under [`NameStyle::Uppercase`], and [`NameStyle::DoubledHyphen`]
    /// admits no uppercase. A name carrying no initialism is legal under both and
    /// decodes identically.
    pub fn from_canonical_string(source: &str) -> Result<Self, String> {
        if source.is_empty() {
            return Ok(Name {
                segments: Vec::new(),
            });
        }

        let has_doubled = source.contains("--");
        let has_upper = source.chars().any(|c| c.is_ascii_uppercase());

        if has_doubled && has_upper {
            return Err(format!(
                "canonical name mixes both encodings, uppercase and doubled separator: {source}"
            ));
        }

        if has_doubled {
            parse_doubled_hyphen(source)
        } else {
            parse_uppercase(source)
        }
    }

    /// The filename stem this name projects onto in a document tree.
    ///
    /// The result is entirely lowercase, so it is stable on case-insensitive
    /// filesystems. An initialism carries a `_` prefix; a stem that collides with
    /// a Windows reserved device name carries a `_` suffix.
    pub fn to_file_stem(&self) -> String {
        let stem = self
            .segments
            .iter()
            .map(|segment| match segment {
                Segment::Word(text) => text.clone(),
                Segment::Initialism(text) => format!("_{text}"),
            })
            .collect::<Vec<_>>()
            .join("-");

        if RESERVED_DEVICE_STEMS.contains(&stem.to_ascii_lowercase().as_str()) {
            format!("{stem}_")
        } else {
            stem
        }
    }

    /// Recover a name from a filename stem produced by [`Name::to_file_stem`].
    ///
    /// This does not recover a truncated stem. When a stem is truncated for path
    /// length, the module manifest records the mapping.
    pub fn from_file_stem(stem: &str) -> Result<Self, String> {
        if stem.is_empty() {
            return Ok(Name {
                segments: Vec::new(),
            });
        }

        // No escaped stem ends in `_` unless the reserved-name suffix was applied,
        // and a word never contains `_`, so both markers are unambiguous.
        let stem = stem.strip_suffix('_').unwrap_or(stem);

        let mut segments = Vec::new();
        for part in stem.split('-') {
            let (text, initialism) = match part.strip_prefix('_') {
                Some(rest) => (rest, true),
                None => (part, false),
            };
            if text.is_empty() {
                return Err(format!("empty segment in file stem: {stem}"));
            }
            if !text
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            {
                return Err(format!("invalid character in file stem: {stem}"));
            }
            let segment = if initialism {
                Segment::Initialism(text.to_string())
            } else {
                Segment::Word(text.to_string())
            };
            segment
                .validate()
                .map_err(|error| format!("{error} in file stem: {stem}"))?;
            segments.push(segment);
        }
        Ok(Name { segments })
    }

    /// `valueInUSD`. A leading initialism is lowercased whole, so `IO-error`
    /// renders `ioError` rather than `iOError`.
    pub fn to_camel_case(&self) -> String {
        let mut out = String::new();
        for (index, segment) in self.segments.iter().enumerate() {
            if index == 0 {
                out.push_str(segment.text());
            } else if segment.is_initialism() {
                out.push_str(&segment.text().to_ascii_uppercase());
            } else {
                out.push_str(&segment.titled());
            }
        }
        out
    }

    /// `ValueInUSD`, the Go convention: an initialism is fully uppercase.
    pub fn to_pascal_case(&self) -> String {
        self.segments
            .iter()
            .map(|segment| {
                if segment.is_initialism() {
                    segment.text().to_ascii_uppercase()
                } else {
                    segment.titled()
                }
            })
            .collect()
    }

    /// `ValueInUsd`, the Rust and .NET convention: an initialism is title cased.
    pub fn to_pascal_case_pascal_initialism(&self) -> String {
        self.segments.iter().map(Segment::titled).collect()
    }

    /// `ValueInUSD`. Retained under its previous name; equivalent to
    /// [`Name::to_pascal_case`].
    pub fn to_title_case(&self) -> String {
        self.to_pascal_case()
    }

    pub fn to_snake_case(&self) -> String {
        self.segments
            .iter()
            .map(Segment::text)
            .collect::<Vec<_>>()
            .join("_")
    }

    pub fn to_kebab_case(&self) -> String {
        self.segments
            .iter()
            .map(Segment::text)
            .collect::<Vec<_>>()
            .join("-")
    }

    pub fn to_screaming_snake_case(&self) -> String {
        self.segments
            .iter()
            .map(|segment| segment.text().to_ascii_uppercase())
            .collect::<Vec<_>>()
            .join("_")
    }

    /// Segment text is lowercase by construction, so this is always true. Retained
    /// for callers that assert it.
    pub fn is_lowercase(&self) -> bool {
        self.segments
            .iter()
            .all(|segment| segment.text().chars().all(|c| !c.is_ascii_uppercase()))
    }
}

fn parse_uppercase(source: &str) -> Result<Name, String> {
    let mut segments = Vec::new();
    for part in source.split('-') {
        if part.is_empty() {
            return Err(format!("empty segment in canonical name: {source}"));
        }
        if !part.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(format!("invalid character in canonical name: {source}"));
        }
        let has_upper = part.chars().any(|c| c.is_ascii_uppercase());
        let has_lower = part.chars().any(|c| c.is_ascii_lowercase());
        if has_upper && has_lower {
            return Err(format!(
                "mixed-case segment {part:?} in canonical name: {source}"
            ));
        }
        segments.push(if has_upper {
            Segment::Initialism(part.to_ascii_lowercase())
        } else {
            // A digits-only segment reaches here and is classified as a word.
            Segment::Word(part.to_string())
        });
    }
    Ok(Name { segments })
}

fn parse_doubled_hyphen(source: &str) -> Result<Name, String> {
    if !source
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(format!("invalid character in canonical name: {source}"));
    }

    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut index = 0usize;
    let mut initialism;

    if source.starts_with("--") {
        initialism = true;
        index = 2;
    } else if source.starts_with('-') {
        return Err(format!("leading separator in canonical name: {source}"));
    } else {
        initialism = false;
    }

    let mut segments = Vec::new();
    loop {
        let start = index;
        while index < len && bytes[index] != b'-' {
            index += 1;
        }
        if index == start {
            return Err(format!("empty segment in canonical name: {source}"));
        }
        let text = source[start..index].to_string();
        let segment = if initialism {
            Segment::Initialism(text)
        } else {
            Segment::Word(text)
        };
        segment
            .validate()
            .map_err(|error| format!("{error} in canonical name: {source}"))?;
        segments.push(segment);

        if index == len {
            break;
        }

        let hyphen_start = index;
        while index < len && bytes[index] == b'-' {
            index += 1;
        }
        match index - hyphen_start {
            1 => initialism = false,
            2 => initialism = true,
            n => {
                return Err(format!("run of {n} separators in canonical name: {source}"));
            }
        }
        if index == len {
            return Err(format!("trailing separator in canonical name: {source}"));
        }
    }

    Ok(Name { segments })
}

/// Split one `camelCase` or `PascalCase` chunk into segments.
///
/// A run of two or more uppercase letters is an initialism, ending one letter
/// early when the next character is lowercase, so `parseHTMLDocument` yields
/// `parse`, initialism `html`, `document`. A run of one uppercase letter starts a
/// word.
fn split_camel_case(chunk: &str) -> Vec<Segment> {
    let chars: Vec<char> = chunk.chars().collect();
    let mut segments = Vec::new();
    let mut buffer = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index].is_ascii_uppercase() {
            let start = index;
            while index < chars.len() && chars[index].is_ascii_uppercase() {
                index += 1;
            }
            let mut end = index;
            if end - start >= 2 && index < chars.len() && chars[index].is_ascii_lowercase() {
                // The final uppercase letter begins the next word.
                end -= 1;
                index = end;
            }
            if end - start == 1 {
                if !buffer.is_empty() {
                    segments.push(Segment::Word(std::mem::take(&mut buffer)));
                }
                buffer.push(chars[start].to_ascii_lowercase());
            } else {
                if !buffer.is_empty() {
                    segments.push(Segment::Word(std::mem::take(&mut buffer)));
                }
                let run: String = chars[start..end]
                    .iter()
                    .map(|c| c.to_ascii_lowercase())
                    .collect();
                segments.push(Segment::Initialism(run));
            }
        } else {
            buffer.push(chars[index].to_ascii_lowercase());
            index += 1;
        }
    }

    if !buffer.is_empty() {
        segments.push(Segment::Word(buffer));
    }
    segments
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_canonical_string())
    }
}

impl Serialize for Name {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_canonical_string())
    }
}

impl<'de> Deserialize<'de> for Name {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;

        // Accept both canonical string encodings and the legacy array.
        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(s) => {
                Name::from_canonical_string(&s).map_err(de::Error::custom)
            }
            serde_json::Value::Array(arr) => {
                let words: Result<Vec<String>, _> = arr
                    .into_iter()
                    .map(|v| match v {
                        serde_json::Value::String(s) => Ok(s),
                        _ => Err(de::Error::custom("expected string in Name array")),
                    })
                    .collect();
                Ok(Name::from_words(words?))
            }
            _ => Err(de::Error::custom("expected string or array for Name")),
        }
    }
}
