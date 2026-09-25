//! Explicit, bounded acquisition of authored metadata context resources.
//!
//! The returned bytes are handed to the pure `morphir-core` resolver. This
//! loader only reads paths below a caller-selected tree or archive directory,
//! or asks a caller-supplied digest resolver. It never performs network I/O.

#[cfg(unix)]
use cap_fs_ext::OpenOptionsSyncExt;
use cap_std::ambient_authority;
use cap_std::fs::{Dir, OpenOptions};
use morphir_core::metadata::{ContextError, ContextResources, resolve_context};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const DIGEST_PREFIX: &str = "morphir://context/sha256/";
// The pure resolver has the same ceiling. Never acquire a deeper closure
// even when the caller configures a larger budget.
const MAX_CORE_IMPORT_DEPTH: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum ResourceIdentity {
    Local(PathBuf),
    Digest(String),
}

struct CachedResource {
    bytes: Vec<u8>,
    context: Value,
}

struct Loader<'a> {
    directory: Dir,
    root: PathBuf,
    resolver: Option<&'a dyn ContextDigestResolver>,
    limits: ContextResourceLimits,
    resources: ContextResources,
    cache: BTreeMap<ResourceIdentity, CachedResource>,
    loaded_paths: BTreeSet<String>,
    total_bytes: usize,
}

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
    /// Maximum import chain depth, including the first resource. The core
    /// resolver's ceiling of 128 applies even if this value is higher.
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
/// Each request is finally resolved by `morphir-core`, which checks the bounded
/// grammar and term conflicts. This loader also compares canonical file
/// identities so symlink aliases cannot hide a duplicate import or cycle.
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
    let mut loader = Loader {
        directory,
        root: canonical_root,
        resolver,
        limits,
        resources: ContextResources::new("."),
        cache: BTreeMap::new(),
        loaded_paths: BTreeSet::new(),
        total_bytes: 0,
    };
    for request in requests {
        let base = request
            .base_file
            .map(|value| normalize_relative_path(value, None))
            .transpose()?;
        loader.visit_value(
            request.authored,
            base.as_deref(),
            1,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
        )?;
    }
    for request in requests {
        resolve_context(None, request.authored, &loader.resources, request.base_file)?;
    }
    Ok(loader.resources)
}

impl Loader<'_> {
    fn visit_value(
        &mut self,
        value: &Value,
        base: Option<&str>,
        depth: usize,
        seen: &mut BTreeSet<ResourceIdentity>,
        active: &mut BTreeSet<ResourceIdentity>,
    ) -> Result<(), ContextResourceError> {
        match value {
            Value::String(reference) => self.visit_reference(reference, base, depth, seen, active),
            Value::Array(items) => {
                for item in items {
                    if let Value::String(reference) = item {
                        self.visit_reference(reference, base, depth, seen, active)?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn visit_reference(
        &mut self,
        reference: &str,
        base: Option<&str>,
        depth: usize,
        seen: &mut BTreeSet<ResourceIdentity>,
        active: &mut BTreeSet<ResourceIdentity>,
    ) -> Result<(), ContextResourceError> {
        let is_digest = reference.starts_with(DIGEST_PREFIX);
        let identity = if is_digest {
            validate_digest_reference(reference)?;
            reference.to_owned()
        } else {
            if base == Some(DIGEST_PREFIX) {
                return Err(ContextError::RelativeImportWithoutBase.into());
            }
            normalize_local(reference, base)?
        };
        if depth > self.limits.import_depth.min(MAX_CORE_IMPORT_DEPTH) {
            return Err(ContextResourceError::ImportDepthExceeded);
        }
        let canonical = if is_digest {
            ResourceIdentity::Digest(identity.clone())
        } else {
            ResourceIdentity::Local(canonical_local(&self.root, &identity)?)
        };
        if active.contains(&canonical) {
            return Err(ContextError::ImportCycle(identity).into());
        }
        if !seen.insert(canonical.clone()) {
            return Err(ContextError::DuplicateImport(identity).into());
        }
        active.insert(canonical.clone());
        let new_path = !self.loaded_paths.contains(&identity);
        if new_path && self.loaded_paths.len() >= self.limits.resource_count {
            return Err(ContextResourceError::ResourceCountExceeded);
        }
        if !self.cache.contains_key(&canonical) {
            let bytes = if is_digest {
                self.read_digest(&identity)?
            } else {
                read_local(&self.directory, &identity, self.limits.per_resource_bytes)?
            };
            let document: Value = serde_json::from_slice(&bytes)
                .map_err(|_| ContextError::InvalidResource(identity.clone()))?;
            let context = document
                .as_object()
                .filter(|object| object.len() == 1)
                .and_then(|object| object.get("@context"))
                .ok_or_else(|| ContextError::InvalidResource(identity.clone()))?
                .clone();
            self.cache
                .insert(canonical.clone(), CachedResource { bytes, context });
        }
        let cached = self.cache.get(&canonical).expect("resource was cached");
        if new_path {
            check_byte_limits(
                &identity,
                cached.bytes.len(),
                &mut self.total_bytes,
                self.limits,
            )?;
            self.loaded_paths.insert(identity.clone());
        }
        if is_digest {
            self.resources
                .insert_verified(&identity, cached.bytes.clone(), true);
        } else {
            self.resources.insert_local(&identity, cached.bytes.clone());
        }
        let context = cached.context.clone();
        let child_base = if is_digest {
            DIGEST_PREFIX
        } else {
            identity.as_str()
        };
        let result = self.visit_value(&context, Some(child_base), depth + 1, seen, active);
        active.remove(&canonical);
        result
    }

    fn read_digest(&self, identity: &str) -> Result<Vec<u8>, ContextResourceError> {
        let admitted = self
            .resolver
            .ok_or_else(|| ContextError::ResourceUnavailable(identity.to_owned()))?
            .resolve(identity, self.limits.per_resource_bytes)
            .map_err(|error| ContextResourceError::Resolver(identity.to_owned(), error))?
            .ok_or_else(|| ContextError::ResourceUnavailable(identity.to_owned()))?;
        let bytes = match admitted {
            ResolvedContextResource::Trusted(bytes) => bytes,
            ResolvedContextResource::Untrusted => {
                return Err(ContextError::ResourceUntrusted(identity.to_owned()).into());
            }
        };
        if bytes.len() > self.limits.per_resource_bytes {
            return Err(ContextResourceError::ResourceBytesExceeded(
                identity.to_owned(),
            ));
        }
        let digest = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if !identity.ends_with(&digest) {
            return Err(ContextError::DigestMismatch(identity.to_owned()).into());
        }
        Ok(bytes)
    }
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

fn canonical_local(root: &Path, identity: &str) -> Result<PathBuf, ContextResourceError> {
    let path = root.join(identity);
    let canonical = fs::canonicalize(&path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => ContextResourceError::Missing(identity.to_owned()),
        _ => ContextResourceError::Read(identity.to_owned(), error),
    })?;
    if !canonical.starts_with(root) {
        return Err(ContextResourceError::PathEscape);
    }
    Ok(canonical)
}

fn read_local(
    directory: &Dir,
    identity: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, ContextResourceError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.nonblock(true);
    let file = directory
        .open_with(identity, &options)
        .map_err(|error| ContextResourceError::Read(identity.to_owned(), error))?;
    let metadata = file
        .metadata()
        .map_err(|error| ContextResourceError::Read(identity.to_owned(), error))?;
    if !metadata.is_file() {
        return Err(ContextResourceError::NotFile(identity.to_owned()));
    }
    if metadata.len() > max_bytes as u64 {
        return Err(ContextResourceError::ResourceBytesExceeded(
            identity.to_owned(),
        ));
    }
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
