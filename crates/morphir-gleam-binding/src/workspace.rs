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
use morphir_workspace::{DiscoveryRequest, DiscoveryResponse, RelativePath, SourceIdentity};

use crate::{GleamExtension, canonicalize_gleam_module_segments};

/// Stable diagnostic code for a synthesized source whose path this provider
/// cannot turn into a valid Gleam module name (for example, a filename
/// containing a hyphen).
const GLEAM_WORKSPACE_INVALID_MODULE_PATH: &str = "gleam.workspace.invalid-module-path";

impl Workspace for GleamExtension {
    fn discover(&self, request: DiscoveryRequest) -> Result<DiscoveryResponse> {
        // The shared provider-tier rules live in `discover_with_identity`;
        // this provider supplies only Gleam's path-derived identity, which
        // needs no source text.
        Ok(morphir_workspace::discover_with_identity(
            request,
            &GleamIdentity,
        ))
    }
}

/// Gleam's answer to how a selected source becomes package and module identity.
struct GleamIdentity;

impl SourceIdentity for GleamIdentity {
    fn module_name(
        &self,
        root: &RelativePath,
        path: &RelativePath,
        _text: &str,
    ) -> std::result::Result<String, (String, String)> {
        module_path(root, path)
            .map_err(|message| (GLEAM_WORKSPACE_INVALID_MODULE_PATH.to_owned(), message))
    }

    /// The module path under the placeholder `local` scope, with `/` joins
    /// flattened to `-`, since a package name is a single scoped segment, not
    /// a nested path.
    ///
    /// Two different selections can collide on this name: segment
    /// canonicalization already maps `_` to `-` per segment, and flattening
    /// compounds that across segments, so `a/b.gleam` and `a_b.gleam` both
    /// yield `local/a-b`. Only an unnamed single-source selection is named
    /// this way, so the collision is between separate invocations, never
    /// within one snapshot.
    fn synthesized_package_name(&self, module: &str) -> String {
        format!("local/{}", module.replace('/', "-"))
    }

    fn check_package_name(&self, name: &str) -> std::result::Result<(), String> {
        crate::validate_package_name(name).map(|_| ())
    }
}

/// Derives a selected source's module path from its path relative to `root`.
///
/// Gleam has no module header, so — unlike Elm's declared-name-then-filename
/// policy — this always derives from the path: the segments between `root`
/// and the file, canonicalized the same way a compile-time document URI is
/// (snake_case segments become kebab-case, joined by `/`).
fn module_path(root: &RelativePath, input: &RelativePath) -> std::result::Result<String, String> {
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
    Ok(canonicalize_gleam_module_segments(&refs)?.to_string())
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
        DiagnosticSeverity as WorkspaceDiagnosticSeverity, DiscoveryPurpose, FileEntry, FileTree,
        ProjectSource, ProjectState, SourceSelection, WORKSPACE_DISCOVERY_PROTOCOL,
        WORKSPACE_PROJECT_NAME_INVALID, WORKSPACE_PROTOCOL_UNSUPPORTED, WorkspaceState,
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

    /// A *named* selection may hold more than one source, and exposes every
    /// module it selects, in selection order.
    #[test]
    fn a_named_multi_source_selection_exposes_every_module() {
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
        assert_eq!(
            project.exposed_modules,
            Some(vec!["domain/widget".to_owned(), "domain/gadget".to_owned()])
        );
        assert_eq!(project.state, ProjectState::Unloaded);
    }

    /// An explicit name goes through the same canonical-package contract a
    /// compile applies, so a name compile would refuse is refused here,
    /// before a provider is ever asked to compile.
    #[test]
    fn an_explicit_name_must_be_a_canonical_package_name() {
        let mut request = ad_hoc_request("models", "models/domain/widget.gleam");
        request.cli_overlay = serde_json::json!({ "project": { "name": "Acme/Widgets" } });

        let response = GleamExtension
            .discover(request)
            .expect("the typed call itself should not error");

        let DiscoveryResponse::Failure { error } = response else {
            panic!("expected a discovery failure");
        };
        assert_eq!(error.code, WORKSPACE_PROJECT_NAME_INVALID);
    }

    /// The module name discovery derives for a file, via `module_path`.
    fn discovery_module_name(root: &str, document: &str) -> String {
        module_path(
            &RelativePath::parse(root).expect("a confined wire path"),
            &RelativePath::parse(document).expect("a confined wire path"),
        )
        .expect("a derivable module path")
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
    /// places: `module_path` here, which names the module discovery
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
