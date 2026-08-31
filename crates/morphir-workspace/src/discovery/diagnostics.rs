//! Stable discovery diagnostics and deterministic ordering.

use std::collections::BTreeMap;

use crate::{
    DiagnosticSeverity, DiscoveryFailure, ProjectSnapshot, ProjectState, RelativePath,
    WORKSPACE_MEMBER_DUPLICATE_NAME, WorkspaceDiagnostic,
};

pub(super) fn error_project(
    directory: &RelativePath,
    anchor: Option<RelativePath>,
    code: &str,
    message: String,
) -> ProjectSnapshot {
    let diagnostic = WorkspaceDiagnostic {
        severity: DiagnosticSeverity::Error,
        code: code.to_owned(),
        message,
        path: anchor.clone(),
        project_path: Some(directory.clone()),
    };
    ProjectSnapshot {
        name: directory.as_str().to_owned(),
        version: None,
        relative_path: directory.clone(),
        config_anchor: anchor,
        source_directory: RelativePath::parse("src").expect("default source path is confined"),
        state: ProjectState::Error,
        diagnostics: vec![diagnostic],
    }
}

pub(super) fn duplicate_name_diagnostics(projects: &[ProjectSnapshot]) -> Vec<WorkspaceDiagnostic> {
    let mut names = BTreeMap::<&str, Vec<&RelativePath>>::new();
    for project in projects
        .iter()
        .filter(|project| project.state != ProjectState::Error)
    {
        names
            .entry(project.name.as_str())
            .or_default()
            .push(&project.relative_path);
    }
    names
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(name, mut paths)| {
            paths.sort();
            let listed = paths
                .iter()
                .map(|path| path.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            WorkspaceDiagnostic {
                severity: DiagnosticSeverity::Warning,
                code: WORKSPACE_MEMBER_DUPLICATE_NAME.to_owned(),
                message: format!("duplicate project name `{name}` at paths: {listed}"),
                path: None,
                project_path: None,
            }
        })
        .collect()
}

pub(super) fn sort_diagnostics(diagnostics: &mut Vec<WorkspaceDiagnostic>) {
    diagnostics.sort_by(|left, right| {
        left.project_path
            .cmp(&right.project_path)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| severity_order(left.severity).cmp(&severity_order(right.severity)))
            .then_with(|| left.message.cmp(&right.message))
    });
    diagnostics.dedup();
}

const fn severity_order(severity: DiagnosticSeverity) -> u8 {
    match severity {
        DiagnosticSeverity::Info => 0,
        DiagnosticSeverity::Warning => 1,
        DiagnosticSeverity::Error => 2,
    }
}

pub(super) fn failure(code: &str, message: String, path: Option<RelativePath>) -> DiscoveryFailure {
    DiscoveryFailure {
        code: code.to_owned(),
        message,
        path,
    }
}
