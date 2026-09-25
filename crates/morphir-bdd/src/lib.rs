//! Runs Morphir's Gherkin suites on cucumber-rs.
//!
//! One world type ([`MorphirWorld`]) holds a typed context that extensions fill before the
//! first step. Step libraries in any crate write steps against it. A runner ([`Suite`]) runs
//! every suite the same way.

#![warn(missing_docs)]

pub mod diff;
pub mod parser;
pub mod steps;
pub mod suite;
pub mod tags;
pub mod world;

pub use suite::{Suite, SuiteResult, standard_extensions};
pub use world::MorphirWorld;

/// Keeps this crate's base step libraries (files, output, CLI and probe steps) in a test binary.
///
/// cucumber-rs collects steps through link-time registration, so a step defined in a library
/// crate reaches a test binary only if the linker keeps that crate's step statics.
/// [`Suite::run`] already calls this, so a suite run through [`Suite`] needs nothing more. Call it
/// yourself only from a raw cucumber chain (`MorphirWorld::cucumber()…run(…)`) that does not go
/// through [`Suite`]. A separate step-library crate that wants its own steps kept needs its own
/// `link()` in the same way, called from the test binary's `main`.
pub fn link() {
    steps::link();
}
