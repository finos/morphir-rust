//! Navigation by path, by source offset, and by cursor moves.

use crate::model::{Document, StepArgument};
use crate::path::{NodePath, Segment};
use crate::span::Span;
use crate::visit::{Node, children, preamble_children};

/// Whether a byte offset falls inside a span. A zero-width span still contains its own start.
fn contains(span: Span, offset: usize) -> bool {
    span.start <= offset && offset < span.end.max(span.start + 1)
}

fn join(a: Span, b: Span) -> Span {
    Span {
        start: a.start.min(b.start),
        end: a.end.max(b.end),
    }
}

/// The extent of a node: its own span, joined with its argument's span (a step's doc string or
/// table) and the extent of each of its children. A step's own span covers only its line, so
/// `Document::at` descends by extent, not by span, to find the node that owns an offset inside a
/// step's argument or notes.
fn extent(path: &NodePath, node: Node<'_>) -> Span {
    let mut span = node.span();
    if let Node::Step(step) = node
        && let Some(argument) = &step.argument
    {
        let argument_span = match argument {
            StepArgument::DocString(doc_string) => doc_string.span,
            StepArgument::Table(table) => table.span,
        };
        span = join(span, argument_span);
    }
    for (child_path, child) in children(path, node) {
        span = join(span, extent(&child_path, child));
    }
    span
}

impl Document {
    /// The node at a path, if the path names one. `preamble` alone names no node; its children do.
    pub fn node(&self, path: &NodePath) -> Option<Node<'_>> {
        match path.segments().first()? {
            Segment::Preamble => preamble_children(self)
                .into_iter()
                .find(|(p, _)| p == path)
                .map(|(_, node)| node),
            Segment::Feature => {
                let feature = self.feature.as_ref()?;
                let mut current = (NodePath::feature(), Node::Feature(feature));
                for depth in 2..=path.segments().len() {
                    let wanted = NodePath::from_segments(&path.segments()[..depth]);
                    current = children(&current.0, current.1)
                        .into_iter()
                        .find(|(p, _)| *p == wanted)?;
                }
                Some(current.1)
            }
            _ => None,
        }
    }

    /// The deepest node whose extent contains a byte offset. The preamble's blocks are tried
    /// before the feature, since they come first in the source. An offset inside a step's doc
    /// string or table resolves to the step; inside a step's notes, to the note block; in the gap
    /// between two steps, to the enclosing scenario or background.
    pub fn at(&self, offset: usize) -> Option<NodePath> {
        if let Some((path, _)) = preamble_children(self)
            .into_iter()
            .find(|(_, node)| contains(node.span(), offset))
        {
            return Some(path);
        }
        let feature = self.feature.as_ref()?;
        let mut current = (NodePath::feature(), Node::Feature(feature));
        if !contains(extent(&current.0, current.1), offset) {
            return None;
        }
        while let Some(next) = children(&current.0, current.1)
            .into_iter()
            .find(|(path, node)| contains(extent(path, *node), offset))
        {
            current = next;
        }
        Some(current.0)
    }

    /// A cursor at the document's feature.
    pub fn cursor(&self) -> Cursor<'_> {
        Cursor {
            doc: self,
            path: NodePath::feature(),
        }
    }

    /// A cursor at the first block of the document's preamble, or `None` when the preamble is
    /// empty.
    pub fn preamble_cursor(&self) -> Option<Cursor<'_>> {
        let (path, _) = preamble_children(self).into_iter().next()?;
        Some(Cursor { doc: self, path })
    }
}

#[derive(Debug, Clone)]
pub struct Cursor<'d> {
    doc: &'d Document,
    path: NodePath,
}

impl<'d> Cursor<'d> {
    pub fn path(&self) -> &NodePath {
        &self.path
    }

    pub fn node(&self) -> Node<'d> {
        self.doc
            .node(&self.path)
            .expect("a cursor always points at a node")
    }

    /// Whether this cursor is at a block of the preamble. Those blocks have no parent: the
    /// preamble itself is not a node, so a preamble block is a root like the feature.
    fn is_preamble_block(&self) -> bool {
        matches!(self.path.segments().first(), Some(Segment::Preamble))
    }

    pub fn parent(&self) -> Option<Cursor<'d>> {
        if self.is_preamble_block() {
            return None;
        }
        self.path.parent().map(|path| Cursor {
            doc: self.doc,
            path,
        })
    }

    pub fn children(&self) -> Vec<Cursor<'d>> {
        children(&self.path, self.node())
            .into_iter()
            .map(|(path, _)| Cursor {
                doc: self.doc,
                path,
            })
            .collect()
    }

    fn siblings(&self) -> Vec<Cursor<'d>> {
        if self.is_preamble_block() {
            return preamble_children(self.doc)
                .into_iter()
                .map(|(path, _)| Cursor {
                    doc: self.doc,
                    path,
                })
                .collect();
        }
        self.parent().map(|p| p.children()).unwrap_or_default()
    }

    pub fn next_sibling(&self) -> Option<Cursor<'d>> {
        let siblings = self.siblings();
        let at = siblings.iter().position(|s| s.path == self.path)?;
        siblings.into_iter().nth(at + 1)
    }

    pub fn previous_sibling(&self) -> Option<Cursor<'d>> {
        let siblings = self.siblings();
        let at = siblings.iter().position(|s| s.path == self.path)?;
        at.checked_sub(1).and_then(|i| siblings.into_iter().nth(i))
    }
}
