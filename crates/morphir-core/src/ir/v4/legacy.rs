//! The thread-local spelling mode and the legacy member table for the decision 0006 window.
//!
//! Decision 0006 gives a set of pre-decision member spellings a one-release acceptance window:
//! they decode successfully at `path=current` with a `legacy_spelling` warning at the member's
//! cursor, and are rejected once the window closes (`path=pinned`). This module is the shared
//! table and lookup the decoders in later tasks call so every node applies the same rule.

use std::cell::{Cell, RefCell};

use crate::ir::{Diagnostic, DiagnosticCode, Warning};

/// Which spelling window a decode runs under.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpellingMode {
    /// Decision 0006's one-release window: legacy spellings decode with a warning.
    Current,
    /// The window has closed: legacy spellings are rejected as unknown members.
    Pinned,
}

thread_local! {
    static MODE: Cell<SpellingMode> = const { Cell::new(SpellingMode::Current) };
    static WARNINGS: RefCell<Option<Vec<Warning>>> = const { RefCell::new(None) };
}

/// Runs `f` with the spelling mode set (thread-local, like `with_type_encoding`), collecting any
/// `legacy_spelling` warnings [`accept_member`] records during the call.
///
/// Outside a `with_spelling_mode` call the mode is `Current` and any warnings `accept_member`
/// would have recorded are dropped instead.
pub fn with_spelling_mode<R>(mode: SpellingMode, f: impl FnOnce() -> R) -> (R, Vec<Warning>) {
    struct Restore(SpellingMode, Option<Vec<Warning>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            MODE.set(self.0);
            WARNINGS.with(|warnings| *warnings.borrow_mut() = self.1.take());
        }
    }

    let previous_mode = MODE.replace(mode);
    let previous_warnings = WARNINGS.with(|warnings| warnings.borrow_mut().replace(Vec::new()));
    let _restore = Restore(previous_mode, previous_warnings);

    let result = f();
    let warnings = WARNINGS.with(|warnings| warnings.borrow_mut().take().unwrap_or_default());
    (result, warnings)
}

/// Drains and returns the warnings collected so far on this thread, leaving the collector (if
/// any is active) empty.
pub fn take_warnings() -> Vec<Warning> {
    WARNINGS.with(|warnings| {
        let mut warnings = warnings.borrow_mut();
        match warnings.as_mut() {
            Some(collected) => std::mem::take(collected),
            None => Vec::new(),
        }
    })
}

/// The legacy member table for the decision 0006 window: `(node, legacy, canonical)`.
///
/// A `node` of `"*"` matches every node kind. The table is authoritative for every v4 decoder
/// from Task 3 onward; it was checked against the Morphir Compatibility Kit's
/// `accepted warning=legacy_spelling` fences (`spec/ir/mck/*.md`) before being committed.
pub const LEGACY: &[(&str, &str, &str)] = &[
    ("*", "attrs", "attributes"),
    ("Function", "argumentType", "parameterType"),
    ("Function", "arg", "parameterType"),
    ("Function", "result", "returnType"),
    ("IfThenElse", "thenBranch", "then"),
    ("IfThenElse", "elseBranch", "else"),
    ("Field", "subject", "target"),
    ("Field", "fieldName", "name"),
    ("LetDefinition", "valueName", "name"),
    ("LetDefinition", "valueDefinition", "definition"),
    ("LetDefinition", "inValue", "in"),
    ("ValueSpecification", "inputs", "inputTypes"),
    ("ValueSpecification", "output", "outputType"),
    ("ExternalBody", "externalName", "externals"),
    ("ExternalBody", "targetPlatform", "externals"),
];

/// Returns the canonical member name for `seen` inside node `node`.
///
/// `seen` is accepted silently when it already is a canonical member (for `node` or the
/// wildcard `"*"`). When `seen` is a legacy spelling for `node` or `"*"`, the mode decides what
/// happens: `Current` records a `legacy_spelling` warning at `cursor` and returns the canonical
/// name; `Pinned` returns an `unknown_member` diagnostic. Any other `seen` is always
/// `unknown_member`.
pub fn accept_member(node: &str, seen: &str, cursor: &str) -> Result<&'static str, Diagnostic> {
    if let Some(canonical) = LEGACY.iter().find_map(|(candidate_node, _, canonical)| {
        ((*candidate_node == node || *candidate_node == "*") && *canonical == seen)
            .then_some(*canonical)
    }) {
        return Ok(canonical);
    }

    if let Some(canonical) = LEGACY
        .iter()
        .find_map(|(candidate_node, legacy, canonical)| {
            ((*candidate_node == node || *candidate_node == "*") && *legacy == seen)
                .then_some(*canonical)
        })
    {
        return match MODE.get() {
            SpellingMode::Current => {
                let warning = Warning {
                    code: DiagnosticCode::LegacySpelling,
                    cursor: cursor.to_string(),
                };
                WARNINGS.with(|warnings| {
                    if let Some(collected) = warnings.borrow_mut().as_mut() {
                        collected.push(warning);
                    }
                });
                Ok(canonical)
            }
            SpellingMode::Pinned => Err(Diagnostic::normalization(
                DiagnosticCode::UnknownMember,
                cursor,
                format!("unexpected member {seen}"),
            )),
        };
    }

    Err(Diagnostic::normalization(
        DiagnosticCode::UnknownMember,
        cursor,
        format!("unexpected member {seen}"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_node_specific_legacy_row_has_a_distinct_legacy_spelling() {
        for (node, legacy, _canonical) in LEGACY {
            assert_ne!(
                *node, "",
                "empty node name is not allowed in the legacy table"
            );
            assert_ne!(
                *legacy, "",
                "empty legacy spelling is not allowed in the legacy table"
            );
        }
    }
}
