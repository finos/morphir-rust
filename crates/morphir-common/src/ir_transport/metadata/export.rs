//! Self-contained metadata contexts for a standalone document or an inventory-backed tree.

use super::{
    ContextDigestResolver, ContextRequest, ContextResourceError, ContextResourceLimits,
    load_context_resources_inner,
};
use morphir_core::metadata::{ContextError, ContextResources, EffectiveContext, resolve_context};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Component, Path};

/// Where the exported IR document will live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextOutput {
    /// One file with no accompanying resource directory.
    Standalone,
    /// A document tree whose context resources are supplied beside the IR files.
    DocumentTree,
    /// A package archive whose inventory authenticates its context resources.
    Archive,
}

/// Authored context placement in the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextStorage {
    /// Inline for standalone output, external for a tree or archive.
    Auto,
    /// Resolve every import into its original document scope.
    Inline,
    /// Emit self-contained `.jsonld` files and replace scopes with relative references.
    External,
}

/// One resource for the caller to write into a staging tree and declare in its inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedContextResource {
    /// Confined path from the output root.
    pub path: String,
    /// The fixed JSON-LD context media kind.
    pub media_type: &'static str,
    /// SHA-256 of the exact `bytes` supplied below.
    pub sha256: String,
    /// Complete, self-contained context resource bytes.
    pub bytes: Vec<u8>,
}

/// A rewritten 4.1 document and any resources required to re-read it.
#[derive(Debug, Clone, PartialEq)]
pub struct ContextExport {
    pub document: Value,
    pub resources: Vec<ExportedContextResource>,
}

/// Explicit input and destination boundaries for one context export.
pub struct ContextExportRequest<'a> {
    pub source_root: &'a Path,
    pub document: &'a Value,
    pub source_file: Option<&'a str>,
    pub output_file: Option<&'a str>,
    pub output: ContextOutput,
    pub storage: ContextStorage,
    pub resolver: Option<&'a dyn ContextDigestResolver>,
    pub limits: ContextResourceLimits,
}

/// An unsafe or unresolved export request.
#[derive(Debug, thiserror::Error)]
pub enum ContextExportError {
    #[error("context export requires a formatVersion 4.1.0 document")]
    Version,
    #[error("external contexts require a document-tree destination")]
    ExternalStandalone,
    #[error("external contexts require a confined output document path")]
    Destination,
    #[error(transparent)]
    Resource(#[from] ContextResourceError),
    #[error(transparent)]
    Context(#[from] ContextError),
    #[error("cannot encode resolved context resource: {0}")]
    Encode(#[from] serde_json::Error),
}

/// Resolve every authored context and choose inline or external output.
///
/// `source_root` is the explicit authoring tree; `source_file` is the original
/// IR file's relative path, used only for relative imports. `output_file` is
/// the IR file's path in the destination tree or archive. The returned files
/// must be written beside that file and, for archives, admitted by the signed
/// package inventory before publication. This function never fetches a URL or
/// mutates either directory. Its inline result can be passed to the normal
/// JSON/YAML/Ion 4.1 codec without the authoring tree.
pub fn export_contexts(
    request: ContextExportRequest<'_>,
) -> Result<ContextExport, ContextExportError> {
    let ContextExportRequest {
        source_root,
        document,
        source_file,
        output_file,
        output,
        storage,
        resolver,
        limits,
    } = request;
    if document.get("formatVersion").and_then(Value::as_str) != Some("4.1.0") {
        return Err(ContextExportError::Version);
    }
    let storage = match (storage, output) {
        (ContextStorage::Auto, ContextOutput::Standalone) => ContextStorage::Inline,
        (ContextStorage::Auto, _) => ContextStorage::External,
        (ContextStorage::External, ContextOutput::Standalone) => {
            return Err(ContextExportError::ExternalStandalone);
        }
        (selected, _) => selected,
    };
    let output_file = if storage == ContextStorage::External {
        Some(
            output_file
                .filter(|path| valid_output_path(path))
                .ok_or(ContextExportError::Destination)?,
        )
    } else {
        None
    };

    let mut requests = Vec::new();
    if let Some(context) = document.get("$meta").and_then(|meta| meta.get("@context")) {
        requests.push(ContextRequest {
            authored: context,
            base_file: source_file,
        });
    }
    collect_scoped_contexts(document, source_file, &mut requests);
    // A local carrier can use an inherited document prefix, so validation of
    // each request is deferred until it is resolved over that document scope.
    let resources = load_context_resources_inner(source_root, &requests, resolver, limits, false)?;
    let mut rewritten = document.clone();
    let mut emitted = BTreeMap::new();
    let parent = if let Some(meta) = rewritten.get_mut("$meta") {
        if let Some(context) = meta.get_mut("@context") {
            let effective = resolve_context(None, context, &resources, source_file)?;
            *context = place_context(&effective, storage, output_file, &mut emitted)?;
            effective
        } else {
            EffectiveContext::default()
        }
    } else {
        EffectiveContext::default()
    };
    rewrite_scoped_contexts(
        &mut rewritten,
        &parent,
        &resources,
        source_file,
        storage,
        output_file,
        &mut emitted,
    )?;
    if emitted.len() > limits.resource_count {
        return Err(ContextResourceError::ResourceCountExceeded.into());
    }
    let mut total_bytes = 0usize;
    for file in emitted.values() {
        if file.bytes.len() > limits.per_resource_bytes {
            return Err(ContextResourceError::ResourceBytesExceeded(file.path.clone()).into());
        }
        total_bytes = total_bytes
            .checked_add(file.bytes.len())
            .ok_or(ContextResourceError::TotalBytesExceeded)?;
        if total_bytes > limits.total_bytes {
            return Err(ContextResourceError::TotalBytesExceeded.into());
        }
    }
    Ok(ContextExport {
        document: rewritten,
        resources: emitted.into_values().collect(),
    })
}

fn collect_scoped_contexts<'a>(
    value: &'a Value,
    source_file: Option<&'a str>,
    requests: &mut Vec<ContextRequest<'a>>,
) {
    match value {
        Value::Object(object) => {
            for (name, child) in object {
                if excluded_child(name) {
                    continue;
                }
                if matches!(name.as_str(), "attributes" | "annotations")
                    && let Some(context) = child.get("@context")
                {
                    requests.push(ContextRequest {
                        authored: context,
                        base_file: source_file,
                    });
                }
                collect_scoped_contexts(child, source_file, requests);
            }
        }
        Value::Array(items) => {
            for child in items {
                collect_scoped_contexts(child, source_file, requests);
            }
        }
        _ => {}
    }
}

fn rewrite_scoped_contexts(
    value: &mut Value,
    parent: &EffectiveContext,
    resources: &ContextResources,
    source_file: Option<&str>,
    storage: ContextStorage,
    output_file: Option<&str>,
    emitted: &mut BTreeMap<String, ExportedContextResource>,
) -> Result<(), ContextExportError> {
    match value {
        Value::Object(object) => {
            for (name, child) in object {
                if excluded_child(name) {
                    continue;
                }
                if matches!(name.as_str(), "attributes" | "annotations")
                    && let Some(context) = child.get_mut("@context")
                {
                    let effective = resolve_context(Some(parent), context, resources, source_file)?;
                    *context = place_context(&effective, storage, output_file, emitted)?;
                }
                rewrite_scoped_contexts(
                    child,
                    parent,
                    resources,
                    source_file,
                    storage,
                    output_file,
                    emitted,
                )?;
            }
        }
        Value::Array(items) => {
            for child in items {
                rewrite_scoped_contexts(
                    child,
                    parent,
                    resources,
                    source_file,
                    storage,
                    output_file,
                    emitted,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn excluded_child(name: &str) -> bool {
    matches!(
        name,
        "$meta" | "@context" | "facts" | "@graph" | "assertionSources" | "extensions"
    )
}

fn place_context(
    context: &EffectiveContext,
    storage: ContextStorage,
    output_file: Option<&str>,
    emitted: &mut BTreeMap<String, ExportedContextResource>,
) -> Result<Value, ContextExportError> {
    let inline = context.to_inline_value();
    if storage == ContextStorage::Inline {
        return Ok(inline);
    }
    let bytes = serde_json::to_vec(&json!({"@context": inline}))?;
    let digest = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let path = format!("contexts/sha256-{digest}.jsonld");
    emitted
        .entry(path.clone())
        .or_insert_with(|| ExportedContextResource {
            path: path.clone(),
            media_type: "application/ld+json",
            sha256: digest,
            bytes,
        });
    let parent_count = Path::new(output_file.expect("external storage checked output path"))
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .count();
    Ok(Value::String(format!(
        "{}{path}",
        "../".repeat(parent_count)
    )))
}

fn valid_output_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        && Path::new(path).file_name().is_some()
}
