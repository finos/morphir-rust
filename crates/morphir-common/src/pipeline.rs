//! Transformation Pipeline
//!
//! A composable pipeline for transforming data.

use crate::Result;

pub mod decorators;
pub mod ir;

/// A step in a transformation pipeline.
///
/// Consumes an input of type `In` and produces an output of type `Out`.
pub trait Step {
    type Base;
    type Input;
    type Output;

    fn run(&self, input: Self::Input) -> Result<Self::Output>;
}

/// A pipeline allows chaining steps.
pub struct Pipeline<S> {
    step: S,
}

impl<S> Pipeline<S> {
    pub fn new(step: S) -> Self {
        Self { step }
    }

    pub fn run<I, O>(&self, input: I) -> Result<O>
    where
        S: Step<Input = I, Output = O>,
    {
        self.step.run(input)
    }

    pub fn then<Next>(self, next: Next) -> Pipeline<ChainedStep<S, Next>> {
        Pipeline {
            step: ChainedStep {
                first: self.step,
                second: next,
            },
        }
    }
}

pub struct ChainedStep<A, B> {
    first: A,
    second: B,
}

impl<A, B, I, M, O> Step for ChainedStep<A, B>
where
    A: Step<Input = I, Output = M>,
    B: Step<Input = M, Output = O>,
{
    type Base = (); // Placeholder
    type Input = I;
    type Output = O;

    fn run(&self, input: Self::Input) -> Result<Self::Output> {
        let intermediate = self.first.run(input)?;
        self.second.run(intermediate)
    }
}
