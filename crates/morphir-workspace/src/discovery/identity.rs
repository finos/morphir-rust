//! Provider-tier completion of ad-hoc discovery.
//!
//! Portable discovery cannot know how a source becomes a module, so it leaves
//! an ad-hoc project's name and exposure unset and does not judge how many
//! distinct sources a selection holds. A provider supplies that knowledge as
//! a [`SourceIdentity`], and [`discover_with_identity`] applies it the same
//! way for every language: the checks, their order and their diagnostic codes
//! are shared, and only the language policy differs.

use std::collections::BTreeMap;

use crate::{
    DiagnosticSeverity, DiscoveryFailure, DiscoveryPurpose, DiscoveryRequest, DiscoveryResponse,
    ProjectState, RelativePath, WORKSPACE_PROJECT_NAME_INVALID,
    WORKSPACE_SELECTION_MODULE_COLLISION, WORKSPACE_SELECTION_NAME_REQUIRED, WorkspaceDiagnostic,
    WorkspaceState,
};

use super::discover;

/// How one language turns selected sources into package and module identity.
pub trait SourceIdentity {
    /// The module a selected source defines, or a diagnostic `(code, message)`
    /// explaining why this language cannot name it.
    ///
    /// `root` is the selection's root, which a path-derived language measures
    /// module names from. `text` is the source's complete contents.
    fn module_name(
        &self,
        root: &RelativePath,
        path: &RelativePath,
        text: &str,
    ) -> Result<String, (String, String)>;

    /// The package name synthesized for an unnamed single-source selection
    /// whose one module is `module`.
    fn synthesized_package_name(&self, module: &str) -> String;

    /// Checks an explicit package name against this language's package
    /// contract, returning why it is rejected.
    fn check_package_name(&self, name: &str) -> Result<(), String>;

    /// How many distinct sources `paths` holds. Discovery has already
    /// rejected repeated paths, so the default counts them.
    fn distinct_sources(&self, paths: &[RelativePath]) -> usize {
        paths.len()
    }
}

/// Discovers a workspace and completes an ad-hoc project with `identity`.
///
/// A manifest-projects request, and any discovery failure, pass through
/// unchanged. For an ad-hoc project, in order:
///
/// 1. an explicit name must satisfy the package contract
///    (`workspace.project-name.invalid`);
/// 2. an unnamed selection must hold exactly one distinct source
///    (`workspace.selection.name-required`);
/// 3. every source is named; a source the language cannot name is a project
///    diagnostic, and leaves name and exposure unset;
/// 4. no two sources may name the same module
///    (`workspace.selection.module-collision`);
/// 5. exposure is every selected module, in selection order, and an unnamed
///    project is named for its one module.
///
/// A derived name is not checked against the package contract. It is the
/// historical identity, and a name that never satisfied the contract keeps
/// failing where it always did: at compile.
#[must_use]
pub fn discover_with_identity(
    request: DiscoveryRequest,
    identity: &impl SourceIdentity,
) -> DiscoveryResponse {
    // Only the selected sources' texts are needed, and `discover` consumes
    // the request, so capture exactly those rather than cloning the tree.
    let (selected, texts): (Vec<RelativePath>, BTreeMap<RelativePath, String>) =
        match &request.purpose {
            DiscoveryPurpose::AdHocSources { sources, .. } => (
                sources.paths.clone(),
                sources
                    .paths
                    .iter()
                    .filter_map(|path| {
                        request
                            .development_root
                            .file_text(path)
                            .map(|text| (path.clone(), text.to_owned()))
                    })
                    .collect(),
            ),
            DiscoveryPurpose::ManifestProjects => return discover(request),
        };
    match discover(request) {
        DiscoveryResponse::Success { mut snapshot } => {
            for project in &mut snapshot.projects {
                // An ad-hoc snapshot holds exactly one project, and it selects
                // the request's sources whatever its origin.
                if let Err(error) = complete_project(project, &selected, &texts, identity) {
                    return DiscoveryResponse::Failure { error };
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
        failure @ DiscoveryResponse::Failure { .. } => failure,
    }
}

fn complete_project(
    project: &mut crate::ProjectSnapshot,
    inputs: &[RelativePath],
    texts: &BTreeMap<RelativePath, String>,
    identity: &impl SourceIdentity,
) -> Result<(), DiscoveryFailure> {
    let root = project.relative_path.clone();
    if !project.name.is_empty() {
        identity
            .check_package_name(&project.name)
            .map_err(|reason| DiscoveryFailure {
                code: WORKSPACE_PROJECT_NAME_INVALID.to_owned(),
                message: format!("project name `{}` is invalid: {reason}", project.name),
                path: Some(root.clone()),
            })?;
    } else if identity.distinct_sources(inputs) != 1 {
        return Err(DiscoveryFailure {
            code: WORKSPACE_SELECTION_NAME_REQUIRED.to_owned(),
            message: format!(
                "ad-hoc selection rooted at `{}` selects {} sources but has no explicit name; an unnamed synthesized selection must select exactly one source",
                root.as_str(),
                inputs.len()
            ),
            path: Some(root),
        });
    }

    let mut modules = Vec::with_capacity(inputs.len());
    for input in inputs {
        let text = texts.get(input).map_or("", String::as_str);
        match identity.module_name(&root, input, text) {
            Ok(module) => modules.push((input, module)),
            Err((code, message)) => {
                project.state = ProjectState::Error;
                project.diagnostics.push(WorkspaceDiagnostic {
                    severity: DiagnosticSeverity::Error,
                    code,
                    message,
                    path: Some(input.clone()),
                    project_path: Some(root.clone()),
                });
            }
        }
    }
    if project.state == ProjectState::Error {
        return Ok(());
    }

    let mut seen: BTreeMap<&str, &RelativePath> = BTreeMap::new();
    for (path, module) in &modules {
        if let Some(first) = seen.insert(module.as_str(), path) {
            return Err(DiscoveryFailure {
                code: WORKSPACE_SELECTION_MODULE_COLLISION.to_owned(),
                message: format!(
                    "selected sources `{}` and `{}` both define module `{module}`",
                    first.as_str(),
                    path.as_str()
                ),
                path: Some((*path).clone()),
            });
        }
    }

    let modules: Vec<String> = modules.into_iter().map(|(_, module)| module).collect();
    if project.name.is_empty() {
        project.name = identity.synthesized_package_name(&modules[0]);
    }
    if project.exposed_modules.is_none() {
        project.exposed_modules = Some(modules);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::{FileEntry, FileTree, ProjectOrigin, ProjectSource, SourceSelection};

    /// Names a module for the file's stem, uppercased; refuses a stem holding
    /// `-`; accepts only lowercase package names.
    struct Stems;

    impl SourceIdentity for Stems {
        fn module_name(
            &self,
            _root: &RelativePath,
            path: &RelativePath,
            _text: &str,
        ) -> Result<String, (String, String)> {
            let stem = path
                .as_str()
                .rsplit('/')
                .next()
                .and_then(|name| name.split('.').next())
                .unwrap_or_default();
            if stem.contains('-') {
                return Err(("test.bad-stem".into(), format!("bad stem `{stem}`")));
            }
            Ok(stem.to_ascii_uppercase())
        }

        fn synthesized_package_name(&self, module: &str) -> String {
            format!("local/{}", module.to_ascii_lowercase())
        }

        fn check_package_name(&self, name: &str) -> Result<(), String> {
            if name.chars().any(|character| character.is_ascii_uppercase()) {
                Err("must be lowercase".into())
            } else {
                Ok(())
            }
        }
    }

    fn request(paths: &[&str], name: Option<&str>, project: ProjectSource) -> DiscoveryRequest {
        let mut entries = BTreeMap::from([
            (RelativePath::root(), FileEntry::Directory),
            (
                RelativePath::parse("morphir.toml").unwrap(),
                FileEntry::File {
                    text: String::new(),
                },
            ),
        ]);
        for path in paths {
            entries.insert(
                RelativePath::parse(*path).unwrap(),
                FileEntry::File {
                    text: String::new(),
                },
            );
        }
        DiscoveryRequest {
            protocol_version: crate::workspace_discovery_protocol(),
            development_root: FileTree { entries },
            morphir_home: None,
            system_config: None,
            environment: BTreeMap::new(),
            cli_overlay: name.map_or_else(|| json!({}), |name| json!({"project": {"name": name}})),
            purpose: DiscoveryPurpose::AdHocSources {
                project,
                sources: SourceSelection {
                    root: RelativePath::parse("src").unwrap(),
                    paths: paths
                        .iter()
                        .map(|path| RelativePath::parse(*path).unwrap())
                        .collect(),
                },
                language_id: "test".into(),
            },
        }
    }

    fn synthesized(paths: &[&str], name: Option<&str>) -> DiscoveryResponse {
        discover_with_identity(request(paths, name, ProjectSource::Synthesized), &Stems)
    }

    #[test]
    fn a_single_unnamed_source_is_named_for_its_module() {
        let snapshot = synthesized(&["src/a.x"], None).into_result().unwrap();

        let project = &snapshot.projects[0];
        assert_eq!(project.name, "local/a");
        assert_eq!(project.exposed_modules, Some(vec!["A".to_owned()]));
    }

    #[test]
    fn a_named_selection_exposes_every_module_in_selection_order() {
        let snapshot = synthesized(&["src/b.x", "src/a.x"], Some("acme/pkg"))
            .into_result()
            .unwrap();

        let project = &snapshot.projects[0];
        assert_eq!(project.name, "acme/pkg");
        assert_eq!(
            project.exposed_modules,
            Some(vec!["B".to_owned(), "A".to_owned()])
        );
    }

    #[test]
    fn an_unnamed_multi_source_selection_requires_a_name() {
        let error = synthesized(&["src/a.x", "src/b.x"], None)
            .into_result()
            .unwrap_err();

        assert_eq!(error.code, WORKSPACE_SELECTION_NAME_REQUIRED);
    }

    #[test]
    fn an_explicit_name_is_checked_against_the_package_contract() {
        let error = synthesized(&["src/a.x"], Some("Acme/Pkg"))
            .into_result()
            .unwrap_err();

        assert_eq!(error.code, WORKSPACE_PROJECT_NAME_INVALID);
        assert!(error.message.contains("must be lowercase"));
    }

    /// The contract check runs before cardinality would matter, and a bad
    /// name wins over a bad module: the first thing wrong is reported.
    #[test]
    fn the_contract_check_precedes_module_naming() {
        let error = synthesized(&["src/a-b.x"], Some("Acme/Pkg"))
            .into_result()
            .unwrap_err();

        assert_eq!(error.code, WORKSPACE_PROJECT_NAME_INVALID);
    }

    #[test]
    fn two_sources_naming_one_module_collide() {
        let error = synthesized(&["src/a.x", "src/sub/a.y"], Some("acme/pkg"))
            .into_result()
            .unwrap_err();

        assert_eq!(error.code, WORKSPACE_SELECTION_MODULE_COLLISION);
        assert!(error.message.contains("src/a.x"));
        assert!(error.message.contains("src/sub/a.y"));
    }

    #[test]
    fn an_unnameable_source_is_a_project_diagnostic() {
        let snapshot = synthesized(&["src/a-b.x"], None).into_result().unwrap();

        assert_eq!(snapshot.state, WorkspaceState::Error);
        let project = &snapshot.projects[0];
        assert_eq!(project.state, ProjectState::Error);
        assert_eq!(project.name, "");
        assert_eq!(project.exposed_modules, None);
        assert_eq!(project.diagnostics[0].code, "test.bad-stem");
    }

    #[test]
    fn a_manifest_origin_selection_keeps_its_name_and_derives_exposure() {
        let manifest = RelativePath::parse("morphir.toml").unwrap();
        let response = discover_with_identity(
            request(
                &["src/a.x"],
                Some("acme/widgets"),
                ProjectSource::Manifest {
                    path: manifest.clone(),
                },
            ),
            &Stems,
        );

        let project = &response.into_result().unwrap().projects[0];
        assert_eq!(project.name, "acme/widgets");
        assert_eq!(project.origin, ProjectOrigin::Manifest { path: manifest });
        assert_eq!(project.exposed_modules, Some(vec!["A".to_owned()]));
    }
}
