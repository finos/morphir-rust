//! Provider-owned completion of portable Gleam workspace discovery.
//!
//! Portable discovery (`morphir_workspace::discover`) is language-neutral: it
//! cannot know how `models/domain/widget.gleam` becomes a module, so a
//! synthesized project's `name` and `exposedModules` are left empty. This
//! module delegates discovery to the portable engine — which keeps owning
//! confinement, budgets, ordering and diagnostics — and only fills the
//! fields a Gleam-specific policy can supply, by post-processing the
//! returned snapshot.
//!
//! Gleam derives module identity from a source file's *path*, never from a
//! declaration inside the file — there is no `module` header to read. So
//! this reuses the same path-derived canonicalization compile time already
//! applies to a document URI (`canonicalize_gleam_module_segments`), applied
//! here to the `RelativePath` values discovery already resolved against the
//! selection's root.

use morphir_extension_sdk::prelude::*;
use morphir_workspace::{
    DiagnosticSeverity as WorkspaceDiagnosticSeverity, DiscoveryRequest, DiscoveryResponse,
    ProjectOrigin, ProjectState, RelativePath, WorkspaceDiagnostic, WorkspaceState,
};

use crate::{GleamExtension, canonicalize_gleam_module_segments};

/// Stable diagnostic code for a synthesized source whose path this provider
/// cannot turn into a valid Gleam module name (for example, a filename
/// containing a hyphen).
const GLEAM_WORKSPACE_INVALID_MODULE_PATH: &str = "gleam.workspace.invalid-module-path";

impl Workspace for GleamExtension {
    fn discover(&self, request: DiscoveryRequest) -> Result<DiscoveryResponse> {
        // `morphir_workspace::discover` owns confinement, budgets, ordering
        // and diagnostics; nothing here reimplements any of that. The
        // returned snapshot carries paths, not document text, which is all
        // module-identity synthesis needs.
        Ok(synthesize_identity(morphir_workspace::discover(request)))
    }
}

/// Fills `name` and `exposedModules` on a synthesized project whose name
/// discovery left empty. A project that already has a name — explicit,
/// manifest-derived, or otherwise — is untouched, since a name discovery
/// filled in came from the caller and is not this provider's to change. A
/// discovery failure passes through unchanged: it is not this function's to
/// reinterpret.
fn synthesize_identity(response: DiscoveryResponse) -> DiscoveryResponse {
    let mut snapshot = match response {
        DiscoveryResponse::Success { snapshot } => snapshot,
        failure @ DiscoveryResponse::Failure { .. } => return failure,
    };

    for project in &mut snapshot.projects {
        if !project.name.is_empty() {
            continue;
        }
        let ProjectOrigin::Synthesized { inputs } = &project.origin else {
            continue;
        };
        // An unnamed ad-hoc selection is only ever a single source: portable
        // discovery rejects more than one (`workspace.selection.name-required`)
        // before a snapshot like this can exist. Nothing else to derive a
        // name from.
        let [input] = inputs.as_slice() else {
            continue;
        };

        match derive_identity(&project.relative_path, input) {
            Ok((package_name, module_name)) => {
                project.name = package_name;
                project.exposed_modules = Some(vec![module_name]);
            }
            Err(message) => {
                project.state = ProjectState::Error;
                // `ProjectSnapshot::diagnostics` is documented as sorted by
                // project path, path, code, severity and message. A
                // synthesized project's diagnostics start empty
                // (`discover_ad_hoc_sources` never populates them), and this
                // is the only diagnostic this function ever adds, so a
                // single push keeps that ordering trivially satisfied. If a
                // second diagnostic is ever pushed here, re-sort afterward.
                project.diagnostics.push(WorkspaceDiagnostic {
                    severity: WorkspaceDiagnosticSeverity::Error,
                    code: GLEAM_WORKSPACE_INVALID_MODULE_PATH.to_owned(),
                    message,
                    path: Some(input.clone()),
                    project_path: Some(project.relative_path.clone()),
                });
            }
        }
    }

    if snapshot
        .projects
        .iter()
        .any(|project| project.state == ProjectState::Error)
    {
        snapshot.state = WorkspaceState::Error;
    }

    DiscoveryResponse::Success { snapshot }
}

/// Derives a synthesized project's package name and its single exposed
/// module name from `input`'s path relative to `root`.
///
/// Gleam has no module header, so — unlike Elm's declared-name-then-filename
/// policy — this always derives from the path: the segments between `root`
/// and the file, canonicalized the same way a compile-time document URI is
/// (snake_case segments become kebab-case, joined by `/`). The package name
/// reuses that module path under the placeholder `local` scope, with `/`
/// joins flattened to `-`, since a package name is a single scoped segment,
/// not a nested path.
fn derive_identity(
    root: &RelativePath,
    input: &RelativePath,
) -> std::result::Result<(String, String), String> {
    let mut segments = relative_segments(root, input);
    let file_name = segments
        .last_mut()
        .ok_or_else(|| format!("synthesized source '{}' has no module path", input.as_str()))?;
    *file_name = file_name
        .strip_suffix(".gleam")
        .ok_or_else(|| {
            format!(
                "synthesized source '{}' does not identify a .gleam file",
                input.as_str()
            )
        })?
        .to_owned();
    let refs = segments.iter().map(String::as_str).collect::<Vec<_>>();
    let module_name = canonicalize_gleam_module_segments(&refs)?.to_string();
    // Two different selections can collide on this package name: segment
    // canonicalization already maps `_` to `-` per segment (`a_b` and `a-b`
    // both canonicalize to `a-b`), and flattening `/` to `-` below compounds
    // that across segments, so `a/b.gleam` and `a_b.gleam` both yield
    // `local/a-b`, with different module sets. This never collides *within*
    // one snapshot — an unnamed selection is always exactly one source, so
    // there is only ever one derived name here — but two separate discovery
    // invocations can still produce colliding package identities wherever a
    // name is later used as a key. Not fixed here: the derivation policy is
    // settled, and the case is unreachable in the one place this function
    // runs.
    let package_name = format!("local/{}", module_name.replace('/', "-"));
    Ok((package_name, module_name))
}

/// Splits `input`'s wire path into the segments beneath `root`.
fn relative_segments(root: &RelativePath, input: &RelativePath) -> Vec<String> {
    let skip = if root.as_str() == "." {
        0
    } else {
        root.as_str().split('/').count()
    };
    input
        .as_str()
        .split('/')
        .skip(skip)
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_workspace::{
        DiscoveryPurpose, FileEntry, FileTree, ProjectSnapshot, ProjectSource, SourceSelection,
        WORKSPACE_DISCOVERY_PROTOCOL, WORKSPACE_PROTOCOL_UNSUPPORTED, WorkspaceSnapshot,
    };
    use std::collections::BTreeMap;

    fn ad_hoc_request(root: &str, path: &str) -> DiscoveryRequest {
        let root = RelativePath::parse(root).expect("a confined wire path");
        let source = RelativePath::parse(path).expect("a confined wire path");
        DiscoveryRequest {
            protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
            development_root: FileTree {
                entries: BTreeMap::from([
                    (RelativePath::root(), FileEntry::Directory),
                    (
                        source.clone(),
                        FileEntry::File {
                            text: String::new(),
                        },
                    ),
                ]),
            },
            morphir_home: None,
            system_config: None,
            environment: BTreeMap::new(),
            cli_overlay: serde_json::json!({}),
            purpose: DiscoveryPurpose::AdHocSources {
                project: ProjectSource::Synthesized,
                sources: SourceSelection {
                    root,
                    paths: vec![source],
                },
                language_id: "gleam".to_owned(),
            },
        }
    }

    /// A synthesized project's name is established by the provider. Portable
    /// discovery is language-neutral: it cannot know how
    /// `models/domain/widget.gleam` becomes a module, so it leaves the name
    /// empty and the provider fills it. Nested on purpose: a flat fixture
    /// cannot show whether the selection root was applied or ignored, since
    /// with no intervening directory there is nothing for the root to strip.
    #[test]
    fn a_synthesized_gleam_project_is_named_by_the_provider() {
        let request = ad_hoc_request("models", "models/domain/widget.gleam");

        let response = GleamExtension
            .discover(request)
            .expect("discovery of a valid ad-hoc selection should succeed");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        assert_eq!(snapshot.projects.len(), 1);
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "local/domain-widget");
        assert_eq!(
            project.exposed_modules,
            Some(vec!["domain/widget".to_owned()])
        );
        assert_eq!(project.state, ProjectState::Unloaded);
        assert_eq!(snapshot.state, WorkspaceState::Open);
    }

    /// A flat selection (no directory between the root and the file) proves
    /// the same rule collapses to the plain filename, contrasting with the
    /// nested case above where the root's directory segments feed the name.
    #[test]
    fn a_flat_synthesized_gleam_project_is_named_from_its_filename() {
        let request = ad_hoc_request(".", "widget.gleam");

        let response = GleamExtension
            .discover(request)
            .expect("discovery of a valid ad-hoc selection should succeed");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "local/widget");
        assert_eq!(project.exposed_modules, Some(vec!["widget".to_owned()]));
    }

    /// An explicitly named synthesized project is left exactly as discovery
    /// produced it — the name came from the caller, not from this provider.
    #[test]
    fn an_explicitly_named_synthesized_project_is_not_touched() {
        let mut request = ad_hoc_request("models", "models/domain/widget.gleam");
        request.cli_overlay = serde_json::json!({ "project": { "name": "acme/widgets" } });

        let response = GleamExtension
            .discover(request)
            .expect("discovery of a valid ad-hoc selection should succeed");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "acme/widgets");
        assert_eq!(project.exposed_modules, None);
    }

    /// A discovery failure is passed through unchanged, never reinterpreted.
    /// Asserting the variant alone would also pass a provider that rewrote
    /// the failure's code or message while keeping it a `Failure`, so this
    /// pins the exact code `morphir_workspace::discover` produces.
    #[test]
    fn a_discovery_failure_passes_through_unchanged() {
        let mut request = ad_hoc_request("models", "models/domain/widget.gleam");
        request.protocol_version = 999;

        let response = GleamExtension
            .discover(request)
            .expect("the typed call itself should not error");

        let DiscoveryResponse::Failure { error } = response else {
            panic!("expected a discovery failure");
        };
        assert_eq!(error.code, WORKSPACE_PROTOCOL_UNSUPPORTED);
    }

    /// A manifest-backed project is never touched, even if its name happens
    /// to be empty — nothing in the type system stops that, so the guard
    /// that checks `origin` before deriving anything is load-bearing, not
    /// belt-and-braces. This bypasses `GleamExtension::discover` and drives
    /// `synthesize_identity` directly, since portable discovery itself
    /// never produces an empty-named manifest project.
    #[test]
    fn a_manifest_project_with_an_empty_name_is_not_touched() {
        let manifest_path = RelativePath::parse("morphir.toml").expect("a confined wire path");
        let project = ProjectSnapshot {
            name: String::new(),
            version: None,
            relative_path: RelativePath::root(),
            config_anchor: Some(manifest_path.clone()),
            source_directory: RelativePath::root(),
            state: ProjectState::Unloaded,
            diagnostics: Vec::new(),
            origin: ProjectOrigin::Manifest {
                path: manifest_path.clone(),
            },
            exposed_modules: None,
        };
        let snapshot = WorkspaceSnapshot {
            protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
            config_anchor: Some(manifest_path),
            name: None,
            state: WorkspaceState::Open,
            projects: vec![project],
            diagnostics: Vec::new(),
        };

        let response = synthesize_identity(DiscoveryResponse::Success { snapshot });

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "");
        assert_eq!(project.exposed_modules, None);
        assert_eq!(project.state, ProjectState::Unloaded);
        assert_eq!(snapshot.state, WorkspaceState::Open);
    }

    /// A source filename Gleam cannot turn into a valid module segment (a
    /// literal hyphen) is a project-level diagnostic, not a silently wrong
    /// name and not a panic. Every diagnostic field is asserted, not just
    /// the code, since an unchecked `path`/`projectPath` would let a
    /// diagnostic that points at the wrong file pass unnoticed.
    #[test]
    fn an_unsynthesizable_module_path_is_a_project_diagnostic_not_a_panic() {
        let request = ad_hoc_request("models", "models/order-processing.gleam");

        let response = GleamExtension
            .discover(request)
            .expect("the typed call itself should not error");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        assert_eq!(snapshot.state, WorkspaceState::Error);
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "");
        assert_eq!(project.exposed_modules, None);
        assert_eq!(project.state, ProjectState::Error);
        assert_eq!(project.diagnostics.len(), 1);
        let diagnostic = &project.diagnostics[0];
        assert_eq!(diagnostic.code, GLEAM_WORKSPACE_INVALID_MODULE_PATH);
        assert_eq!(diagnostic.severity, WorkspaceDiagnosticSeverity::Error);
        assert_eq!(
            diagnostic.path,
            Some(
                RelativePath::parse("models/order-processing.gleam").expect("a confined wire path")
            )
        );
        assert_eq!(
            diagnostic.project_path,
            Some(RelativePath::parse("models").expect("a confined wire path"))
        );
    }
}
