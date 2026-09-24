//! Draft node-address adapter suite. It indexes real V3/V4 IR in the testee;
//! the shared MCK driver compares fixed expectations without using an IR codec.

use anyhow::{Result, anyhow};
use morphir_core::ir::classic;
use morphir_core::naming::{PackageName, Path};
use morphir_core::node_address::{
    ArtifactSelector, NodeCatalog, NodeIndex, NodeResolutionError, NodeUri,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

pub const CONTRACT: &str = "0.1.0-draft.1";

#[derive(Deserialize)]
#[serde(tag = "op", deny_unknown_fields)]
enum Request {
    #[serde(rename = "capabilities")]
    Capabilities {},
    #[serde(rename = "resolve")]
    Resolve { input: String, uri: String },
    #[serde(rename = "exit")]
    Exit {},
}

pub fn run(reader: impl BufRead, mut writer: impl Write) -> Result<()> {
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let mut value = morphir_core::ir::json::read(&line)
            .map_err(|error| anyhow!("invalid node-address request: {error:?}"))?;
        let id = value
            .as_object_mut()
            .and_then(|object| object.remove("id"))
            .filter(super::package::positive_integer_id)
            .ok_or_else(|| anyhow!("node-address request id must be a positive integer"))?;
        let request: Request = serde_json::from_value(value)?;
        let mut response = match request {
            Request::Capabilities {} => {
                json!({"suite":"node-address","contractVersion":CONTRACT,"implementation":"morphir-rust","implementationVersion":env!("CARGO_PKG_VERSION"),"operations":["resolve"]})
            }
            Request::Resolve { input, uri } => resolve(&input, &uri),
            Request::Exit {} => break,
        };
        response
            .as_object_mut()
            .expect("response object")
            .insert("id".into(), id);
        writeln!(writer, "{response}")?;
        writer.flush()?;
    }
    Ok(())
}

fn resolve(input: &str, uri: &str) -> Value {
    let uri = match NodeUri::parse(uri) {
        Ok(uri) => uri,
        Err(_) => return json!({"ok":false,"outcome":"invalid_node_uri"}),
    };
    let value = match morphir_core::ir::json::read(input) {
        Ok(value) => value,
        Err(_) => return json!({"ok":false,"outcome":"invalid_artifact"}),
    };
    let version = value.get("formatVersion");
    let mut catalog = NodeCatalog::new();
    let result = match version {
        Some(Value::Number(number)) if number.as_u64() == Some(3) => {
            add_v3(&mut catalog, input, value)
        }
        Some(Value::String(text)) if text == "3.0.0" => add_v3(&mut catalog, input, value),
        Some(Value::Number(number)) if number.as_u64() == Some(4) => add_v4(&mut catalog, input),
        Some(Value::String(text)) if text == "4.0.0" => add_v4(&mut catalog, input),
        _ => return json!({"ok":false,"outcome":"format_version_mismatch"}),
    };
    if result.is_err() {
        return json!({"ok":false,"outcome":"invalid_artifact"});
    }
    match catalog.resolve_node(&uri) {
        Ok(node) => {
            json!({"ok":true,"outcome":"resolved","kind":format!("{:?}", node.kind),"canonicalUri":uri.to_string(),"node":node.semantic_value})
        }
        Err(error) => json!({"ok":false,"outcome":outcome(&error)}),
    }
}

fn add_v3(catalog: &mut NodeCatalog, input: &str, value: Value) -> Result<()> {
    let distribution: classic::Distribution = serde_json::from_value(value)?;
    let classic::DistributionBody::Library(package, _, _) = &distribution.distribution else {
        anyhow::bail!("a v3 Specs distribution has no definitions to address");
    };
    let package = package
        .segments
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("/");
    let index = NodeIndex::v3(
        &distribution,
        ArtifactSelector::Package(PackageName::new(Path::new(&package))),
    )?;
    catalog.add_current(index);
    catalog.add_v3_json_snapshot(input.as_bytes(), None)?;
    Ok(())
}

fn add_v4(catalog: &mut NodeCatalog, input: &str) -> Result<()> {
    let (file, _) =
        morphir_core::ir::json::read_ir_file(input).map_err(|error| anyhow!("{error}"))?;
    catalog.add_current(NodeIndex::v4_file(&file)?);
    catalog.add_v4_json_snapshot(input.as_bytes(), None)?;
    Ok(())
}

fn outcome(error: &NodeResolutionError) -> &'static str {
    match error {
        NodeResolutionError::ArtifactMismatch => "artifact_mismatch",
        NodeResolutionError::AmbiguousArtifact => "ambiguous_artifact",
        NodeResolutionError::RevisionUnavailable => "revision_unavailable",
        NodeResolutionError::RevisionMismatch => "revision_mismatch",
        NodeResolutionError::FormatVersionMismatch => "format_version_mismatch",
        NodeResolutionError::StaleTarget => "stale_target",
        NodeResolutionError::AmbiguousTarget => "ambiguous_target",
        NodeResolutionError::InvalidName(_)
        | NodeResolutionError::InvalidFingerprint(_)
        | NodeResolutionError::InvalidSnapshot(_)
        | NodeResolutionError::UnsupportedDistribution => "invalid_artifact",
    }
}
