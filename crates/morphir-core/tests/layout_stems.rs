//! Pins `morphir_core::ir::layout::stems::stem_for` against the reference `stemFor`
//! (`IR/src/layout/stems.ts`), per
//! `.dev/docs/superpowers/maps/2026-09-17-reference-tree-layout-map.md` section 2.2.

use morphir_core::ir::DiagnosticCode;
use morphir_core::ir::layout::stem_for;
use morphir_core::naming::Name;

fn words(words: &[&str]) -> Name {
    Name::from_words(words.iter().map(|w| w.to_string()))
}

/// The worked example, pinned three ways in the reference (kit `document-tree-0004`,
/// `stems.test.ts:27-34`, `sha256.test.ts:19-21`).
#[test]
fn the_worked_example_truncates_to_the_pinned_stem() {
    let name = words(&["customer", "relationship", "management", "record"]);
    let prefix = "pkg/my-org/my-project/domain/";
    let suffix = ".type.yaml";
    let budget = 64;

    let result = stem_for(&name, prefix, suffix, budget).expect("a truncated stem is not an error");

    assert!(result.truncated);
    assert_eq!(result.stem, "customer-relati__44a101f8");

    let physical = format!("{prefix}{}{suffix}", result.stem);
    assert_eq!(physical.chars().count(), 64);
    assert_eq!(
        physical,
        "pkg/my-org/my-project/domain/customer-relati__44a101f8.type.yaml"
    );
}

#[test]
fn a_stem_that_fits_exactly_at_the_budget_is_not_truncated() {
    let name = words(&["ab"]);
    let result = stem_for(&name, "p/", ".t", 6).expect("6 == 2 + 2 + 2");
    assert!(!result.truncated);
    assert_eq!(result.stem, "ab");
}

#[test]
fn a_kept_prefix_ending_in_a_separator_is_stripped_and_not_re_extended() {
    // escaped = "aa-bbbbbbbbbbbbbbb" (18 chars, longer than the budget so truncation is forced);
    // prefix/suffix empty; budget 13 -> keep = 13-0-0-10 = 3, which cuts to "aa-", exposing a
    // trailing separator that is stripped rather than re-extended to reclaim the freed character.
    let name = words(&["aa", "bbbbbbbbbbbbbbb"]);
    let escaped = "aa-bbbbbbbbbbbbbbb";
    assert_eq!(morphir_core::naming::file_stem(&name), escaped);
    assert_eq!(escaped.chars().count(), 18);

    let result = stem_for(&name, "", "", 13).expect("keep = 3 >= 1");
    assert!(result.truncated);
    // The kept prefix "aa-" loses its trailing hyphen and is not re-extended to "aa-b" or longer.
    assert!(
        result.stem.starts_with("aa__"),
        "stem was {:?}",
        result.stem
    );
    assert_eq!(result.stem.len(), "aa".len() + "__".len() + 8);
}

#[test]
fn a_budget_too_small_to_keep_even_one_character_is_invalid_distribution_shape() {
    let long_word = "b".repeat(20);
    let name = words(&[&long_word]);
    let escaped = morphir_core::naming::file_stem(&name);
    assert_eq!(escaped, long_word);

    let diagnostic = stem_for(&name, "", "", 9).expect_err("keep = 9 - 0 - 0 - 10 = -1 < 1");

    assert_eq!(diagnostic.code, DiagnosticCode::InvalidDistributionShape);
    assert_eq!(diagnostic.cursor, "/");
    assert_eq!(
        diagnostic.message,
        format!("path budget 9 cannot fit {escaped}")
    );
}
