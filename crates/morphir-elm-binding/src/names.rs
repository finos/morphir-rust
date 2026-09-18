//! The name decomposition the whole binding shares, and the spellings that
//! survive a round trip through a Morphir name.
//!
//! An identifier becomes a list of words exactly once, here, and each emitter
//! builds its own version's name from those words: classic keeps the word list,
//! v4 rebuilds it with `Name::from_words`, which collapses a run of single
//! letters into an initialism. Sharing this step is what makes a natively
//! emitted v4 document identical to the one migrating the classic sibling
//! produces — the migration splits names the same way
//! (`morphir_core::migration::migrate_name`).
//!
//! A Morphir name keeps only the words, so the spelling an identifier had in
//! source cannot be recovered from a document: `localDate`, `LocalDate` and
//! `local_date` all read back as `["local", "date"]`. Anything compared across
//! the source/document boundary — above all the public interface a module's
//! digest is taken over — therefore uses [`type_spelling`] or [`value_spelling`],
//! which name the one spelling a document *can* express. Two identifiers that a
//! Morphir document cannot tell apart are, deliberately, the same identifier
//! here too.

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

/// The upper-initial spelling a type, constructor or path segment has once it
/// has been through a Morphir name: `LocalDate`, `SDK`.
pub fn type_spelling(source: &str) -> String {
    type_spelling_of(&words(source))
}

/// [`type_spelling`], for words already split out of a document.
pub fn type_spelling_of(words: &[String]) -> String {
    words.iter().map(|word| capitalize(word)).collect()
}

/// The lower-initial spelling a type variable, type parameter or record field
/// has once it has been through a Morphir name: `localDate`, `a`.
pub fn value_spelling(source: &str) -> String {
    value_spelling_of(&words(source))
}

/// [`value_spelling`], for words already split out of a document.
pub fn value_spelling_of(words: &[String]) -> String {
    words
        .iter()
        .enumerate()
        .map(|(index, word)| {
            if index == 0 {
                word.clone()
            } else {
                capitalize(word)
            }
        })
        .collect()
}

fn capitalize(word: &str) -> String {
    let mut characters = word.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pascal_case_type_name_survives_the_round_trip() {
        for name in ["LocalDate", "SDK", "Status", "Vec2"] {
            assert_eq!(type_spelling(name), name);
        }
    }

    #[test]
    fn a_camel_case_value_name_survives_the_round_trip() {
        for name in ["localDate", "a", "id", "r", "arg1"] {
            assert_eq!(value_spelling(name), name);
        }
    }

    #[test]
    fn spellings_a_document_cannot_tell_apart_agree() {
        assert_eq!(type_spelling("local"), type_spelling("Local"));
        assert_eq!(value_spelling("local_date"), value_spelling("LocalDate"));
    }
}
