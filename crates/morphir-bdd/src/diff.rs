//! Git-style unified diffs for every text mismatch.

use similar::TextDiff;

/// The most lines a diff will print before it is cut short.
const MAX_LINES: usize = 400;

/// Builds a unified diff between `expected` and `actual`, with `expected_name` and `actual_name`
/// as the `---`/`+++` header labels and three lines of context around each hunk.
///
/// The diff is capped at [`MAX_LINES`] lines. A longer diff is cut at that line and ends with a
/// closing line, `… N more lines not shown`.
///
/// ```
/// use morphir_bdd::diff::unified_diff;
///
/// let diff = unified_diff("a\nb\nc\n", "a\nB\nc\n", "expected", "actual");
/// assert!(diff.starts_with("--- expected\n+++ actual\n@@ -1,3 +1,3 @@\n"));
/// assert!(diff.contains("-b\n+B\n"));
/// ```
#[must_use]
pub fn unified_diff(
    expected: &str,
    actual: &str,
    expected_name: &str,
    actual_name: &str,
) -> String {
    let full = TextDiff::from_lines(expected, actual)
        .unified_diff()
        .context_radius(3)
        .header(expected_name, actual_name)
        .to_string();
    let lines: Vec<&str> = full.lines().collect();
    if lines.len() <= MAX_LINES {
        return full;
    }
    let mut capped = lines[..MAX_LINES].join("\n");
    capped.push_str(&format!(
        "\n… {} more lines not shown\n",
        lines.len() - MAX_LINES
    ));
    capped
}

#[cfg(test)]
mod tests {
    use super::unified_diff;

    #[test]
    fn a_diff_has_headers_hunks_and_context() {
        let diff = unified_diff("a\nb\nc\nd\n", "a\nB\nc\nd\n", "expected", "actual");
        assert!(
            diff.starts_with("--- expected\n+++ actual\n@@ -1,4 +1,4 @@\n"),
            "{diff}"
        );
        assert!(diff.contains("-b\n+B\n"), "{diff}");
    }

    #[test]
    fn a_long_diff_is_capped() {
        let expected: String = (0..1000).map(|i| format!("{i}\n")).collect();
        let diff = unified_diff(&expected, "", "e", "a");
        assert!(diff.lines().count() <= 402, "{}", diff.lines().count());
        assert!(diff.trim_end().ends_with("more lines not shown"), "{diff}");
    }
}
