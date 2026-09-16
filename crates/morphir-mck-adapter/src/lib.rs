//! The Morphir Compatibility Kit adapter for morphir-rust.
//!
//! This crate implements the testee side of the kit's JSON-lines adapter
//! protocol (`spec/ir/mck/protocol.schema.json` in the parent repository):
//! [`protocol`] carries the wire types and the line-framing entry point,
//! and [`testee`] will carry the operations that answer each request once
//! they exist.

pub mod protocol;
pub mod testee;
