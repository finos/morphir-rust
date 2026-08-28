//! Shared helpers ported from the small utilities duplicated across the Scala sources
//! (`KbScaffold.scala:20-26`, `KbIntentEdit.scala:52-54`, `KbSync.scala:328-330`).

/// Quote a string for emission into YAML frontmatter when it contains characters that
/// would change its meaning unquoted, or leading/trailing spaces.
pub fn yaml_str(s: &str) -> String {
    let needs_quote = s.chars().any(|c| ":#{}[]&*!|>'\"%@`,".contains(c))
        || s.starts_with(' ')
        || s.ends_with(' ');
    if needs_quote {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Lowercase slug: non-alphanumeric runs collapse to `-`, trimmed at both ends.
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_dash = false;
    for c in s.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c);
        } else {
            pending_dash = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_str_plain_stays_bare() {
        assert_eq!(yaml_str("plain words"), "plain words");
    }

    #[test]
    fn yaml_str_quotes_specials_and_escapes() {
        assert_eq!(yaml_str("a: b"), "\"a: b\"");
        assert_eq!(yaml_str("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(yaml_str("back\\slash, comma"), "\"back\\\\slash, comma\"");
        assert_eq!(yaml_str(" leading"), "\" leading\"");
        assert_eq!(yaml_str("trailing "), "\"trailing \"");
    }

    #[test]
    fn slugify_collapses_and_trims() {
        assert_eq!(
            slugify("  OKF Knowledge  Library! "),
            "okf-knowledge-library"
        );
        assert_eq!(slugify("Morphir IR v5"), "morphir-ir-v5");
        assert_eq!(slugify("--x--"), "x");
    }
}
