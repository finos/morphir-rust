//! A path to a node of a document: `feature/rule[1]/scenario[2]/step[3]`. Indexes are 0-based and
//! count nodes of that kind under the same parent. A path can also start at `preamble`, the root
//! of the Markdown before the `Feature` heading of a `.feature.md` file.

use std::fmt;
use std::str::FromStr;

/// One segment of a `NodePath`: the kind of node it names, plus the index among its siblings of
/// the same kind for the segments that need one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Segment {
    /// The document's feature.
    Feature,
    /// The root of the document's preamble: the Markdown before the `Feature` heading of a
    /// `.feature.md` file.
    Preamble,
    /// A feature's or a rule's background.
    Background,
    /// A rule under the feature, by index among the feature's rules.
    Rule(usize),
    /// A scenario under a feature or a rule, by index among its scenarios.
    Scenario(usize),
    /// An examples block under a scenario, by index among its examples blocks.
    Examples(usize),
    /// A step under a scenario or a background, by index among its steps.
    Step(usize),
    /// A fence of a description or notes, by index among that description's fences.
    Fence(usize),
    /// A prose block of a description or notes, by index among that description's prose blocks.
    Prose(usize),
}

/// A path to a node of a document, as a sequence of `Segment`s from a root (`feature` or
/// `preamble`) down to the node.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NodePath {
    segments: Vec<Segment>,
}

impl NodePath {
    /// The path to the document's feature.
    pub fn feature() -> Self {
        Self {
            segments: vec![Segment::Feature],
        }
    }

    /// The root of the document's preamble.
    pub fn preamble() -> Self {
        Self {
            segments: vec![Segment::Preamble],
        }
    }

    /// A new path with `segment` appended, naming a child of the node this path names.
    #[must_use]
    pub fn push(&self, segment: Segment) -> Self {
        let mut segments = self.segments.clone();
        segments.push(segment);
        Self { segments }
    }

    /// Builds a path from its segments.
    pub fn from_segments(segments: &[Segment]) -> Self {
        Self {
            segments: segments.to_vec(),
        }
    }

    /// The path to this path's parent node, or `None` for a root path (`feature` or `preamble`).
    pub fn parent(&self) -> Option<Self> {
        (self.segments.len() > 1).then(|| Self {
            segments: self.segments[..self.segments.len() - 1].to_vec(),
        })
    }

    /// The path's segments, from its root to the node it names.
    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    /// The path's last segment: the segment that names the node itself.
    pub fn last(&self) -> Segment {
        *self
            .segments
            .last()
            .expect("a path has at least the feature segment")
    }
}

impl fmt::Display for Segment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Segment::Feature => f.write_str("feature"),
            Segment::Preamble => f.write_str("preamble"),
            Segment::Background => f.write_str("background"),
            Segment::Rule(i) => write!(f, "rule[{i}]"),
            Segment::Scenario(i) => write!(f, "scenario[{i}]"),
            Segment::Examples(i) => write!(f, "examples[{i}]"),
            Segment::Step(i) => write!(f, "step[{i}]"),
            Segment::Fence(i) => write!(f, "fence[{i}]"),
            Segment::Prose(i) => write!(f, "prose[{i}]"),
        }
    }
}

impl fmt::Display for NodePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.segments.iter().map(ToString::to_string).collect();
        f.write_str(&parts.join("/"))
    }
}

/// A string that does not parse as a `NodePath`.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("not a node path: {0}")]
pub struct NodePathError(
    /// The string that failed to parse.
    pub String,
);

impl FromStr for NodePath {
    type Err = NodePathError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = || NodePathError(text.to_owned());
        let mut segments = Vec::new();
        for (position, part) in text.split('/').enumerate() {
            let segment = match part {
                "feature" if position == 0 => Segment::Feature,
                "preamble" if position == 0 => Segment::Preamble,
                "background" => Segment::Background,
                _ => {
                    let (name, rest) = part.split_once('[').ok_or_else(bad)?;
                    let index: usize = rest
                        .strip_suffix(']')
                        .ok_or_else(bad)?
                        .parse()
                        .map_err(|_| bad())?;
                    match name {
                        "rule" => Segment::Rule(index),
                        "scenario" => Segment::Scenario(index),
                        "examples" => Segment::Examples(index),
                        "step" => Segment::Step(index),
                        "fence" => Segment::Fence(index),
                        "prose" => Segment::Prose(index),
                        _ => return Err(bad()),
                    }
                }
            };
            segments.push(segment);
        }
        if !matches!(
            segments.first(),
            Some(&Segment::Feature) | Some(&Segment::Preamble)
        ) {
            return Err(bad());
        }
        Ok(Self { segments })
    }
}
