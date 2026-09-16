//! The Morphir Compatibility Kit adapter for morphir-rust.
//!
//! This crate implements the testee side of the kit's JSON-lines adapter
//! protocol (`spec/ir/mck/protocol.schema.json` in the parent repository):
//! [`protocol`] carries the wire types and per-line parsing, [`runtime`]
//! carries the framing loop that drives them over a reader and writer, and
//! [`testee`] will carry the operations that answer each request once they
//! exist.

pub mod package;
pub mod protocol;
pub mod runtime;
pub mod testee;
