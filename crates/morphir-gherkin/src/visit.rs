//! A visitor over a document, in source order.

use crate::model::*;
use crate::path::{NodePath, Segment};
use crate::span::Span;

/// A node of a document, as seen by a `Visitor` or `Cursor`: a borrow of the model value the
/// node's path names.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Node<'d> {
    /// A feature.
    Feature(&'d Feature),
    /// A background.
    Background(&'d Background),
    /// A rule.
    Rule(&'d Rule),
    /// A scenario.
    Scenario(&'d Scenario),
    /// An examples block.
    Examples(&'d Examples),
    /// A step.
    Step(&'d Step),
    /// A free fence of a description or of notes.
    Fence(&'d Fence),
    /// A prose block of a description or of notes.
    Prose(&'d ProseBlock),
}

impl Node<'_> {
    /// The node's own span. For a step, this covers only its line, not its argument or notes;
    /// see `crate::cursor` for the node that owns an offset inside those.
    pub fn span(&self) -> Span {
        match self {
            Node::Feature(n) => n.span,
            Node::Background(n) => n.span,
            Node::Rule(n) => n.span,
            Node::Scenario(n) => n.span,
            Node::Examples(n) => n.span,
            Node::Step(n) => n.span,
            Node::Fence(n) => n.span,
            Node::Prose(n) => n.span,
        }
    }
}

/// What `walk` does after a `Visitor::enter` call: descend into the node's children, or move on
/// without them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Walk {
    /// Visit the node's children next.
    Children,
    /// Move on to the node's next sibling, without visiting its children.
    Skip,
}

/// Visits a document's nodes in source order. `walk` calls `enter` before a node's children and
/// `exit` after them; both have a default that does nothing, so a visitor implements only the
/// one it needs.
///
/// ```
/// use morphir_gherkin::read_str;
/// use morphir_gherkin::visit::{Node, Visitor, Walk, walk};
///
/// struct CountSteps(usize);
/// impl<'d> Visitor<'d> for CountSteps {
///     fn enter(&mut self, _path: &morphir_gherkin::NodePath, node: Node<'d>) -> Walk {
///         if matches!(node, Node::Step(_)) {
///             self.0 += 1;
///         }
///         Walk::Children
///     }
/// }
///
/// let (doc, _) = read_str("f.feature", "Feature: F\n  Scenario: S\n    Given a\n    Then b\n")
///     .unwrap();
/// let mut counter = CountSteps(0);
/// walk(&doc, &mut counter);
/// assert_eq!(counter.0, 2);
/// ```
pub trait Visitor<'d> {
    /// Called when `walk` reaches a node, before its children (if any are visited). The default
    /// always descends into the node's children.
    fn enter(&mut self, _path: &NodePath, _node: Node<'d>) -> Walk {
        Walk::Children
    }
    /// Called when `walk` leaves a node, after its children (if `enter` chose to visit them).
    fn exit(&mut self, _path: &NodePath, _node: Node<'d>) {}
}

/// Appends the blocks of a description as `Fence`/`Prose` nodes rooted at `path`, in source
/// order. `fences` and `prose` are the next index to give each kind, and are updated so a caller
/// can keep counting across more than one description under the same path.
fn description_children<'d>(
    path: &NodePath,
    description: &'d Description,
    out: &mut Vec<(NodePath, Node<'d>)>,
    fences: &mut usize,
    prose: &mut usize,
) {
    for block in &description.blocks {
        match block {
            DescriptionBlock::Fence(f) => {
                out.push((path.push(Segment::Fence(*fences)), Node::Fence(f)));
                *fences += 1;
            }
            DescriptionBlock::Prose(p) => {
                out.push((path.push(Segment::Prose(*prose)), Node::Prose(p)));
                *prose += 1;
            }
        }
    }
}

/// The children of a node, with their paths, in source order.
pub fn children<'d>(path: &NodePath, node: Node<'d>) -> Vec<(NodePath, Node<'d>)> {
    let mut out = Vec::new();
    let description = |out: &mut Vec<(NodePath, Node<'d>)>, d: &'d Description| {
        let (mut fences, mut prose) = (0, 0);
        description_children(path, d, out, &mut fences, &mut prose);
    };
    let steps = |out: &mut Vec<(NodePath, Node<'d>)>, steps: &'d [Step]| {
        for (i, s) in steps.iter().enumerate() {
            out.push((path.push(Segment::Step(i)), Node::Step(s)));
        }
    };
    match node {
        Node::Feature(f) => {
            description(&mut out, &f.description);
            if let Some(b) = &f.background {
                out.push((path.push(Segment::Background), Node::Background(b)));
            }
            for (i, s) in f.scenarios.iter().enumerate() {
                out.push((path.push(Segment::Scenario(i)), Node::Scenario(s)));
            }
            for (i, r) in f.rules.iter().enumerate() {
                out.push((path.push(Segment::Rule(i)), Node::Rule(r)));
            }
        }
        Node::Rule(r) => {
            description(&mut out, &r.description);
            if let Some(b) = &r.background {
                out.push((path.push(Segment::Background), Node::Background(b)));
            }
            for (i, s) in r.scenarios.iter().enumerate() {
                out.push((path.push(Segment::Scenario(i)), Node::Scenario(s)));
            }
        }
        Node::Background(b) => {
            description(&mut out, &b.description);
            steps(&mut out, &b.steps);
        }
        Node::Scenario(s) => {
            description(&mut out, &s.description);
            steps(&mut out, &s.steps);
            for (i, e) in s.examples.iter().enumerate() {
                out.push((path.push(Segment::Examples(i)), Node::Examples(e)));
            }
        }
        Node::Examples(e) => {
            // The notes' prose and fence indexes continue on from the description's.
            let (mut fences, mut prose) = (0, 0);
            description_children(path, &e.description, &mut out, &mut fences, &mut prose);
            description_children(path, &e.notes, &mut out, &mut fences, &mut prose);
        }
        Node::Step(s) => description(&mut out, &s.notes),
        Node::Fence(_) | Node::Prose(_) => {}
    }
    out
}

/// The blocks of a document's preamble, as `Fence`/`Prose` nodes rooted at `NodePath::preamble()`,
/// in source order.
pub(crate) fn preamble_children(doc: &Document) -> Vec<(NodePath, Node<'_>)> {
    let mut out = Vec::new();
    let (mut fences, mut prose) = (0, 0);
    description_children(
        &NodePath::preamble(),
        &doc.preamble,
        &mut out,
        &mut fences,
        &mut prose,
    );
    out
}

/// Walks a document in source order: the preamble's blocks, then the feature, if it has one.
pub fn walk<'d>(doc: &'d Document, visitor: &mut impl Visitor<'d>) {
    for (path, node) in preamble_children(doc) {
        walk_node(&path, node, visitor);
    }
    if let Some(feature) = &doc.feature {
        walk_node(&NodePath::feature(), Node::Feature(feature), visitor);
    }
}

fn walk_node<'d>(path: &NodePath, node: Node<'d>, visitor: &mut impl Visitor<'d>) {
    if visitor.enter(path, node) == Walk::Children {
        for (child_path, child) in children(path, node) {
            walk_node(&child_path, child, visitor);
        }
    }
    visitor.exit(path, node);
}
