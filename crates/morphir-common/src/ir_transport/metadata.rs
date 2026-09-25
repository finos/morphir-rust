//! Explicit, bounded acquisition of authored metadata context resources.
//!
//! The returned bytes are handed to the pure `morphir-core` resolver. This
//! loader only reads paths below a caller-selected tree or archive directory,
//! or asks a caller-supplied digest resolver. It never performs network I/O.

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use morphir_core::metadata::{ContextError, ContextResources, resolve_context};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const DIGEST_PREFIX: &str = "morphir://context/sha256/";

/// One authored context and the containing local document, if it has one.
#[derive(Debug, Clone, Copy)]
pub struct ContextRequest<'a> {
    /// The authored `@context` value.
    pub authored: &'a Value,
    /// A path relative to the supplied root, used for local child imports.
    pub base_file: Option<&'a str>,
}

impl<'a> ContextRequest<'a> {
    /// Resolve imports at the selected root.
    pub fn at_root(authored: &'a Value) -> Self {
        Self {
            authored,
            base_file: None,
        }
    }

    /// Resolve imports relative to a document inside the selected root.
    pub fn in_file(authored: &'a Value, base_file: &'a str) -> Self {
        Self {
            authored,
            base_file: Some(base_file),
        }
    }
}

/// Limits for one context closure load. All byte limits concern raw bytes.
#[derive(Debug, Clone, Copy)]
pub struct ContextResourceLimits {
    /// Maximum bytes in one resource.
    pub per_resource_bytes: usize,
    /// Maximum combined bytes in unique resources.
    pub total_bytes: usize,
    /// Maximum number of unique resources.
    pub resource_count: usize,
    /// Maximum import chain depth, including the first resource.
    pub import_depth: usize,
}

impl ContextResourceLimits {
    /// Construct explicit acquisition limits.
    pub const fn new(
        per_resource_bytes: usize,
        total_bytes: usize,
        resource_count: usize,
        import_depth: usize,
    ) -> Self {
        Self {
            per_resource_bytes,
            total_bytes,
            resource_count,
            import_depth,
        }
    }
}

impl Default for ContextResourceLimits {
    fn default() -> Self {
        Self::new(1024 * 1024, 8 * 1024 * 1024, 128, 128)
    }
}

/// The caller's trust admission for a digest-addressed context.
#[derive(Debug, Clone)]
pub enum ResolvedContextResource {
    /// Accepted raw bytes. The loader still verifies their digest.
    Trusted(Vec<u8>),
    /// A located resource whose provenance is not accepted.
    Untrusted,
}

impl ResolvedContextResource {
    /// Admit raw bytes from an explicitly trusted resolver.
    pub fn trusted(bytes: Vec<u8>) -> Self {
        Self::Trusted(bytes)
    }
    /// Reject a located resource before hashing or parsing.
    pub fn untrusted() -> Self {
        Self::Untrusted
    }
}

/// An explicitly supplied source for digest-addressed context bytes.
pub trait ContextDigestResolver {
    /// Return a trust decision and raw bytes, or `None` when unavailable.
    /// `max_bytes` is a request to avoid large allocations; the loader checks it again.
    fn resolve(
        &self,
        reference: &str,
        max_bytes: usize,
    ) -> Result<Option<ResolvedContextResource>, String>;
}

/// An acquisition failure or a pure context-resolution failure.
#[derive(Debug, thiserror::Error)]
pub enum ContextResourceError {
    /// The supplied root could not be opened.
    #[error("cannot open context root: {0}")]
    Root(#[source] std::io::Error),
    /// A local import traverses above or outside the root.
    #[error("context path escapes the supplied root")]
    PathEscape,
    /// A named file is absent.
    #[error("context resource missing: {0}")]
    Missing(String),
    /// A named path is not a regular file.
    #[error("context resource is not a file: {0}")]
    NotFile(String),
    /// Reading a resource failed.
    #[error("cannot read context resource {0}: {1}")]
    Read(String, #[source] std::io::Error),
    /// The resolver failed to supply a decision.
    #[error("context resolver failed for {0}: {1}")]
    Resolver(String, String),
    /// One resource exceeds its raw-byte limit.
    #[error("context resource exceeds byte limit: {0}")]
    ResourceBytesExceeded(String),
    /// The closure exceeds its combined raw-byte limit.
    #[error("context resources exceed total byte limit")]
    TotalBytesExceeded,
    /// The closure has too many unique resources.
    #[error("context resources exceed count limit")]
    ResourceCountExceeded,
    /// One import chain exceeds the configured depth.
    #[error("context import depth exceeds configured limit")]
    ImportDepthExceeded,
    /// The bounded grammar or core semantic resolution failed.
    #[error(transparent)]
    Context(#[from] ContextError),
}

/// Load and validate the transitive context closure for several document scopes.
///
/// The `root` is an explicit document tree or extracted archive directory. Paths
/// are checked lexically and against the canonical root; capability-based file
/// opening keeps the read confined if a symlink changes during acquisition.
/// Each request is finally resolved by `morphir-core`, which detects duplicate
/// imports, cycles, invalid envelopes, and term conflicts.
///
/// ```
/// use morphir_common::ir_transport::metadata::{ContextRequest, ContextResourceLimits, load_context_resources};
/// use morphir_core::metadata::resolve_context;
/// use serde_json::json;
/// let root = tempfile::tempdir().unwrap();
/// std::fs::write(root.path().join("terms.jsonld"), br#"{"@context":{}}"#).unwrap();
/// let authored = json!("terms.jsonld");
/// let resources = load_context_resources(root.path(), &[ContextRequest::at_root(&authored)], None, ContextResourceLimits::default()).unwrap();
/// resolve_context(None, &authored, &resources, None).unwrap();
/// ```
pub fn load_context_resources(
    root: &Path,
    requests: &[ContextRequest<'_>],
    resolver: Option<&dyn ContextDigestResolver>,
    limits: ContextResourceLimits,
) -> Result<ContextResources, ContextResourceError> {
    let canonical_root = fs::canonicalize(root).map_err(ContextResourceError::Root)?;
    let directory = Dir::open_ambient_dir(&canonical_root, ambient_authority())
        .map_err(ContextResourceError::Root)?;
    let mut resources = ContextResources::new(".");
    let mut seen = BTreeSet::new();
    let mut total_bytes = 0usize;
    let mut pending = Vec::new();
    for request in requests {
        let base = request
            .base_file
            .map(|value| normalize_relative_path(value, None))
            .transpose()?;
        enqueue_references(request.authored, base.as_deref(), 1, &mut pending)?;
    }
    while let Some((reference, base, depth)) = pending.pop() {
        let is_digest = reference.starts_with(DIGEST_PREFIX);
        let identity = if is_digest {
            validate_digest_reference(&reference)?;
            reference.clone()
        } else {
            if base.as_deref() == Some(DIGEST_PREFIX) {
                return Err(ContextError::RelativeImportWithoutBase.into());
            }
            normalize_local(&reference, base.as_deref())?
        };
        if !seen.insert(identity.clone()) {
            continue;
        }
        if depth > limits.import_depth {
            return Err(ContextResourceError::ImportDepthExceeded);
        }
        if seen.len() > limits.resource_count {
            return Err(ContextResourceError::ResourceCountExceeded);
        }
        let bytes = if is_digest {
            let admitted = resolver
                .ok_or_else(|| ContextError::ResourceUnavailable(identity.clone()))?
                .resolve(&identity, limits.per_resource_bytes)
                .map_err(|error| ContextResourceError::Resolver(identity.clone(), error))?
                .ok_or_else(|| ContextError::ResourceUnavailable(identity.clone()))?;
            let bytes = match admitted {
                ResolvedContextResource::Trusted(bytes) => bytes,
                ResolvedContextResource::Untrusted => {
                    return Err(ContextError::ResourceUntrusted(identity).into());
                }
            };
            check_byte_limits(&identity, bytes.len(), &mut total_bytes, limits)?;
            let digest = Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            if !identity.ends_with(&digest) {
                return Err(ContextError::DigestMismatch(identity).into());
            }
            resources.insert_verified(&identity, bytes.clone(), true);
            bytes
        } else {
            let bytes = read_local(
                &directory,
                &canonical_root,
                &identity,
                limits.per_resource_bytes,
            )?;
            check_byte_limits(&identity, bytes.len(), &mut total_bytes, limits)?;
            resources.insert_local(&identity, bytes.clone());
            bytes
        };
        let document: Value = serde_json::from_slice(&bytes)
            .map_err(|_| ContextError::InvalidResource(identity.clone()))?;
        let context = document
            .as_object()
            .filter(|object| object.len() == 1)
            .and_then(|object| object.get("@context"))
            .ok_or_else(|| ContextError::InvalidResource(identity.clone()))?;
        let child_base = if is_digest {
            DIGEST_PREFIX
        } else {
            identity.as_str()
        };
        enqueue_references(context, Some(child_base), depth + 1, &mut pending)?;
    }
    for request in requests {
        resolve_context(None, request.authored, &resources, request.base_file)?;
    }
    Ok(resources)
}

fn enqueue_references(
    value: &Value,
    base: Option<&str>,
    depth: usize,
    pending: &mut Vec<(String, Option<String>, usize)>,
) -> Result<(), ContextResourceError> {
    match value {
        Value::String(reference) => {
            pending.push((reference.clone(), base.map(str::to_owned), depth))
        }
        Value::Array(items) => {
            for item in items.iter().rev() {
                if let Value::String(reference) = item {
                    pending.push((reference.clone(), base.map(str::to_owned), depth));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_digest_reference(reference: &str) -> Result<(), ContextResourceError> {
    let digest = reference
        .strip_prefix(DIGEST_PREFIX)
        .expect("checked digest prefix");
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ContextError::InvalidForm.into());
    }
    Ok(())
}

fn normalize_local(reference: &str, base: Option<&str>) -> Result<String, ContextResourceError> {
    if reference.contains("://") {
        return Err(ContextError::RemoteForbidden(reference.to_owned()).into());
    }
    if !reference.ends_with(".jsonld") || reference.contains('\\') {
        return Err(ContextError::InvalidForm.into());
    }
    normalize_relative_path(reference, base)
}

fn normalize_relative_path(
    reference: &str,
    base: Option<&str>,
) -> Result<String, ContextResourceError> {
    if reference.is_empty() || reference.contains('\\') {
        return Err(ContextError::InvalidForm.into());
    }
    let mut path = PathBuf::new();
    if let Some(base) = base {
        path.extend(Path::new(base).parent());
    }
    for component in Path::new(reference).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !path.pop() {
                    return Err(ContextResourceError::PathEscape);
                }
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(ContextResourceError::PathEscape);
            }
        }
    }
    Ok(path.to_string_lossy().replace('\\', "/"))
}

fn read_local(
    directory: &Dir,
    root: &Path,
    identity: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, ContextResourceError> {
    let path = root.join(identity);
    let canonical = fs::canonicalize(&path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => ContextResourceError::Missing(identity.to_owned()),
        _ => ContextResourceError::Read(identity.to_owned(), error),
    })?;
    if !canonical.starts_with(root) {
        return Err(ContextResourceError::PathEscape);
    }
    let metadata = fs::metadata(&canonical)
        .map_err(|error| ContextResourceError::Read(identity.to_owned(), error))?;
    if !metadata.is_file() {
        return Err(ContextResourceError::NotFile(identity.to_owned()));
    }
    if metadata.len() > max_bytes as u64 {
        return Err(ContextResourceError::ResourceBytesExceeded(
            identity.to_owned(),
        ));
    }
    let file = directory
        .open(identity)
        .map_err(|error| ContextResourceError::Read(identity.to_owned(), error))?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| ContextResourceError::Read(identity.to_owned(), error))?;
    if bytes.len() > max_bytes {
        return Err(ContextResourceError::ResourceBytesExceeded(
            identity.to_owned(),
        ));
    }
    Ok(bytes)
}

fn check_byte_limits(
    identity: &str,
    len: usize,
    total: &mut usize,
    limits: ContextResourceLimits,
) -> Result<(), ContextResourceError> {
    if len > limits.per_resource_bytes {
        return Err(ContextResourceError::ResourceBytesExceeded(
            identity.to_owned(),
        ));
    }
    *total = total
        .checked_add(len)
        .ok_or(ContextResourceError::TotalBytesExceeded)?;
    if *total > limits.total_bytes {
        return Err(ContextResourceError::TotalBytesExceeded);
    }
    Ok(())
}
