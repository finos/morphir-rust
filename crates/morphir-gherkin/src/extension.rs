//! Extensions read tags, free fences and prose into a typed context, from the outside in: feature,
//! then rule, then scenario, then the scenario's examples. A later scope overrides an earlier one
//! because it inserts later.
//!
//! Extensions read the description of the feature, the rule, the scenario and the examples only.
//! The document's preamble, a step's notes and an examples block's notes are not fed to these
//! extensions in this crate. A later requirements extension can reach those through the visitor.

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::fmt;

use crate::model::{Description, DescriptionBlock, Document, Fence, ProseBlock, Tag};
use crate::path::{NodePath, Segment};
use crate::span::Span;
use crate::visit::Node;

/// A value an extension can store in a `Context`: any owned, thread-safe, debuggable type.
pub trait Component: Any + Send + Sync + fmt::Debug {}
impl<T: Any + Send + Sync + fmt::Debug> Component for T {}

/// A typed bag of values built by extensions. Each type stores at most one value; a later insert
/// of the same type replaces the earlier one.
#[derive(Debug, Default)]
pub struct Context {
    components: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Context {
    /// Stores `value`, replacing any value of the same type already stored.
    pub fn insert<T: Component>(&mut self, value: T) {
        self.components.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// The stored value of type `T`, if one has been inserted.
    pub fn get<T: Component>(&self) -> Option<&T> {
        self.components
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref())
    }

    /// A mutable reference to the stored value of type `T`, if one has been inserted.
    pub fn get_mut<T: Component>(&mut self) -> Option<&mut T> {
        self.components
            .get_mut(&TypeId::of::<T>())
            .and_then(|value| value.downcast_mut())
    }

    /// Removes and returns the stored value of type `T`, if one has been inserted.
    pub fn remove<T: Component>(&mut self) -> Option<T> {
        self.components
            .remove(&TypeId::of::<T>())
            .and_then(|value| value.downcast().ok())
            .map(|value| *value)
    }
}

/// The scope an extension is applying at: which kind of node owns the tag, fence or prose block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// A feature's own tag, fence or prose block.
    Feature,
    /// A rule's own tag, fence or prose block.
    Rule,
    /// A scenario's own tag, fence or prose block.
    Scenario,
    /// An examples block's own tag, fence or prose block.
    Examples,
}

/// The outcome of building a context: run the scenario, or skip it with a reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Run the scenario.
    Continue,
    /// Skip the scenario, with a human-readable reason.
    Skip(String),
}

/// An error raised while building a context. It always names the node path and the span of the
/// tag, fence or prose block that caused it, so the error is never silent about where it came
/// from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionError {
    /// The path of the node whose tag, fence or prose block caused the error.
    pub path: NodePath,
    /// The span of the tag, fence or prose block that caused the error.
    pub span: Span,
    /// A human-readable description of what went wrong.
    pub message: String,
}

impl fmt::Display for ExtensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// Reads tags into the context. An extension that owns a namespace (`syntax` for `@syntax:elm`)
/// always receives a tag in that namespace; an unknown value is then an error, not a label. An
/// extension can also claim unnamespaced tags through `matches`. A tag no extension claims stays
/// a plain label. When more than one extension matches a plain tag, the first one registered
/// handles it.
pub trait TagExtension: Send + Sync {
    /// The namespace this extension owns, if any.
    fn namespace(&self) -> Option<&str>;
    /// Whether this extension handles a tag outside any namespace it owns.
    fn matches(&self, _tag: &Tag) -> bool {
        false
    }
    /// Reads `tag` into `ctx`. Returns `Effect::Skip` to skip the scenario, or an error message
    /// naming what was wrong with the tag.
    fn apply(&self, tag: &Tag, scope: Scope, ctx: &mut Context) -> Result<Effect, String>;
}

/// Reads a free fence (a fenced block that is not a step's doc string) into the context. When
/// more than one extension matches a fence, the first one registered handles it.
pub trait FenceExtension: Send + Sync {
    /// Whether this extension reads `fence`.
    fn matches(&self, fence: &Fence) -> bool;
    /// Reads `fence` into `ctx`. Returns an error message naming what was wrong with it.
    fn apply(&self, fence: &Fence, scope: Scope, ctx: &mut Context) -> Result<(), String>;
}

/// Reads a prose block into the context. Every prose block goes to every prose extension; an
/// extension decides for itself whether a block matters.
pub trait ProseExtension: Send + Sync {
    /// Reads `prose` into `ctx`, if it matters to this extension. Returns an error message
    /// naming what was wrong with it.
    fn apply(&self, prose: &ProseBlock, scope: Scope, ctx: &mut Context) -> Result<(), String>;
}

/// Runs after every scope has been applied, to derive further context from the whole document.
pub trait Processor: Send + Sync {
    /// Derives further context for the scenario at `at`. Returns an error message naming what
    /// went wrong.
    fn process(&self, doc: &Document, at: &NodePath, ctx: &mut Context) -> Result<(), String>;
}

/// The extensions registered for a document: tag, fence and prose readers, and processors that
/// run once the context is built.
#[derive(Default)]
pub struct Extensions {
    tags: Vec<Box<dyn TagExtension>>,
    fences: Vec<Box<dyn FenceExtension>>,
    prose: Vec<Box<dyn ProseExtension>>,
    processors: Vec<Box<dyn Processor>>,
}

impl Extensions {
    /// An empty set of extensions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a tag extension. When more than one extension matches a plain tag, the first
    /// one registered handles it.
    #[must_use]
    pub fn with_tags(mut self, extension: impl TagExtension + 'static) -> Self {
        self.tags.push(Box::new(extension));
        self
    }

    /// Registers a fence extension. When more than one extension matches a fence, the first one
    /// registered handles it.
    #[must_use]
    pub fn with_fences(mut self, extension: impl FenceExtension + 'static) -> Self {
        self.fences.push(Box::new(extension));
        self
    }

    /// Registers a prose extension. Every prose block is offered to every registered prose
    /// extension.
    #[must_use]
    pub fn with_prose(mut self, extension: impl ProseExtension + 'static) -> Self {
        self.prose.push(Box::new(extension));
        self
    }

    /// Registers a processor, to run once a scenario's context is built from its tags, fences
    /// and prose.
    #[must_use]
    pub fn with_processor(mut self, processor: impl Processor + 'static) -> Self {
        self.processors.push(Box::new(processor));
        self
    }

    /// Builds the context of one scenario. It applies the feature scope, then the rule scope
    /// (when the scenario is under a rule), then the scenario scope, then each of the scenario's
    /// examples, then the processors. A skip from any scope wins over a continue from another.
    ///
    /// Only the description of the feature, the rule, the scenario and the examples is read. The
    /// document's preamble, a step's notes and an examples block's notes are not read here.
    ///
    /// `scenario` must name a scenario. A path that names a rule, a feature or anything else
    /// fails with `"not a scenario"`.
    pub fn context_for(
        &self,
        doc: &Document,
        scenario: &NodePath,
    ) -> Result<(Context, Effect), Vec<ExtensionError>> {
        match doc.node(scenario) {
            Some(Node::Scenario(_)) => {}
            other => {
                let span = other.map(|node| node.span()).unwrap_or_default();
                return Err(vec![ExtensionError {
                    path: scenario.clone(),
                    span,
                    message: "not a scenario".to_owned(),
                }]);
            }
        }
        let mut ctx = Context::default();
        let mut effect = Effect::Continue;
        let mut errors = Vec::new();
        let segments = scenario.segments();
        for depth in 1..=segments.len() {
            let path = NodePath::from_segments(&segments[..depth]);
            let Some(node) = doc.node(&path) else {
                errors.push(ExtensionError {
                    path,
                    span: Span::default(),
                    message: "no such node".to_owned(),
                });
                return Err(errors);
            };
            let (scope, tags, description): (Scope, &[Tag], &Description) = match node {
                Node::Feature(f) => (Scope::Feature, &f.tags, &f.description),
                Node::Rule(r) => (Scope::Rule, &r.tags, &r.description),
                Node::Scenario(s) => (Scope::Scenario, &s.tags, &s.description),
                _ => continue,
            };
            self.apply_scope(
                &path,
                scope,
                tags,
                description,
                &mut ctx,
                &mut effect,
                &mut errors,
            );
            if let Node::Scenario(s) = node {
                for (i, examples) in s.examples.iter().enumerate() {
                    let path = path.push(Segment::Examples(i));
                    self.apply_scope(
                        &path,
                        Scope::Examples,
                        &examples.tags,
                        &examples.description,
                        &mut ctx,
                        &mut effect,
                        &mut errors,
                    );
                }
            }
        }
        for processor in &self.processors {
            if let Err(message) = processor.process(doc, scenario, &mut ctx) {
                errors.push(ExtensionError {
                    path: scenario.clone(),
                    span: Span::default(),
                    message,
                });
            }
        }
        if errors.is_empty() {
            Ok((ctx, effect))
        } else {
            Err(errors)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_scope(
        &self,
        path: &NodePath,
        scope: Scope,
        tags: &[Tag],
        description: &Description,
        ctx: &mut Context,
        effect: &mut Effect,
        errors: &mut Vec<ExtensionError>,
    ) {
        for tag in tags {
            let owner = tag
                .namespaced()
                .and_then(|(ns, _)| self.tags.iter().find(|e| e.namespace() == Some(ns)));
            let handler = owner.or_else(|| self.tags.iter().find(|e| e.matches(tag)));
            if let Some(handler) = handler {
                match handler.apply(tag, scope, ctx) {
                    Ok(Effect::Skip(reason)) => *effect = Effect::Skip(reason),
                    Ok(Effect::Continue) => {}
                    Err(message) => errors.push(ExtensionError {
                        path: path.clone(),
                        span: tag.span,
                        message,
                    }),
                }
            }
        }
        for block in &description.blocks {
            match block {
                DescriptionBlock::Fence(fence) => {
                    if let Some(handler) = self.fences.iter().find(|e| e.matches(fence))
                        && let Err(message) = handler.apply(fence, scope, ctx)
                    {
                        errors.push(ExtensionError {
                            path: path.clone(),
                            span: fence.span,
                            message,
                        });
                    }
                }
                DescriptionBlock::Prose(prose) => {
                    for handler in &self.prose {
                        if let Err(message) = handler.apply(prose, scope, ctx) {
                            errors.push(ExtensionError {
                                path: path.clone(),
                                span: prose.span,
                                message,
                            });
                        }
                    }
                }
            }
        }
    }
}
