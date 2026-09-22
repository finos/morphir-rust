//! Lexical scanning of an Elm module header.
//!
//! Unlike Gleam, Elm derives module identity primarily from a *declared*
//! module header inside the source, not from the file's path. Reading that
//! header is lexical analysis of Elm source text, which is a different
//! concern from workspace discovery: nothing here knows what a project, a
//! snapshot or a selection is, and nothing in [`super`] knows what a block
//! comment is. Hence the separate file.
//!
//! This module ports — byte-for-byte, not merely "equivalently" — the
//! scanner the CLI used to run inline for a standalone single-file compile:
//! `elm_module_name` and `fallback_elm_module_name`
//! (`crates/morphir/src/commands/compile.rs` in the parent repository).
//! Reimplementing this with Elm's own compiler parser is not automatically
//! equivalent — the parent's scanner accepts and rejects different inputs
//! than a real parser would, especially for malformed headers and the
//! fallback cases — so this is a direct port, not a rewrite. Anyone tempted
//! to swap in a real parser has to reproduce those differences first, which
//! is what the tests at the bottom of this file are for: they drive the
//! scanner on source strings directly, because the behaviour being pinned is
//! lexical and a discovery-level test can only reach it indirectly.

/// Parses a declared Elm module name — `module`, `port module`, or `effect
/// module`, skipping one leading run of trivia including a nested block
/// comment — returning `None` when the source has no such declaration.
///
/// Ported from `elm_module_name` (parent `compile.rs:185`), with `Option`
/// replacing the parent's `Result<_, CliError>`: this provider never
/// surfaces the parse failure, since every caller falls back to a
/// filename-or-`"Main"` name instead of reporting it.
pub(super) fn elm_module_name(source: &str) -> Option<String> {
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
pub(super) fn fallback_elm_module_name(file_name: &str) -> String {
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

    /// An unterminated block comment consumes the rest of the source, so
    /// `skip_elm_trivia` runs out of input with a non-zero nesting depth and
    /// reports no declaration rather than guessing at one. The caller falls
    /// back to the filename, which is the historical behaviour: a source this
    /// malformed will fail to compile anyway, and inventing a module name
    /// from a half-scanned header would name the package after whatever text
    /// happened to follow the comment opener.
    #[test]
    fn an_unterminated_block_comment_yields_no_declaration() {
        assert_eq!(
            elm_module_name("{- module Acme.Widget exposing (..)\n"),
            None
        );
    }

    /// A nested comment left unterminated is the same case one level deeper.
    /// Note this pins the *contract* -- no declaration -- and cannot pin the
    /// depth bookkeeping behind it: see
    /// `an_unterminated_comment_is_distinguishable_from_exhausted_trivia`
    /// below for why that needs `skip_elm_trivia` tested directly.
    #[test]
    fn an_unterminated_nested_block_comment_yields_no_declaration() {
        assert_eq!(
            elm_module_name("{- outer {- inner -}\nmodule Acme.Widget exposing (..)\n"),
            None
        );
    }

    /// `skip_elm_trivia` reports an unterminated block comment as `None`
    /// rather than as trivia that happened to reach the end of the source.
    ///
    /// This needs asserting here, directly, because the distinction is
    /// invisible through `elm_module_name`: dropping the depth check makes
    /// the scanner return `Some(source.len())` instead, and the caller then
    /// looks for `module` at the end of the source, fails, and reports no
    /// declaration either way. The two unterminated-comment tests above
    /// therefore pass against both versions. Only the trivia scanner's own
    /// return value separates "the source ended mid-comment" from "the source
    /// is all trivia", which is the difference a future caller that wants to
    /// diagnose a malformed comment would depend on.
    #[test]
    fn an_unterminated_comment_is_distinguishable_from_exhausted_trivia() {
        assert_eq!(skip_elm_trivia("{- unterminated", 0), None);
        assert_eq!(skip_elm_trivia("{- outer {- inner -}", 0), None);
        // A source that really is all trivia still reports its end.
        let all_trivia = "  -- just a comment\n{- and a block -}\n";
        assert_eq!(skip_elm_trivia(all_trivia, 0), Some(all_trivia.len()));
    }

    /// A byte order mark is trivia, not the start of a declaration. Editors
    /// on Windows write one routinely, and a scanner that treated it as the
    /// first character of the source would find no `module` keyword at offset
    /// zero and fall back to the filename for every such file.
    #[test]
    fn a_byte_order_mark_before_the_declaration_is_skipped() {
        assert_eq!(
            elm_module_name("\u{feff}module Acme.Widget exposing (..)\n"),
            Some("Acme.Widget".to_owned())
        );
    }

    /// `port` is only a declaration prefix when `module` actually follows it.
    /// A bare `port` declaration -- ordinary Elm, appearing in a port
    /// module's body -- must not be mistaken for a header.
    #[test]
    fn a_port_declaration_without_module_yields_no_declaration() {
        assert_eq!(
            elm_module_name("port sendMessage : String -> Cmd msg\n"),
            None
        );
    }

    /// The ordinary case: a plain `module` header yields the dot-separated
    /// module path exactly as written, with no case folding — only the
    /// package name derived from it in [`super`] is lowercased.
    #[test]
    fn a_plain_module_header_yields_its_declared_path() {
        assert_eq!(
            elm_module_name("module Acme.Widget exposing (..)\n"),
            Some("Acme.Widget".to_owned())
        );
    }

    /// A `port module` header names the module exactly as the equivalent
    /// plain `module` header would: the `port` keyword is consumed as part of
    /// the declaration kind and contributes nothing to the name.
    #[test]
    fn a_port_module_header_yields_its_declared_path() {
        assert_eq!(
            elm_module_name("port module App.Ports exposing (sendMessage)\n"),
            Some("App.Ports".to_owned())
        );
    }

    /// An `effect module` header is accepted too, and is the one shape whose
    /// required trailing keyword is `where` rather than `exposing`. The
    /// scanner stops at that keyword and never inspects the manager record
    /// that follows it.
    #[test]
    fn an_effect_module_header_yields_its_declared_path() {
        assert_eq!(
            elm_module_name("effect module Foo.Bar where { command = MyCmd } exposing (Size)\n"),
            Some("Foo.Bar".to_owned())
        );
    }

    /// Elm block comments nest, so a naive scan for the first `-}` would stop
    /// inside the outer comment and then fail to find a declaration. This
    /// pins that `skip_elm_trivia` tracks depth and steps over the whole
    /// nested run.
    #[test]
    fn a_nested_block_comment_before_the_declaration_is_skipped() {
        assert_eq!(
            elm_module_name("{- outer {- nested -} comment -}\nmodule Acme.Widget exposing (..)\n"),
            Some("Acme.Widget".to_owned())
        );
    }

    /// A `--` line comment before the declaration is trivia as well, skipped
    /// through to the end of its line rather than treated as the start of the
    /// header.
    #[test]
    fn a_line_comment_before_the_declaration_is_skipped() {
        assert_eq!(
            elm_module_name("-- a copyright banner\nmodule Acme.Widget exposing (..)\n"),
            Some("Acme.Widget".to_owned())
        );
    }

    /// A source with no module declaration at all scans to `None` rather than
    /// guessing, which is what makes the filename fallback in
    /// [`fallback_elm_module_name`] reachable.
    #[test]
    fn a_source_without_a_declaration_scans_to_none() {
        assert_eq!(elm_module_name("x = 1\n"), None);
    }

    /// The boundary condition this hand-written scanner exists to get right:
    /// a keyword only matches when it ends at a non-identifier character, so
    /// an identifier that merely *starts* with `module` is not a `module`
    /// keyword. Without this check `modulesomething = 1` would scan as a
    /// header and the following text would be misread as a module path.
    #[test]
    fn a_keyword_does_not_match_a_longer_identifier_that_starts_with_it() {
        let source = "modulesomething = 1\n";
        assert_eq!(elm_keyword_end(source, 0, "module"), None);
        assert_eq!(elm_module_name(source), None);
        // The same text with the identifier boundary restored does match, so
        // the assertion above is about the boundary and not about some other
        // part of the header failing.
        assert_eq!(elm_keyword_end("module Something", 0, "module"), Some(6));
    }

    /// A filename whose stem is already a legal module path becomes that
    /// path: the extension is dropped and nothing else is rewritten.
    #[test]
    fn a_legal_filename_falls_back_to_its_stem() {
        assert_eq!(fallback_elm_module_name("Widget.elm"), "Widget");
    }

    /// A stem that cannot parse as a module path in full — here because the
    /// hyphen is not an Elm identifier character, so `elm_module_path` stops
    /// short of the end — falls all the way back to `"Main"` rather than to a
    /// truncated `"Not"`.
    #[test]
    fn an_illegal_filename_falls_back_to_main() {
        assert_eq!(fallback_elm_module_name("not-a-module.elm"), "Main");
    }
}
