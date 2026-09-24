//! Dump a built WASM extension's own capability claims.

use morphir_extension_sdk::claims::CapabilityClaimSet;
use morphir_extension_sdk::protocol::{DescribeParams, SUPPORTED_MEP_VERSIONS, methods};
use morphir_host_native::extism::{ExtensionContainer, MorphirHostFunctions};
use serde_json::Value;
use std::error::Error;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("extension-claims: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().map(PathBuf::from);
    let path = match (path, args.next()) {
        (Some(path), None) => path,
        _ => return Err("Usage: extension-claims <path/to/guest.wasm>".into()),
    };
    let container =
        ExtensionContainer::new("extension-claims", &path, MorphirHostFunctions::default())
            .map_err(|error| format!("cannot load {}: {error}", path.display()))?;
    let claims: Value = container
        .call(methods::DESCRIBE, describe_params())
        .await
        .map_err(|error| format!("{} describe failed: {error}", path.display()))?;
    let claims = validate_claims(claims).map_err(|error| {
        format!(
            "invalid describe claim set from {}: {error}",
            path.display()
        )
    })?;
    let mut stdout = io::stdout().lock();
    serde_json::to_writer_pretty(&mut stdout, &claims)?;
    writeln!(stdout)?;
    Ok(())
}

fn describe_params() -> DescribeParams {
    DescribeParams {
        protocol_versions: SUPPORTED_MEP_VERSIONS
            .iter()
            .map(|version| (*version).into())
            .collect(),
    }
}

fn validate_claims(value: Value) -> Result<Value, serde_json::Error> {
    // Validate with the SDK, but keep the original members: serializing the
    // typed value could normalize versions or drop unknown optional fields.
    let _: CapabilityClaimSet = serde_json::from_value(value.clone())?;
    Ok(value)
}

#[cfg(test)]
mod tests;
