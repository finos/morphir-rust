use super::{
    Error, PROFILE, PROFILE_VERSION, RestoredPackage, files, fresh, policy, require, store, verify,
};
use crate::{
    local_registry::*,
    resolution::{self, LockedGraph, ReleaseId, ResolutionResult},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::path::Path;
mod catalog;

/// Initial resolution from a fixed published root in one local repository.
pub struct ResolveRequest<'a> {
    /// Current caller-supplied fresh-metadata policy.
    pub policy: &'a [u8],
    /// Exact published root; no root version selection is performed.
    pub root: ReleaseId,
    /// Caller-controlled immutable or coordinated local registry directory.
    pub registry: &'a Path,
    /// Previously initialized protected trust directory.
    pub state: &'a Path,
    /// Absent lock file, with an existing caller-controlled parent directory.
    pub output: &'a Path,
}

/// Scoped update from an untrusted full lock and current authenticated metadata.
pub struct UpdateRequest<'a> {
    /// Current caller-supplied fresh-metadata policy.
    pub policy: &'a [u8],
    /// Full draft.3 baseline lock; never rewritten.
    pub lock: &'a [u8],
    /// Existing non-root paths to update, eligible or exact.
    pub targets: &'a [resolution::UpdateTarget],
    /// Caller-controlled immutable or coordinated local registry directory.
    pub registry: &'a Path,
    /// Previously initialized protected trust directory.
    pub state: &'a Path,
    /// Absent new lock file with an existing parent directory.
    pub output: &'a Path,
}

/// Update only the requested old dependency closure. The root and unrelated
/// releases remain fixed. Historical metadata pins do not authorize this call;
/// all old immutable records are checked against current authenticated metadata.
///
/// ```no_run
/// use morphir_package::{local_registry::mvp::{update, UpdateRequest},
///     resolution::{PackagePath, UpdateTarget}};
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let targets = [UpdateTarget::Eligible { package_path: PackagePath::parse("example.com/lib")? }];
/// update(UpdateRequest { policy: &std::fs::read("trust-policy.json")?,
///     lock: &std::fs::read("morphir.lock")?, targets: &targets,
///     registry: "registry".as_ref(), state: "trust".as_ref(),
///     output: "updated.lock".as_ref() }).await?;
/// # Ok(())
/// # }
/// ```
pub async fn update(request: UpdateRequest<'_>) -> Result<ResolveReport, Error> {
    let old = decode_library_lock(request.lock)?;
    require(
        old.registries().len() == 1,
        "MVP requires exactly one lock registry",
    )?;
    let root = old.graph().root().clone();
    execute(
        ResolveRequest {
            policy: request.policy,
            root,
            registry: request.registry,
            state: request.state,
            output: request.output,
        },
        Operation::Update {
            old: &old,
            targets: request.targets,
            bytes: request.lock.len(),
        },
        jiff::Timestamp::now(),
    )
    .await
}

enum Operation<'a> {
    Initial,
    Update {
        old: &'a LibraryLock,
        targets: &'a [resolution::UpdateTarget],
        bytes: usize,
    },
}

/// A complete authenticated and content-verified graph with its published lock.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveReport {
    /// Prerelease capability name, not provider qualification.
    pub profile: &'static str,
    /// Prerelease capability revision.
    pub profile_version: &'static str,
    /// Complete graph in resolver presentation order.
    pub graph: LockedGraph,
    /// Verified releases in graph order. Directories describe restore layout;
    /// resolve publishes only the lock file, not these package directories.
    pub packages: Vec<RestoredPackage>,
}

/// Resolve, authenticate and verify a complete Library graph before publishing a
/// new draft.3 lock file. No partial lock is published on failure. Every attempt
/// reauthenticates current metadata; this operation does not update an old lock.
///
/// ```no_run
/// use morphir_package::{local_registry::mvp::{resolve, ResolveRequest},
///     resolution::{PackagePath, ReleaseId, StableVersion}};
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let policy = std::fs::read("trust-policy.json")?;
/// let report = resolve(ResolveRequest {
///     policy: &policy,
///     root: ReleaseId::new(PackagePath::parse("example.com/app")?, StableVersion::parse("1.0.0")?),
///     registry: "registry".as_ref(), state: "initialized-trust".as_ref(),
///     output: "morphir.lock".as_ref(),
/// }).await?;
/// assert_eq!(report.graph.nodes().len(), report.packages.len());
/// # Ok(())
/// # }
/// ```
pub async fn resolve(request: ResolveRequest<'_>) -> Result<ResolveReport, Error> {
    resolve_at(request, jiff::Timestamp::now()).await
}

pub(super) async fn resolve_at(
    request: ResolveRequest<'_>,
    now: jiff::Timestamp,
) -> Result<ResolveReport, Error> {
    execute(request, Operation::Initial, now).await
}

async fn execute(
    request: ResolveRequest<'_>,
    operation: Operation<'_>,
    now: jiff::Timestamp,
) -> Result<ResolveReport, Error> {
    let policy = policy(request.policy)?;

    require(
        repository_permits(
            &policy,
            policy.repositories()[0].identity(),
            request.root.package_path(),
        ),
        "repository namespace unauthorized",
    )?;
    RegistryPath::parse(&format!(
        "{}/{}",
        request.root.package_path(),
        request.root.version().as_str()
    ))
    .map_err(|_| Error::Refused("unsafe release directory"))?;
    files::absent(request.output)?;
    let parent = request
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = files::directory(parent)?;
    let registry = files::directory(request.registry)?;
    let stage = tempfile::Builder::new()
        .prefix(".morphir-resolve-")
        .tempdir_in(parent)?;
    let backend = store::Backend::begin(
        request.state,
        &policy.repositories()[0],
        now,
        request.policy.len()
            + serde_json::to_vec(&request.root)?.len()
            + match &operation {
                Operation::Initial => 0,
                Operation::Update { bytes, targets, .. } => {
                    bytes + serde_json::to_vec(targets)?.len()
                }
            },
    )?;
    let metadata = fresh::authenticate(&backend, &registry, &policy.repositories()[0]).await?;
    let catalog = catalog::read(&backend, &registry, &policy, metadata.targets())?;
    let active = catalog.active();
    let resolved = match &operation {
        Operation::Initial => resolution::resolve_library(&serde_json::to_string(
            &catalog.resolution_input(&request.root)?,
        )?)?,
        Operation::Update { old, targets, .. } => resolution::update_library(
            &serde_json::to_string(&catalog.update_input(old, targets)?)?,
            &active,
        )?,
    };
    let graph = match resolved {
        ResolutionResult::Resolved(graph) => graph,
        ResolutionResult::Rejected(diagnostic) => {
            return Err(Error::ResolutionRejected(Box::new(diagnostic)));
        }
    };
    let mut document = lock_document(&graph, &catalog, metadata.evidence())?;
    // Host dependencies may enable serde_json/preserve_order. Artifact bytes
    // must not depend on Cargo feature unification or object insertion order.
    document.sort_all_objects();
    let mut bytes = serde_json::to_vec_pretty(&document)?;
    bytes.push(b'\n');
    // Validate the complete wire contract rather than serializing LibraryLock's
    // internal representation, which intentionally omits format discriminators.
    let lock = decode_library_lock(&bytes)?;
    // The solver admitted these exact yanked releases only through its frozen
    // pins. Preserve that eligibility during content and publisher verification.
    let frozen = graph
        .nodes()
        .iter()
        .filter(|n| !active.contains(n.release()))
        .map(|n| n.release().clone())
        .collect();
    let selection = match operation {
        Operation::Initial => verify::Selection::New,
        Operation::Update { .. } => verify::Selection::ScopedUpdate(&frozen),
    };
    let packages = verify::graph(
        &backend,
        &registry,
        &policy,
        &lock,
        metadata.targets(),
        &stage.path().join("libraries"),
        selection,
    )?;
    files::write(stage.path(), "morphir.lock", &bytes)?;
    backend.accept_operation_time()?;
    files::absent(request.output)?;
    files::promote(&stage.path().join("morphir.lock"), request.output)?;
    if let Err(error) = backend.finish() {
        let _ = std::fs::remove_file(request.output);
        return Err(error);
    }
    Ok(ResolveReport {
        profile: PROFILE,
        profile_version: PROFILE_VERSION,
        graph,
        packages,
    })
}

fn lock_document(
    graph: &LockedGraph,
    catalog: &catalog::Catalog,
    mut evidence: Vec<Value>,
) -> Result<Value, Error> {
    let mut nodes: Vec<_> = graph.nodes().iter().collect();
    nodes.sort_by(|a, b| {
        (
            a.release().package_path().as_str(),
            a.release().version().as_str(),
        )
            .cmp(&(
                b.release().package_path().as_str(),
                b.release().version().as_str(),
            ))
    });
    let mut acquisitions = Vec::new();
    for (i, node) in nodes.into_iter().enumerate() {
        let entry = catalog.selected(node.release())?;
        let statement = format!("statement-{i}");
        acquisitions.push(
            json!({"release": node.release(), "registry":"local", "record":entry.reference,
            "source":entry.record.source(), "statement":statement}),
        );
        evidence.push(
            json!({"id":statement,"registry":"local","kind":"release-statement",
            "path":entry.record.statement().path(),"digest":entry.record.statement().digest()}),
        );
    }
    evidence.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    Ok(json!({"formatVersion":"0.1.0-draft.3","kind":"LibraryLock",
        "resolution":{"policy":"flat-library:0.1.0-draft.2","profile":"local-library",
            "requiredCapabilities":["dsse-ed25519","local-directory","tuf-1.0.36"]},
        "graph":graph,"registries":[{"id":"local","snapshot":"snapshot"}],
        "acquisitions":acquisitions,"evidence":evidence}))
}
