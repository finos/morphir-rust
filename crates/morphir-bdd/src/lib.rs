//! Runs Morphir's Gherkin suites on cucumber-rs.
//!
//! One world type ([`world::MorphirWorld`]) holds a typed context that extensions fill before the
//! first step. Step libraries in any crate write steps against it. A runner runs every suite the
//! same way.

pub mod parser;
pub mod steps;
pub mod suite;
pub mod tags;
pub mod world;

/// Keeps the base step libraries in a test binary.
pub fn link() {
    steps::link();
}
