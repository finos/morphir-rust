//! Provider-owned completion of portable Elm workspace discovery.
//!
//! Portable discovery (`morphir_workspace::discover`) is language-neutral: it
//! cannot know how a source file becomes a module, so an ad-hoc project's
//! `name` and `exposedModules` are left for the provider. The shared
//! provider-tier rules — package contract, cardinality, collisions — live in
//! [`morphir_workspace::discover_with_identity`]; this module supplies only the
//! Elm policy they apply.
//!
//! Unlike Gleam, Elm derives module identity primarily from a *declared*
//! module header inside the source, not from the file's path. Scanning that
//! header is a separate, purely lexical concern and lives in
//! [`module_header`]. The package-name formatting is a byte-for-byte port of
//! the rule the CLI used to run inline for a standalone single-file compile;
//! see [`module_header`] for the scanner half of that port and why it may not
//! be swapped for a real Elm parser.

mod module_header;

use morphir_extension_sdk::prelude::*;
use morphir_workspace::{DiscoveryRequest, DiscoveryResponse, RelativePath, SourceIdentity};

use crate::ElmExtension;
use crate::names::words;
use module_header::{elm_module_name, fallback_elm_module_name};

impl Workspace for ElmExtension {
    fn discover(&self, request: DiscoveryRequest) -> Result<DiscoveryResponse> {
        Ok(morphir_workspace::discover_with_identity(
            request,
            &ElmIdentity,
        ))
    }
}

/// Elm's answer to how a selected source becomes package and module identity.
struct ElmIdentity;

impl SourceIdentity for ElmIdentity {
    /// The declared module name — including a `port module` or `effect
    /// module` header, and skipping leading nested block comments — when the
    /// source has one; otherwise the file's stem as a bare module path;
    /// otherwise `"Main"`. Every branch produces a name, so Elm never reports
    /// a source it cannot name.
    fn module_name(
        &self,
        _root: &RelativePath,
        path: &RelativePath,
        text: &str,
    ) -> std::result::Result<String, (String, String)> {
        Ok(elm_module_name(text).unwrap_or_else(|| fallback_elm_module_name(file_name(path))))
    }

    /// The module name ASCII-lowercased with `.` replaced by `-`, under the
    /// placeholder `local` scope — deliberately not the per-segment
    /// `_`-to-`-` canonicalization Gleam uses, since this ports the CLI's own
    /// simpler rule.
    fn synthesized_package_name(&self, module: &str) -> String {
        format!("local/{}", module.to_ascii_lowercase().replace('.', "-"))
    }

    /// The Elm package-name contract every Elm workspace provider shares:
    /// split on both `/` and `.`, trim each piece and drop empty ones, split
    /// each piece into words as morphir-elm `Name.fromString` does, and join
    /// words with `-` and pieces with `/`. So `My.Package`, `My/Package` and
    /// `my/package` all report `my/package`, which is also the package path
    /// each of them compiles to.
    fn normalize_package_name(&self, name: &str) -> std::result::Result<String, String> {
        let pieces: Vec<&str> = name
            .split(['/', '.'])
            .map(str::trim)
            .filter(|piece| !piece.is_empty())
            .collect();
        if pieces.is_empty() {
            return Err("it names no package path segments".into());
        }
        let mut segments = Vec::with_capacity(pieces.len());
        for piece in pieces {
            let piece_words = words(piece);
            if piece_words.is_empty() {
                return Err(format!("segment `{piece}` has no letters or digits"));
            }
            segments.push(piece_words.join("-"));
        }
        Ok(segments.join("/"))
    }
}

/// The final `/`-separated segment of `path`'s wire form — the source file's
/// name, matching what `Path::file_name` would see given the same string.
fn file_name(path: &RelativePath) -> &str {
    path.as_str()
        .rsplit_once('/')
        .map_or(path.as_str(), |(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_workspace::{
        DiscoveryPurpose, FileEntry, FileTree, ProjectSource, ProjectState, SourceSelection,
        WORKSPACE_PROTOCOL_UNSUPPORTED, WorkspaceState,
    };
    use std::collections::BTreeMap;

    fn ad_hoc_request(root: &str, path: &str, text: &str) -> DiscoveryRequest {
        let root = RelativePath::parse(root).expect("a confined wire path");
        let source = RelativePath::parse(path).expect("a confined wire path");
        DiscoveryRequest {
            protocol_version: morphir_workspace::workspace_discovery_protocol(),
            development_root: FileTree {
                entries: BTreeMap::from([
                    (RelativePath::root(), FileEntry::Directory),
                    (
                        source.clone(),
                        FileEntry::File {
                            text: text.to_owned(),
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
                language_id: "elm".to_owned(),
            },
        }
    }

    /// A synthesized project's name is established by the provider from the
    /// source's *declared* module name — unlike Gleam, path segments (the
    /// `src` root here) play no part. Pinned against
    /// `a_synthesized_package_is_named_for_its_declared_module` in the
    /// parent's `single_file_compatibility.rs`: the same source produces the
    /// same package path (`local/acme-widget`) and the same single exposed
    /// module (`Acme.Widget`, not lowercased — only the package name is).
    #[test]
    fn a_synthesized_elm_project_is_named_for_its_declared_module() {
        let request = ad_hoc_request(
            "src",
            "src/Widget.elm",
            "module Acme.Widget exposing (Size)\n\n\ntype alias Size =\n    Int\n",
        );

        let response = ElmExtension
            .discover(request)
            .expect("discovery of a valid ad-hoc selection should succeed");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        assert_eq!(snapshot.projects.len(), 1);
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "local/acme-widget");
        assert_eq!(
            project.exposed_modules,
            Some(vec!["Acme.Widget".to_owned()])
        );
        assert_eq!(project.state, ProjectState::Unloaded);
        assert_eq!(snapshot.state, WorkspaceState::Open);
    }

    /// Mirrors the parent's `a_port_module_is_named_like_a_plain_declared_module`:
    /// a `port module` header is named exactly as the equivalent plain
    /// `module` header would be. The `port` keyword and the skipped `port
    /// sendMessage : ...` value declaration do not change the derived
    /// identity.
    #[test]
    fn a_port_module_is_named_like_a_plain_declared_module() {
        let request = ad_hoc_request(
            ".",
            "Ports.elm",
            "port module App.Ports exposing (Size, sendMessage)\n\nport sendMessage : String -> Cmd msg\n\n\ntype alias Size =\n    Int\n",
        );

        let response = ElmExtension
            .discover(request)
            .expect("discovery of a valid ad-hoc selection should succeed");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "local/app-ports");
        assert_eq!(project.exposed_modules, Some(vec!["App.Ports".to_owned()]));
    }

    /// An `effect module ... where { ... } exposing (...)` header is named
    /// the same way: the scanner only checks for the `where` keyword
    /// following the module path, not the manager-record clause after it.
    #[test]
    fn an_effect_module_is_named_like_a_plain_declared_module() {
        let request = ad_hoc_request(
            ".",
            "Bar.elm",
            "effect module Foo.Bar where { command = MyCmd, subscription = MySub } exposing (Size)\n\n\ntype alias Size =\n    Int\n",
        );

        let response = ElmExtension
            .discover(request)
            .expect("discovery of a valid ad-hoc selection should succeed");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "local/foo-bar");
        assert_eq!(project.exposed_modules, Some(vec!["Foo.Bar".to_owned()]));
    }

    /// Mirrors the parent's
    /// `a_nested_block_comment_before_the_module_declaration_does_not_change_identity`:
    /// a nested block comment before the `module` line is skipped entirely
    /// by `skip_elm_trivia` and has no effect on the derived identity.
    #[test]
    fn a_nested_block_comment_before_the_module_declaration_does_not_change_identity() {
        let request = ad_hoc_request(
            ".",
            "Widget.elm",
            "{- outer {- nested -} comment -}\nmodule Acme.Widget exposing (Size)\n\n\ntype alias Size =\n    Int\n",
        );

        let response = ElmExtension
            .discover(request)
            .expect("discovery of a valid ad-hoc selection should succeed");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "local/acme-widget");
        assert_eq!(
            project.exposed_modules,
            Some(vec!["Acme.Widget".to_owned()])
        );
    }

    /// A source with no parseable `module` declaration falls back to a
    /// module path built from its filename, exactly as
    /// `fallback_elm_module_name` (parent `compile.rs:366`) does. This
    /// discovery-time fallback is exercised even though the parent's own
    /// characterisation suite shows the built-in provider's *compile* stage
    /// separately rejects the same source (see
    /// `a_file_without_a_module_declaration_fails_to_compile` and its
    /// commentary in `single_file_compatibility.rs`): discovery and compile
    /// are different steps, and this task's gate is derivation parity at
    /// discovery time only.
    #[test]
    fn a_source_with_no_module_declaration_falls_back_to_its_filename() {
        let request = ad_hoc_request(".", "Widget.elm", "x = 1\n");

        let response = ElmExtension
            .discover(request)
            .expect("discovery of a valid ad-hoc selection should succeed");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "local/widget");
        assert_eq!(project.exposed_modules, Some(vec!["Widget".to_owned()]));
    }

    /// A source with no module declaration AND an illegal filename (a stem
    /// that cannot itself parse as a module path, here because of the
    /// hyphen) falls all the way back to `"Main"`, exactly as
    /// `fallback_elm_module_name` does when `elm_module_path` fails to
    /// consume the whole stem.
    #[test]
    fn a_source_with_no_module_declaration_and_an_illegal_filename_falls_back_to_main() {
        let request = ad_hoc_request(".", "not-a-module.elm", "x = 1\n");

        let response = ElmExtension
            .discover(request)
            .expect("discovery of a valid ad-hoc selection should succeed");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "local/main");
        assert_eq!(project.exposed_modules, Some(vec!["Main".to_owned()]));
    }

    /// A declared module segment containing an underscore keeps it in the
    /// package name: unlike Gleam's path-derived canonicalization (which
    /// maps `_` to `-` per segment via `Name::from`), the CLI's own
    /// derivation only ASCII-lowercases and replaces `.` with `-` — it never
    /// touches `_`. This pins that this provider reproduces the CLI's
    /// simpler rule rather than Gleam's segment canonicalization.
    #[test]
    fn a_declared_module_name_with_an_underscore_segment_keeps_the_underscore() {
        let request = ad_hoc_request(
            ".",
            "Foo.elm",
            "module Foo_Bar exposing (X)\n\n\ntype alias X =\n    Int\n",
        );

        let response = ElmExtension
            .discover(request)
            .expect("discovery of a valid ad-hoc selection should succeed");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "local/foo_bar");
        assert_eq!(project.exposed_modules, Some(vec!["Foo_Bar".to_owned()]));
    }

    /// An explicit name already in normal form survives untouched, but
    /// exposure is still derived, because the two
    /// fields answer different questions. `--package-name` says what to call
    /// the package; it says nothing about which modules the package
    /// publishes, and the single submitted file's declared module is exactly
    /// as derivable named as unnamed. Pinned so that the same source cannot
    /// advertise `["Acme.Widget"]` unnamed and `null` named, which would make
    /// an unrelated flag change the shape of the response.
    #[test]
    fn an_explicit_name_survives_but_exposure_is_still_derived() {
        let mut request = ad_hoc_request(
            ".",
            "Widget.elm",
            "module Acme.Widget exposing (Size)\n\n\ntype alias Size =\n    Int\n",
        );
        request.cli_overlay = serde_json::json!({ "project": { "name": "acme/widgets" } });

        let response = ElmExtension
            .discover(request)
            .expect("discovery of a valid ad-hoc selection should succeed");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "acme/widgets");
        assert_eq!(
            project.exposed_modules,
            Some(vec!["Acme.Widget".to_owned()])
        );
    }

    /// A *named* selection may hold more than one source, and exposes every
    /// module it selects, in selection order. An unset `exposedModules` would
    /// mean "expose everything the frontend found", which says something
    /// weaker than an explicit selection means.
    #[test]
    fn a_named_multi_source_selection_exposes_every_module() {
        let root = RelativePath::parse("src").expect("a confined wire path");
        let first = RelativePath::parse("src/Widget.elm").expect("a confined wire path");
        let second = RelativePath::parse("src/Gadget.elm").expect("a confined wire path");
        let request = DiscoveryRequest {
            protocol_version: morphir_workspace::workspace_discovery_protocol(),
            development_root: FileTree {
                entries: BTreeMap::from([
                    (RelativePath::root(), FileEntry::Directory),
                    (
                        first.clone(),
                        FileEntry::File {
                            text: "module Acme.Widget exposing (Size)\n".to_owned(),
                        },
                    ),
                    (
                        second.clone(),
                        FileEntry::File {
                            text: "module Acme.Gadget exposing (Size)\n".to_owned(),
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
                language_id: "elm".to_owned(),
            },
        };

        let response = ElmExtension
            .discover(request)
            .expect("discovery of a valid named ad-hoc selection should succeed");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "acme/widgets");
        assert_eq!(
            project.exposed_modules,
            Some(vec!["Acme.Widget".to_owned(), "Acme.Gadget".to_owned()])
        );
        assert_eq!(project.state, ProjectState::Unloaded);
    }

    /// A discovery failure is passed through unchanged, never reinterpreted.
    /// Asserting the variant alone would also pass a provider that rewrote
    /// the failure's code or message while keeping it a `Failure`, so this
    /// pins the exact code `morphir_workspace::discover` produces.
    #[test]
    fn a_discovery_failure_passes_through_unchanged() {
        let mut request = ad_hoc_request(
            ".",
            "Widget.elm",
            "module Acme.Widget exposing (Size)\n\n\ntype alias Size =\n    Int\n",
        );
        request.protocol_version = morphir_workspace::Version::new(999, 0, 0);

        let response = ElmExtension
            .discover(request)
            .expect("the typed call itself should not error");

        let DiscoveryResponse::Failure { error } = response else {
            panic!("expected a discovery failure");
        };
        assert_eq!(error.code, WORKSPACE_PROTOCOL_UNSUPPORTED);
    }

    /// A manifest-projects request is portable discovery's alone: the
    /// provider adds no identity to a project its manifest already describes.
    #[test]
    fn a_manifest_projects_request_is_not_touched() {
        let request = DiscoveryRequest {
            protocol_version: morphir_workspace::workspace_discovery_protocol(),
            development_root: FileTree {
                entries: BTreeMap::from([
                    (RelativePath::root(), FileEntry::Directory),
                    (
                        RelativePath::parse("morphir.toml").expect("a confined wire path"),
                        FileEntry::File {
                            text: "[project]\nname = 'acme/widgets'\n".to_owned(),
                        },
                    ),
                ]),
            },
            morphir_home: None,
            system_config: None,
            environment: BTreeMap::new(),
            cli_overlay: serde_json::json!({}),
            purpose: DiscoveryPurpose::ManifestProjects,
        };

        let response = ElmExtension
            .discover(request)
            .expect("the typed call itself should not error");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "acme/widgets");
        assert_eq!(project.exposed_modules, None);
        assert_eq!(snapshot.state, WorkspaceState::Open);
    }

    /// Discovers `Widget.elm` under the explicit `name`.
    fn discover_named(name: &str) -> DiscoveryResponse {
        let mut request = ad_hoc_request(".", "Widget.elm", "module Acme.Widget exposing (..)\n");
        request.cli_overlay = serde_json::json!({ "project": { "name": name } });
        ElmExtension
            .discover(request)
            .expect("the typed call itself should not error")
    }

    /// Every accepted row of the shared Elm package-name contract: the name
    /// is split on `.` and `/`, each piece into morphir-elm `Name.fromString`
    /// words, and the snapshot reports the normal form.
    #[test]
    fn an_explicit_name_is_reported_in_its_normal_form() {
        let cases = [
            ("acme/widgets", "acme/widgets"),
            ("finos/morphir-sdk", "finos/morphir-sdk"),
            ("My.Package", "my/package"),
            ("My/Package", "my/package"),
            ("Documentation.Decoration", "documentation/decoration"),
            ("Morphir.Reference.Model", "morphir/reference/model"),
            ("morphir/sdk.core", "morphir/sdk/core"),
            ("Acme/Widgets", "acme/widgets"),
            ("acme/my_widgets", "acme/my-widgets"),
            ("MyPackage", "my-package"),
            ("my-package", "my-package"),
            ("local/mywidget", "local/mywidget"),
            ("SDK/v2", "s-d-k/v-2"),
            ("acme//widgets", "acme/widgets"),
            (" acme / widgets ", "acme/widgets"),
            ("a.b/c", "a/b/c"),
            // The trim set is Unicode White_Space, which includes NEL.
            ("acme/\u{0085}/widgets", "acme/widgets"),
        ];
        for (name, expected) in cases {
            let DiscoveryResponse::Success { snapshot } = discover_named(name) else {
                panic!("expected `{name}` to be accepted");
            };
            assert_eq!(
                snapshot.projects[0].name, expected,
                "normal form of `{name}`"
            );
        }
    }

    /// The normal form is its own normal form.
    #[test]
    fn the_normal_form_is_idempotent() {
        for name in ["my/package", "s-d-k/v-2", "acme/my-widgets", "a/b/c"] {
            assert_eq!(
                ElmIdentity.normalize_package_name(name),
                Ok(name.to_owned())
            );
        }
    }

    /// Every refused row of the contract, with its exact message.
    #[test]
    fn an_explicit_name_that_spells_no_package_is_refused() {
        let cases = [
            (
                "/./",
                "project name `/./` is invalid: it names no package path segments",
            ),
            (
                "acme/_",
                "project name `acme/_` is invalid: segment `_` has no letters or digits",
            ),
            (
                "acme/-/x",
                "project name `acme/-/x` is invalid: segment `-` has no letters or digits",
            ),
            // BOM is not White_Space, so it is kept and has no words.
            (
                "acme/\u{feff}/widgets",
                "project name `acme/\u{feff}/widgets` is invalid: segment `\u{feff}` has no letters or digits",
            ),
        ];
        for (name, message) in cases {
            let DiscoveryResponse::Failure { error } = discover_named(name) else {
                panic!("expected `{name}` to be refused");
            };
            assert_eq!(
                error.code,
                morphir_workspace::WORKSPACE_PROJECT_NAME_INVALID
            );
            assert_eq!(error.message, message);
        }
    }

    /// A blank name keeps failing in portable discovery, before this policy.
    #[test]
    fn a_blank_explicit_name_is_empty() {
        let DiscoveryResponse::Failure { error } = discover_named("") else {
            panic!("expected a blank name to be refused");
        };
        assert_eq!(error.code, morphir_workspace::WORKSPACE_PROJECT_NAME_EMPTY);
    }

    /// End to end: a two-file selection named `My.Package` reports
    /// `my/package` and still exposes both modules in selection order.
    #[test]
    fn a_named_multi_source_selection_reports_the_normal_form() {
        let mut request = ad_hoc_request(".", "A.elm", "module Acme.Widget exposing (..)\n");
        let second = RelativePath::parse("B.elm").expect("a confined wire path");
        request.development_root.entries.insert(
            second.clone(),
            FileEntry::File {
                text: "module Acme.Gadget exposing (..)\n".to_owned(),
            },
        );
        request.cli_overlay = serde_json::json!({ "project": { "name": "My.Package" } });
        if let DiscoveryPurpose::AdHocSources { sources, .. } = &mut request.purpose {
            sources.paths.push(second);
        }

        let response = ElmExtension
            .discover(request)
            .expect("the typed call itself should not error");

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "my/package");
        assert_eq!(
            project.exposed_modules,
            Some(vec!["Acme.Widget".to_owned(), "Acme.Gadget".to_owned()])
        );
    }

    /// Two files declaring one module are a collision the provider reports:
    /// portable discovery sees two different paths and cannot tell.
    #[test]
    fn two_files_declaring_one_module_collide() {
        let mut request = ad_hoc_request(".", "A.elm", "module Acme.Widget exposing (..)\n");
        let second = RelativePath::parse("B.elm").expect("a confined wire path");
        request.development_root.entries.insert(
            second.clone(),
            FileEntry::File {
                text: "module Acme.Widget exposing (..)\n".to_owned(),
            },
        );
        request.cli_overlay = serde_json::json!({ "project": { "name": "acme/widgets" } });
        if let DiscoveryPurpose::AdHocSources { sources, .. } = &mut request.purpose {
            sources.paths.push(second);
        }

        let response = ElmExtension
            .discover(request)
            .expect("the typed call itself should not error");

        let DiscoveryResponse::Failure { error } = response else {
            panic!("expected a discovery failure");
        };
        assert_eq!(
            error.code,
            morphir_workspace::WORKSPACE_SELECTION_MODULE_COLLISION
        );
    }
}
