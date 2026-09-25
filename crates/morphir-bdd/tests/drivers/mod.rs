//! Test drivers for `morphir-bdd`'s own BDD acceptance scenarios (see AGENTS.md's "BDD Test
//! Drivers" rule).
//!
//! `tests/bdd.rs` and `tests/suite.rs` each include this module with `mod drivers;`, but neither
//! binary calls every domain method [`suite_driver::SuiteDriver`] offers. Under `-D warnings` a
//! method one binary never calls would fail that binary's own build with a `dead_code` warning,
//! so the allow lives here, once, for the whole module, rather than on every individual method.
#![allow(dead_code)]

pub mod suite_driver;
