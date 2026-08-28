//! Contracts for composable semantic IR transforms.

use morphir_core::traversal::{IrCursor, SemanticEvent};

use super::{EventSink, EventSource, Stage, TransportDiagnostic};

/// Largest semantic scope retained by a transform.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Retention {
    /// Retains no context beyond the current event.
    Event,
    /// Retains one type or value definition.
    Definition,
    /// Retains one module.
    Module,
    /// Retains the complete distribution.
    Distribution,
}

/// Semantic rewrite that is independent of JSON, YAML, and physical layout.
pub trait EventTransform {
    /// Declare the largest semantic scope retained by this transform.
    fn retention(&self) -> Retention;

    /// Transform one input into zero, one, or many output events.
    fn transform(
        &mut self,
        event: SemanticEvent,
        emit: &mut dyn FnMut(SemanticEvent) -> Result<(), TransportDiagnostic>,
    ) -> Result<(), TransportDiagnostic>;

    /// Emit any buffered events after the source ends.
    fn finish(
        &mut self,
        _emit: &mut dyn FnMut(SemanticEvent) -> Result<(), TransportDiagnostic>,
    ) -> Result<(), TransportDiagnostic> {
        Ok(())
    }
}

/// Ordered composition of format-neutral semantic transforms.
#[derive(Default)]
pub struct Pipeline {
    transforms: Vec<Box<dyn EventTransform>>,
}

impl Pipeline {
    /// Create an empty identity pipeline.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a transform and return the updated pipeline.
    pub fn with_transform(mut self, transform: impl EventTransform + 'static) -> Self {
        self.transforms.push(Box::new(transform));
        self
    }

    /// Append a transform in place.
    pub fn push(&mut self, transform: impl EventTransform + 'static) {
        self.transforms.push(Box::new(transform));
    }

    /// Return the largest retention scope declared by any transform.
    pub fn retention(&self) -> Retention {
        self.transforms
            .iter()
            .map(|transform| transform.retention())
            .max()
            .unwrap_or(Retention::Event)
    }

    /// Reject a pipeline that requires whole-distribution buffering.
    pub fn require_bounded(&self) -> Result<(), TransportDiagnostic> {
        if self.retention() == Retention::Distribution {
            return Err(TransportDiagnostic::error(
                "morphir::ir::pipeline::whole_distribution_required",
                Stage::Transformation,
                IrCursor::root(),
                "a transform requires retaining the complete IR distribution",
            )
            .with_guidance(
                "remove the whole-distribution transform or explicitly allow unbounded execution",
            ));
        }
        Ok(())
    }

    /// Stream all source events through the transform chain into a sink.
    pub fn run(
        &mut self,
        source: &mut dyn EventSource,
        sink: &mut dyn EventSink,
    ) -> Result<(), TransportDiagnostic> {
        let mut pipeline_sink = self.sink(sink)?;
        while let Some(event) = source.next_event()? {
            pipeline_sink.accept(event)?;
        }
        pipeline_sink.finish()
    }

    /// Wrap a downstream sink with this transform chain.
    pub fn sink<'pipeline, 'sink>(
        &'pipeline mut self,
        downstream: &'sink mut dyn EventSink,
    ) -> Result<PipelineSink<'pipeline, 'sink>, TransportDiagnostic> {
        self.require_bounded()?;
        Ok(PipelineSink {
            transforms: &mut self.transforms,
            downstream,
            finished: false,
        })
    }
}

/// Push-based view of a pipeline used directly by streaming decoders.
pub struct PipelineSink<'pipeline, 'sink> {
    transforms: &'pipeline mut [Box<dyn EventTransform>],
    downstream: &'sink mut dyn EventSink,
    finished: bool,
}

impl EventSink for PipelineSink<'_, '_> {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        if self.finished {
            return Err(TransportDiagnostic::error(
                "morphir::ir::pipeline::event_after_finish",
                Stage::Transformation,
                event.cursor().clone(),
                "the decoder emitted an event after finishing the pipeline",
            )
            .with_guidance("create a new pipeline sink for each codec operation"));
        }
        forward(self.transforms, event, self.downstream)
    }

    fn finish(&mut self) -> Result<(), TransportDiagnostic> {
        if self.finished {
            return Ok(());
        }
        finish_transforms(self.transforms, self.downstream)?;
        self.downstream.finish()?;
        self.finished = true;
        Ok(())
    }
}

fn forward(
    transforms: &mut [Box<dyn EventTransform>],
    event: SemanticEvent,
    sink: &mut dyn EventSink,
) -> Result<(), TransportDiagnostic> {
    let Some((transform, remaining)) = transforms.split_first_mut() else {
        return sink.accept(event);
    };
    transform.transform(event, &mut |event| forward(remaining, event, sink))
}

fn finish_transforms(
    transforms: &mut [Box<dyn EventTransform>],
    sink: &mut dyn EventSink,
) -> Result<(), TransportDiagnostic> {
    let Some((transform, remaining)) = transforms.split_first_mut() else {
        return Ok(());
    };
    transform.finish(&mut |event| forward(remaining, event, sink))?;
    finish_transforms(remaining, sink)
}
