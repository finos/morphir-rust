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
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["--suite", "ir"] => run(io::stdin().lock(), io::stdout())?,
        ["--suite", "node-address"] => {
            morphir_mck_adapter::node_address::run(io::stdin().lock(), io::stdout())?
        }
        ["--suite", "metadata"] => {
            morphir_mck_adapter::metadata::run(io::stdin().lock(), io::stdout())?
        }
        ["--suite", "package"] => {
            morphir_mck_adapter::package::run(io::stdin().lock(), io::stdout())?
        }
        ["--suite", "package", "--contract", "0.1.0-draft.1"] => {
            morphir_mck_adapter::package::run(io::stdin().lock(), io::stdout())?
        }
        ["--suite", "package", "--contract", "0.1.0-draft.2"] => {
            morphir_mck_adapter::package_resolution::run(io::stdin().lock(), io::stdout())?
        }
        ["package-mvp"] => morphir_mck_adapter::package_mvp::run(io::stdin().lock(), io::stdout())?,
        _ => anyhow::bail!(
            "usage: mck-adapter-rust [--suite ir|metadata|package [--contract 0.1.0-draft.1|0.1.0-draft.2]] | package-mvp"
        ),
    }
    Ok(())
}
