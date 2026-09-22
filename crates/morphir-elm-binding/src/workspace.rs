//! Provider-owned completion of portable Elm workspace discovery.
//!
//! Portable discovery (`morphir_workspace::discover`) is language-neutral: it
//! cannot know how a source file becomes a module, so a synthesized
//! project's `name` and `exposedModules` are left empty. This module
//! delegates discovery to the portable engine — which keeps owning
//! confinement, budgets, ordering and diagnostics — and only fills the
//! fields an Elm-specific policy can supply, by post-processing the returned
//! snapshot.
//!
//! Unlike Gleam, Elm derives module identity primarily from a *declared*
//! module header inside the source, not from the file's path. This module
//! ports — byte-for-byte, not merely "equivalently" — the algorithm the CLI
//! used to run inline for a standalone single-file compile:
//! `elm_module_name`, `fallback_elm_module_name` and the package-name
//! formatting in `prepare_single_file_context`
//! (`crates/morphir/src/commands/compile.rs` in the parent repository).
//! Reimplementing this with Elm's own compiler parser is not automatically
//! equivalent — the parent's scanner accepts and rejects different inputs
//! than a real parser would, especially for malformed headers and the
//! fallback cases — so this is a direct port, not a rewrite.

use morphir_extension_sdk::prelude::*;
use morphir_workspace::{
    DiscoveryRequest, DiscoveryResponse, FileTree, ProjectOrigin, RelativePath,
};

use crate::ElmExtension;

impl Workspace for ElmExtension {
    fn discover(&self, request: DiscoveryRequest) -> Result<DiscoveryResponse> {
        // `morphir_workspace::discover` owns confinement, budgets, ordering
        // and diagnostics; nothing here reimplements any of that. Identity
        // synthesis below needs the submitted source's *text*, not just its
        // path, so the file tree is cloned before `discover` consumes the
        // request.
        let development_root = request.development_root.clone();
        Ok(synthesize_identity(
            morphir_workspace::discover(request),
            &development_root,
        ))
    }
}

/// Fills `name` and `exposedModules` on a synthesized project whose name
/// discovery left empty. A project that already has a name — explicit,
/// manifest-derived, or otherwise — is untouched, since a name discovery
/// filled in came from the caller and is not this provider's to change. A
/// discovery failure passes through unchanged: it is not this function's to
/// reinterpret.
///
/// `development_root` is the request's file tree, carried alongside the
/// response since [`morphir_workspace::discover`] only returns paths, never
/// document text, and deriving an Elm module name needs the declared header
/// inside the source.
fn synthesize_identity(
    response: DiscoveryResponse,
    development_root: &FileTree,
) -> DiscoveryResponse {
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
        // `validate_ad_hoc_selection` already confirmed `input` names a text
        // file in this same tree before a `Success` snapshot could exist, so
        // this is never actually missing; the fallback to an empty string
        // only guards against that invariant changing out from under this
        // function, not a real gap.
        let text = development_root.file_text(input).unwrap_or_default();

        let (package_name, module_name) = derive_identity(input, text);
        project.name = package_name;
        project.exposed_modules = Some(vec![module_name]);
    }

    DiscoveryResponse::Success { snapshot }
}

/// Derives a synthesized Elm project's package name and its single exposed
/// module name from `input`'s declared module header, falling back to its
/// filename and then to `"Main"`.
///
/// This ports, byte-for-byte, the algorithm the CLI used to run inline for a
/// standalone single-file compile (`elm_module_name`,
/// `fallback_elm_module_name`, and the package-name formatting in
/// `prepare_single_file_context`, all in the parent repository's
/// `crates/morphir/src/commands/compile.rs`): the declared module name —
/// including a `port module` or `effect module` header, and skipping a
/// leading nested block comment — wins when the source parses; otherwise the
/// file's stem is tried as a bare module path; otherwise the name is
/// `"Main"`. The package name is that module name ASCII-lowercased with `.`
/// replaced by `-`, nested under the placeholder `local` scope — deliberately
/// not the per-segment `_`-to-`-` canonicalization Gleam's path-derived
/// identity uses, since this is a port of the CLI's own simpler rule, not a
/// reuse of Gleam's.
///
/// Unlike Gleam's path-derived identity, this never fails: every branch
/// (declared name, filename fallback, `"Main"`) produces a name, so there is
/// no analogous "unsynthesizable" diagnostic here.
fn derive_identity(input: &RelativePath, text: &str) -> (String, String) {
    let module_name =
        elm_module_name(text).unwrap_or_else(|| fallback_elm_module_name(file_name(input)));
    let package_name = format!(
        "local/{}",
        module_name.to_ascii_lowercase().replace('.', "-")
    );
    (package_name, module_name)
}

/// The final `/`-separated segment of `path`'s wire form — the source file's
/// name, matching what `Path::file_name` would see given the same string.
fn file_name(path: &RelativePath) -> &str {
    path.as_str()
        .rsplit_once('/')
        .map_or(path.as_str(), |(_, name)| name)
}

/// Parses a declared Elm module name — `module`, `port module`, or `effect
/// module`, skipping one leading run of trivia including a nested block
/// comment — returning `None` when the source has no such declaration.
///
/// Ported from `elm_module_name` (parent `compile.rs:185`), with `Option`
/// replacing the parent's `Result<_, CliError>`: this provider never
/// surfaces the parse failure, since every caller falls back to a
/// filename-or-`"Main"` name instead of reporting it.
fn elm_module_name(source: &str) -> Option<String> {
    let mut offset = skip_elm_trivia(source, 0)?;

    let declaration_kind = if let Some(end) = elm_keyword_end(source, offset, "port") {
        offset = skip_elm_trivia(source, end)?;
        "port"
    } else if let Some(end) = elm_keyword_end(source, offset, "effect") {
        offset = skip_elm_trivia(source, end)?;
        "effect"
    } else {
        "module"
    };

    let module_end = elm_keyword_end(source, offset, "module")?;
    offset = skip_elm_trivia(source, module_end)?;

    let (module_name, module_end) = elm_module_path(source, offset)?;
    offset = skip_elm_trivia(source, module_end)?;
    let required_suffix = if declaration_kind == "effect" {
        "where"
    } else {
        "exposing"
    };
    elm_keyword_end(source, offset, required_suffix)?;

    Some(module_name)
}

/// Ported verbatim from `skip_elm_trivia` (parent `compile.rs:233`): advances
/// past whitespace, a BOM, `--` line comments and nested `{- -}` block
/// comments, returning `None` for an unterminated block comment.
fn skip_elm_trivia(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut offset = start;
    while offset < bytes.len() {
        if source[offset..].starts_with('\u{feff}') {
            offset += '\u{feff}'.len_utf8();
        } else if bytes[offset].is_ascii_whitespace() {
            offset += 1;
        } else if source[offset..].starts_with("--") {
            offset = source[offset + 2..]
                .find('\n')
                .map_or(bytes.len(), |line_end| offset + 2 + line_end + 1);
        } else if source[offset..].starts_with("{-") {
            let mut depth = 1_u32;
            offset += 2;
            while offset < bytes.len() && depth > 0 {
                if source[offset..].starts_with("{-") {
                    depth += 1;
                    offset += 2;
                } else if source[offset..].starts_with("-}") {
                    depth -= 1;
                    offset += 2;
                } else {
                    offset += source[offset..].chars().next()?.len_utf8();
                }
            }
            if depth != 0 {
                return None;
            }
        } else {
            break;
        }
    }
    Some(offset)
}

/// Ported verbatim from `elm_keyword_end` (parent `compile.rs:269`): matches
/// `keyword` at `offset` as a whole identifier, not merely a prefix.
fn elm_keyword_end(source: &str, offset: usize, keyword: &str) -> Option<usize> {
    let end = offset.checked_add(keyword.len())?;
    if !source.get(offset..)?.starts_with(keyword)
        || source[end..]
            .chars()
            .next()
            .is_some_and(is_elm_identifier_character)
    {
        return None;
    }
    Some(end)
}

/// Ported verbatim from `is_elm_identifier_character` (parent `compile.rs:282`).
fn is_elm_identifier_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

/// Ported verbatim from `elm_module_path` (parent `compile.rs:286`): a
/// dot-separated run of segments, each starting with an ASCII uppercase
/// letter.
fn elm_module_path(source: &str, start: usize) -> Option<(String, usize)> {
    let mut offset = start;
    let mut segments = Vec::new();
    loop {
        let first = source[offset..].chars().next()?;
        if !first.is_ascii_uppercase() {
            return None;
        }
        let segment_start = offset;
        offset += first.len_utf8();
        while let Some(character) = source[offset..].chars().next() {
            if !is_elm_identifier_character(character) {
                break;
            }
            offset += character.len_utf8();
        }
        segments.push(&source[segment_start..offset]);
        if !source[offset..].starts_with('.') {
            break;
        }
        offset += 1;
    }
    Some((segments.join("."), offset))
}

/// Ported from `fallback_elm_module_name` (parent `compile.rs:366`), taking a
/// bare filename instead of a filesystem `Path` since discovery only ever
/// hands this a wire-relative path segment, not a real filesystem path.
fn fallback_elm_module_name(file_name: &str) -> String {
    let stem = file_stem(file_name);
    elm_module_path(stem, 0)
        .filter(|(_, end)| *end == stem.len())
        .map(|(module_name, _)| module_name)
        .unwrap_or_else(|| "Main".to_owned())
}

/// The portion of `file_name` before its final `.`, matching
/// `std::path::Path::file_stem`'s documented rule: a name that starts with
/// `.` and has no other `.` has no extension, so the whole name is the stem.
fn file_stem(file_name: &str) -> &str {
    match file_name.rfind('.') {
        Some(0) => file_name,
        Some(index) => &file_name[..index],
        None => file_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use morphir_workspace::{
        DiscoveryPurpose, FileEntry, ProjectSnapshot, ProjectSource, ProjectState, SourceSelection,
        WORKSPACE_DISCOVERY_PROTOCOL, WORKSPACE_PROTOCOL_UNSUPPORTED, WorkspaceSnapshot,
        WorkspaceState,
    };
    use std::collections::BTreeMap;

    fn ad_hoc_request(root: &str, path: &str, text: &str) -> DiscoveryRequest {
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

    /// An explicitly named synthesized project is left exactly as discovery
    /// produced it — the name came from the caller, not from this provider.
    #[test]
    fn an_explicitly_named_synthesized_project_is_not_touched() {
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
        assert_eq!(project.exposed_modules, None);
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
        request.protocol_version = 999;

        let response = ElmExtension
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
    /// belt-and-braces. This bypasses `ElmExtension::discover` and drives
    /// `synthesize_identity` directly, since portable discovery itself never
    /// produces an empty-named manifest project.
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
        let development_root = FileTree {
            entries: BTreeMap::new(),
        };

        let response =
            synthesize_identity(DiscoveryResponse::Success { snapshot }, &development_root);

        let DiscoveryResponse::Success { snapshot } = response else {
            panic!("expected a successful discovery response");
        };
        let project = &snapshot.projects[0];
        assert_eq!(project.name, "");
        assert_eq!(project.exposed_modules, None);
        assert_eq!(project.state, ProjectState::Unloaded);
        assert_eq!(snapshot.state, WorkspaceState::Open);
    }
}
