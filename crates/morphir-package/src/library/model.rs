//! Private decoded boundary models; schemas and semantic checks validate them.

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Manifest {
    pub package_path: String,
    pub version: String,
    pub ir: Ir,
    pub dependencies: BTreeMap<String, Requirement>,
    pub exports: BTreeMap<String, String>,
    pub content: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Ir {
    pub package_name: String,
    pub payload: Payload,
}

#[derive(Deserialize)]
pub(super) struct Payload {
    pub path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Requirement {
    pub package_path: String,
    pub version_range: VersionRange,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct VersionRange {
    pub minimum_inclusive: String,
    pub maximum_exclusive: String,
}

#[derive(Deserialize)]
pub(super) struct Lock {
    pub root: String,
    pub nodes: BTreeMap<String, Node>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Node {
    pub release: Release,
    pub ir_package_name: String,
    pub manifest_digest: String,
    pub content_digest: String,
    pub bindings: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Release {
    pub package_path: String,
    pub version: String,
}
