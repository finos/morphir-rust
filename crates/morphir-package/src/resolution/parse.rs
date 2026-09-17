use super::execution::Budget;
use super::model::{
    ResolutionDiagnostic, ResolutionExecutionError, ResolutionResult, Violation, ViolationRule,
};
use super::replay::validate_lock;
use super::validate::{Mode, outer_identities, outer_shape};
use super::wire::{DocumentError, parse_document};
use super::{diagnostics, search, update};

pub(super) fn resolve(input: &str) -> Result<ResolutionResult, ResolutionExecutionError> {
    let document = match parse_document(input) {
        Ok(document) => document,
        Err(DocumentError::Malformed) => {
            return Ok(ResolutionResult::Rejected(
                ResolutionDiagnostic::InvalidInput {
                    violations: vec![Violation {
                        pointer: String::new(),
                        rule: ViolationRule::MalformedJson,
                    }],
                },
            ));
        }
        Err(DocumentError::ResourceExhausted) => {
            return Err(ResolutionExecutionError::new(
                "JSON nesting exceeds the execution budget",
            ));
        }
    };
    if !document.duplicates.is_empty() {
        return Ok(ResolutionResult::Rejected(
            ResolutionDiagnostic::InvalidInput {
                violations: document.duplicates,
            },
        ));
    }
    let mut input = match outer_shape(&document.value) {
        Ok(input) => input,
        Err(violations) => {
            return Ok(ResolutionResult::Rejected(
                ResolutionDiagnostic::InvalidInput { violations },
            ));
        }
    };
    let violations = outer_identities(&mut input);
    if !violations.is_empty() {
        return Ok(ResolutionResult::Rejected(
            ResolutionDiagnostic::InvalidInput { violations },
        ));
    }
    let mut budget = Budget::default();
    if input.mode == Mode::Replay {
        return Ok(match validate_lock(&input) {
            Ok(graph) => ResolutionResult::Resolved(graph),
            Err(diagnostic) => ResolutionResult::Rejected(*diagnostic),
        });
    }
    if input.mode == Mode::Update {
        let old = match validate_lock(&input) {
            Ok(graph) => graph,
            Err(diagnostic) => return Ok(ResolutionResult::Rejected(*diagnostic)),
        };
        let violations = update::target_violations(&input, &old);
        if !violations.is_empty() {
            return Ok(ResolutionResult::Rejected(
                ResolutionDiagnostic::InvalidInput { violations },
            ));
        }
        let reachable_catalogs = match search::complete_catalog_paths(&input) {
            Ok(paths) => paths,
            Err(missing) => {
                return Ok(ResolutionResult::Rejected(
                    ResolutionDiagnostic::IncompleteInput { missing },
                ));
            }
        };
        let policy = update::policy(&input, &old, true);
        let graphs = search::supported(&input, &policy, &mut budget)?;
        if let Some(graph) = update::choose(graphs, &input, &old) {
            return Ok(ResolutionResult::Resolved(graph));
        }
        let relaxed_policy = update::policy(&input, &old, false);
        let relaxed_graphs = search::supported(&input, &relaxed_policy, &mut budget)?;
        if !relaxed_graphs.is_empty() {
            return Ok(ResolutionResult::Rejected(diagnostics::scope_conflict(
                &input,
                &old,
                &relaxed_graphs,
                &mut budget,
            )?));
        }
        if let Some(diagnostic) =
            diagnostics::unsupported(&input, &relaxed_policy, Some(&old), &mut budget)?
        {
            return Ok(ResolutionResult::Rejected(diagnostic));
        }
        return Ok(ResolutionResult::Rejected(diagnostics::unsatisfiable(
            &input,
            &reachable_catalogs,
        )));
    }
    let reachable_catalogs = match search::complete_catalog_paths(&input) {
        Ok(paths) => paths,
        Err(missing) => {
            return Ok(ResolutionResult::Rejected(
                ResolutionDiagnostic::IncompleteInput { missing },
            ));
        }
    };
    let mut graphs = search::supported(&input, &search::SearchPolicy::default(), &mut budget)?;
    if !graphs.is_empty() {
        return Ok(ResolutionResult::Resolved(graphs.remove(0)));
    }
    if let Some(diagnostic) =
        diagnostics::unsupported(&input, &search::SearchPolicy::default(), None, &mut budget)?
    {
        return Ok(ResolutionResult::Rejected(diagnostic));
    }
    Ok(ResolutionResult::Rejected(diagnostics::unsatisfiable(
        &input,
        &reachable_catalogs,
    )))
}
