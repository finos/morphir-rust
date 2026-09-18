//! The name decomposition both emitters share.
//!
//! An identifier becomes a list of words exactly once, here, and each emitter
//! builds its own version's name from those words: classic keeps the word list,
//! v4 rebuilds it with `Name::from_words`, which collapses a run of single letters
//! into an initialism. Sharing this step is what makes a natively emitted v4
//! document identical to the one migrating the classic sibling produces — the
//! migration splits names the same way (`morphir_core::migration::migrate_name`).

use morphir_core::ir::classic::Name as ClassicName;
use morphir_core::naming::resolve;

/// The words `source` splits into, the way morphir-elm's `Name.fromString` does:
/// `LocalDate` is `["local", "date"]` and `SDK` is `["s", "d", "k"]`.
pub fn words(source: &str) -> Vec<String> {
    ClassicName::from_str(source)
        .words
        .iter()
        .map(|word| resolve(*word).to_owned())
        .collect()
}

/// The words naming the `index`-th positional constructor argument.
///
/// `Morphir.Elm.Frontend` builds the word list `[ "arg", String.fromInt (index + 1) ]`
/// directly, so the names are one-based and already split.
pub fn argument_words(index: usize) -> Vec<String> {
    vec!["arg".to_string(), (index + 1).to_string()]
}

/// A module path as a reader sees it in Elm source, for diagnostics: `My.Types`.
pub fn module_label(segments: &[String]) -> String {
    segments.join(".")
}
