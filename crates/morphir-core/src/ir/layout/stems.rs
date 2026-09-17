//! `stemFor`: the file stem a name is given inside a module directory, truncated when it would
//! not fit the tree's path budget.
//!
//! Mirrors `IR/src/layout/stems.ts:stemFor` in `ecosystem/morphir-typescript`; see
//! `.dev/docs/superpowers/maps/2026-09-17-reference-tree-layout-map.md` section 2.2 for the
//! worked example this is pinned against.

use crate::ir::{Diagnostic, DiagnosticCode, DiagnosticStage};
use crate::naming::{self, Name};

/// `"__"` plus eight hex digits: what a truncated stem's hash suffix costs.
const HASH_SUFFIX_LEN: u32 = 10;

/// The stem `stem_for` chose for a name, and whether it had to be truncated to fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StemResult {
    pub stem: String,
    pub truncated: bool,
}

/// The file stem `name` is given under a module directory whose physical path already carries
/// `physical_prefix` (the root, the module's directory, and the trailing `/`) and will carry
/// `suffix` (the node kind and the profile's extension) after it.
///
/// The whole physical path — root included, version slot included, extension included — is what
/// is measured against `path_budget`, in characters, inclusive: a path exactly at the budget
/// fits. When it does not, the stem is shortened to `path_budget - physical_prefix.len() -
/// suffix.len() - 10` characters of the escaped stem (stripping a trailing `-` or `_` the cut
/// exposed, without re-extending afterwards — the reference computes a fixed cut rather than the
/// longest prefix that fits) followed by `__` and the first eight hex digits of the SHA-256 of
/// the *full, untruncated* escaped stem. A budget too small to hold even one kept character is
/// `invalid_distribution_shape`, cursor `/`, rather than an overrun path.
pub fn stem_for(
    name: &Name,
    physical_prefix: &str,
    suffix: &str,
    path_budget: u32,
) -> Result<StemResult, Diagnostic> {
    let escaped = naming::file_stem(name);

    let prefix_len = char_len(physical_prefix);
    let escaped_len = char_len(&escaped);
    let suffix_len = char_len(suffix);

    if prefix_len + escaped_len + suffix_len <= path_budget {
        return Ok(StemResult {
            stem: escaped,
            truncated: false,
        });
    }

    let budget_error = || {
        Diagnostic::new(
            DiagnosticCode::InvalidDistributionShape,
            DiagnosticStage::Semantic,
            "/",
            format!("path budget {path_budget} cannot fit {physical_prefix}{escaped}{suffix}"),
        )
    };

    let keep = i64::from(path_budget)
        - i64::from(prefix_len)
        - i64::from(suffix_len)
        - i64::from(HASH_SUFFIX_LEN);
    if keep < 1 {
        return Err(budget_error());
    }

    // `truncate_stem`'s `available` is `keep + 10`; its own floor, `MIN_TRUNCATED_STEM_BUDGET`
    // (11), is exactly what `keep >= 1` guarantees here, so this is not expected to refuse — but
    // a caller finding it refuse anyway is a real disagreement between the two budgets, not
    // something to paper over with an `unwrap`.
    let available = keep as u64 + u64::from(HASH_SUFFIX_LEN);
    let available = usize::try_from(available).map_err(|_| budget_error())?;

    match naming::truncate_stem(&escaped, available) {
        Some(stem) => Ok(StemResult {
            stem,
            truncated: true,
        }),
        None => Err(budget_error()),
    }
}

/// A budget is a count of characters, not bytes: the escaped-stem grammar is ASCII, but a
/// caller's prefix or suffix is not guaranteed to be.
fn char_len(text: &str) -> u32 {
    u32::try_from(text.chars().count()).unwrap_or(u32::MAX)
}
