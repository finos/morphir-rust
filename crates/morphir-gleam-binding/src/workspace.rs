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

/// Fills whichever of `name` and `exposedModules` a synthesized project is
/// still missing.
///
/// The two fields are completed independently. A name discovery supplied —
/// explicit, manifest-derived, or otherwise — is never overwritten, since it
/// came from the caller and is not this provider's to change. Exposure,
/// though, is derived whenever discovery left it unset, *including* for a
/// project that arrived with an explicit name: the host's `--package-name`
/// says what to call the package, not which modules it publishes, and a
/// single file's exposed module is exactly as derivable either way. Making
/// exposure depend on the name would mean the same file advertised
/// `exposedModules: ["domain/widget"]` unnamed and `null` named, leaving a
/// consumer unable to tell "no exposure was derived" from "expose
/// everything".
///
/// A manifest-origin project is left entirely alone: everything it could
/// need already came from its manifest. A discovery failure passes through
/// unchanged: it is not this function's to reinterpret.
fn synthesize_identity(response: DiscoveryResponse) -> DiscoveryResponse {
    let mut snapshot = match response {
        DiscoveryResponse::Success { snapshot } => snapshot,
        failure @ DiscoveryResponse::Failure { .. } => return failure,
    };

    for project in &mut snapshot.projects {
        if !project.name.is_empty() && project.exposed_modules.is_some() {
            continue;
        }
        let ProjectOrigin::Synthesized { inputs } = &project.origin else {
            continue;
        };
        // An *unnamed* ad-hoc selection is only ever a single source:
        // portable discovery rejects more than one
        // (`workspace.selection.name-required`) before a snapshot like this
        // can exist, since there would be nothing to derive a name from. The
        // `debug_assert!` states that cross-crate invariant where this code
        // relies on it: were a future change to let an unnamed multi-input
        // synthesized project through, it would otherwise keep its empty
        // name all the way to compile and fail there with an
        // unrelated-looking error, with no test failing first.
        //
        // A *named* multi-source selection is allowed, though, and now
        // reaches here because exposure is derived independently of the
        // name. There is no single module to expose for it, so it falls out
        // of the `let else` below with `exposedModules` unset — which means
        // "expose everything", the only honest answer for a selection whose
        // modules this provider has not enumerated.
        debug_assert!(
            !project.name.is_empty() || inputs.len() == 1,
            "an unnamed synthesized project should hold exactly one input, found {}",
            inputs.len()
        );
        let [input] = inputs.as_slice() else {
            continue;
        };

        match derive_identity(&project.relative_path, input) {
            Ok((package_name, module_name)) => {
                if project.name.is_empty() {
                    project.name = package_name;
                }
                if project.exposed_modules.is_none() {
                    project.exposed_modules = Some(vec![module_name]);
                }
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
///
/// This strips by segment *count*, not by matching the root's segments, and
/// so differs from the compile-time derivation in one place:
/// `module_name_from_document_uri` (`lib.rs`) *errors* when a document is
/// outside the configured source root, whereas this would silently drop the
/// first `n` segments of an unrelated path and hand back a wrong-but-valid
/// module name.
///
/// That difference is unreachable rather than harmless, and what makes it
/// unreachable lives in another crate: `validate_ad_hoc_selection`
/// (`morphir-workspace`, `src/discovery/mod.rs`) rejects any selected path
/// not under the selection's root with `workspace.selection.outside-root`
/// before a provider ever sees the selection, so every `input` reaching here
/// is a path whose leading segments really are `root`'s. Nothing in this
/// crate's types states that dependency, which is why it is written down
/// here: if that check is ever relaxed or reordered, this function needs to
/// start matching segments and failing, not counting them.
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

    /// An explicit name survives untouched — it came from the caller, not
    /// from this provider — but exposure is still derived, because the two
    /// fields answer different questions. `--package-name` says what to call
    /// the package; it says nothing about which modules the package
    /// publishes, and the single submitted file's module path is exactly as
    /// derivable named as unnamed. Pinned so that the same file cannot
    /// advertise `["domain/widget"]` unnamed and `null` named, which would
    /// make an unrelated flag change the shape of the response.
    #[test]
    fn an_explicit_name_survives_but_exposure_is_still_derived() {
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
        assert_eq!(
            project.exposed_modules,
            Some(vec!["domain/widget".to_owned()])
        );
        assert_eq!(project.state, ProjectState::Unloaded);
    }

    /// Deriving exposure for an explicitly named project means an
    /// unsynthesizable module path now fails for that project too, where a
    /// name previously short-circuited the whole derivation and let it pass
    /// silently. That is the intended consequence, not a regression: exposure
    /// genuinely requires a valid Gleam module path, the diagnostic is the
    /// same one the identical file gets without a name, and the alternative —
    /// reporting success and then failing at compile with
    /// `INVALID_MODULE_URI` — hides the problem until later. Unlike Elm,
    /// whose derivation cannot fail, Gleam's can, so only this provider has a
    /// case to pin here.
    #[test]
    fn an_explicit_name_does_not_excuse_an_unsynthesizable_module_path() {
        let mut request = ad_hoc_request("models", "models/order-processing.gleam");
        request.cli_overlay = serde_json::json!({ "project": { "name": "acme/orders" } });

        let response = GleamExtension
            .discover(request)
            .expect("the typed call itself should not error");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        assert_eq!(snapshot.state, WorkspaceState::Error);
        let project = &snapshot.projects[0];
        // The caller's name is still not this provider's to rewrite, even on
        // the failure path.
        assert_eq!(project.name, "acme/orders");
        assert_eq!(project.exposed_modules, None);
        assert_eq!(project.state, ProjectState::Error);
        assert_eq!(project.diagnostics.len(), 1);
        assert_eq!(
            project.diagnostics[0].code,
            GLEAM_WORKSPACE_INVALID_MODULE_PATH
        );
    }

    /// A *named* selection may hold more than one source — portable
    /// discovery only constrains cardinality for an unnamed one — and such a
    /// project now reaches the derivation loop, because exposure is no longer
    /// gated on the name being empty. It must fall out of the single-input
    /// guard leaving `exposedModules` unset (meaning "expose everything"),
    /// not panic on the `debug_assert!` and not invent an exposure list from
    /// one arbitrary member. Tests run in debug, so a `debug_assert!` written
    /// on input count alone would fail here.
    #[test]
    fn a_named_multi_source_selection_derives_no_exposure() {
        let root = RelativePath::parse("models").expect("a confined wire path");
        let first =
            RelativePath::parse("models/domain/widget.gleam").expect("a confined wire path");
        let second =
            RelativePath::parse("models/domain/gadget.gleam").expect("a confined wire path");
        let request = DiscoveryRequest {
            protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
            development_root: FileTree {
                entries: BTreeMap::from([
                    (RelativePath::root(), FileEntry::Directory),
                    (
                        first.clone(),
                        FileEntry::File {
                            text: String::new(),
                        },
                    ),
                    (
                        second.clone(),
                        FileEntry::File {
                            text: String::new(),
                        },
                    ),
                ]),
            },
            morphir_home: None,
            system_config: None,
            environment: BTreeMap::new(),
            cli_overlay: serde_json::json!({ "project": { "name": "acme/widgets" } }),
            purpose: DiscoveryPurpose::AdHocSources {
                project: ProjectSource::Synthesized,
                sources: SourceSelection {
                    root,
                    paths: vec![first, second],
                },
                language_id: "gleam".to_owned(),
            },
        };

        let response = GleamExtension
            .discover(request)
            .expect("discovery of a valid named ad-hoc selection should succeed");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "acme/widgets");
        assert_eq!(project.exposed_modules, None);
        assert_eq!(project.state, ProjectState::Unloaded);
    }

    /// The module name discovery derives for a file, via `derive_identity`.
    fn discovery_module_name(root: &str, document: &str) -> String {
        derive_identity(
            &RelativePath::parse(root).expect("a confined wire path"),
            &RelativePath::parse(document).expect("a confined wire path"),
        )
        .expect("a derivable module path")
        .1
    }

    /// The module name compilation derives for the same file, via
    /// `module_name_from_document_uri`.
    ///
    /// Compile works in absolute paths and URIs where discovery works in
    /// root-relative wire paths, so the two are put on the same footing by
    /// rooting the wire pair under one development root — which is exactly
    /// what the host does when it turns a discovered selection into a
    /// `CompileRequest`. A `.` selection root becomes the development root
    /// itself.
    fn compile_module_name(root: &str, document: &str) -> String {
        const DEVELOPMENT_ROOT: &str = "/work/project";
        let source_root = if root == "." {
            DEVELOPMENT_ROOT.to_owned()
        } else {
            format!("{DEVELOPMENT_ROOT}/{root}")
        };
        let source_root = crate::parsed_path(&source_root).expect("a parseable source root");
        crate::module_name_from_document_uri(
            &format!("{DEVELOPMENT_ROOT}/{document}"),
            Some(&source_root),
        )
        .expect("a derivable module path")
        .to_string()
    }

    /// Gleam derives a module path from a document in two independent
    /// places: `derive_identity` here, which names the module discovery
    /// advertises in `exposedModules`, and `module_name_from_document_uri`
    /// in `lib.rs`, which names the module compilation actually emits. Their
    /// agreement is this provider's central premise — discovery promises a
    /// module that compilation must then produce — and nothing else runs
    /// both over one logical file. Were they to drift, discovery would
    /// advertise one name, compilation would emit another, and the only
    /// symptom would be `MISSING_EXPOSED_MODULE` at compile time, with no
    /// test failing first.
    ///
    /// Deliberately not deduplicated into one function: the two serve
    /// different layers and one errors (outside-root documents) where the
    /// other cannot. Constraining them with a test is the chosen way to keep
    /// them honest. The expected value is asserted as well as the equality,
    /// so a change that moves both derivations together still fails here.
    ///
    /// The nested case is what makes this meaningful — with no directory
    /// between root and file there is nothing for either root-stripping rule
    /// to get wrong — and the snake_case case pins that both run the same
    /// segment canonicalization.
    #[test]
    fn both_gleam_module_derivations_agree_on_the_same_document() {
        for (root, document, expected) in [
            ("models", "models/domain/widget.gleam", "domain/widget"),
            (".", "widget.gleam", "widget"),
            (
                "models",
                "models/data_model/order_processing.gleam",
                "data-model/order-processing",
            ),
        ] {
            let discovery = discovery_module_name(root, document);
            let compile = compile_module_name(root, document);
            assert_eq!(
                discovery, compile,
                "discovery and compile disagree on the module name for `{document}` under root `{root}`"
            );
            assert_eq!(
                discovery, expected,
                "unexpected module name for `{document}` under root `{root}`"
            );
        }
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
