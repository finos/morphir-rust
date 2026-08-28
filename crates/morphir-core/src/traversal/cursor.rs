use std::fmt;

/// A typed semantic location within a Morphir IR tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorSegment {
    Distribution,
    Package(String),
    Dependency(String),
    Module(String),
    Type(String),
    Value(String),
    Constructor(String),
    Field(String),
    Argument(usize),
    PatternCase(usize),
    LetBinding(String),
    Branch(&'static str),
}

impl fmt::Display for CursorSegment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Distribution => formatter.write_str("distribution"),
            Self::Package(name) => write!(formatter, "package:{name}"),
            Self::Dependency(name) => write!(formatter, "dependency:{name}"),
            Self::Module(name) => write!(formatter, "module:{name}"),
            Self::Type(name) => write!(formatter, "type:{name}"),
            Self::Value(name) => write!(formatter, "value:{name}"),
            Self::Constructor(name) => write!(formatter, "constructor:{name}"),
            Self::Field(name) => write!(formatter, "field:{name}"),
            Self::Argument(index) => write!(formatter, "argument:{index}"),
            Self::PatternCase(index) => write!(formatter, "pattern-case:{index}"),
            Self::LetBinding(name) => write!(formatter, "let-binding:{name}"),
            Self::Branch(name) => write!(formatter, "branch:{name}"),
        }
    }
}

/// Cursor used by visitors and migration diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IrCursor {
    segments: Vec<CursorSegment>,
}

impl IrCursor {
    pub fn from_segments(segments: impl IntoIterator<Item = CursorSegment>) -> Self {
        Self {
            segments: segments.into_iter().collect(),
        }
    }

    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn segments(&self) -> &[CursorSegment] {
        &self.segments
    }

    /// Run an operation under a child segment and restore the parent cursor.
    pub fn with_segment<R>(
        &mut self,
        segment: CursorSegment,
        operation: impl FnOnce(&mut Self) -> R,
    ) -> R {
        self.segments.push(segment);
        let result = operation(self);
        self.segments.pop();
        result
    }
}

impl fmt::Display for IrCursor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self
            .segments
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("/");
        formatter.write_str(&path)
    }
}
