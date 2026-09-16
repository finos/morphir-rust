//! `mck-adapter-rust`: the testee side of the Morphir Compatibility Kit's
//! JSON-lines adapter protocol for morphir-rust (`protocol.schema.json`
//! contract version 1; see `protocol.example.json` for a worked exchange).
//!
//! The framing loop itself lives in
//! [`morphir_mck_adapter::runtime`] so it can be driven against stdin/stdout
//! here and against an in-memory buffer in tests.

use anyhow::Result;
use morphir_mck_adapter::runtime::run;
use std::io;

fn main() -> Result<()> {
    run(io::stdin().lock(), io::stdout())?;
    Ok(())
}
