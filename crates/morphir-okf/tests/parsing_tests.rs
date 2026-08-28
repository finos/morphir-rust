//! Port of the `KbParsingSpec` suite from morphir-scala's `KbTests.scala`,
//! plus the frontmatter and markdown edge cases the port spec calls out.

use morphir_okf::{
    extract_headings, extract_links, frontmatter, heading_slug, parse_frontmatter,
    parse_index_entry, split_frontmatter, store,
};
use std::path::Path;

// ------------------------------------------------------- split_frontmatter

#[test]
fn split_returns_none_when_there_is_no_fence() {
    let (fm, body) = split_frontmatter("# Title\n\nprose\n");
    assert!(fm.is_none());
    assert!(body.contains("# Title"));
}

#[test]
fn split_separates_a_normal_block() {
    let (fm, body) = split_frontmatter("---\ntype: Concept\n---\n\n# Title\n");
    assert_eq!(fm.as_deref(), Some("type: Concept\n"));
    assert!(body.contains("# Title"));
    assert!(!body.contains("type:"));
}

#[test]
fn split_normalises_crlf() {
    let (fm, _) = split_frontmatter("---\r\ntype: Concept\r\n---\r\n\r\nbody\r\n");
    assert_eq!(fm.as_deref(), Some("type: Concept\n"));
}

#[test]
fn split_treats_an_unterminated_fence_as_no_frontmatter() {
    let (fm, body) = split_frontmatter("---\ntype: Concept\n\nnever closed\n");
    assert!(fm.is_none());
    assert!(body.contains("never closed"));
}

#[test]
fn split_accepts_a_closing_fence_with_surrounding_whitespace() {
    let (fm, body) = split_frontmatter("---\ntype: Concept\n --- \nbody\n");
    assert_eq!(fm.as_deref(), Some("type: Concept\n"));
    assert_eq!(body, "body\n");
}

#[test]
fn split_accepts_a_closing_fence_as_the_last_unterminated_line() {
    let (fm, body) = split_frontmatter("---\ntype: Concept\n---");
    assert_eq!(fm.as_deref(), Some("type: Concept\n"));
    assert_eq!(body, "");
}

// ------------------------------------------------------- parse_frontmatter

#[test]
fn parse_rejects_duplicate_keys() {
    assert!(parse_frontmatter("type: A\ntype: B\n").is_err());
}

#[test]
fn parse_rejects_nested_duplicate_keys() {
    assert!(parse_frontmatter("generated:\n  by: a\n  by: b\n").is_err());
}

#[test]
fn parse_rejects_a_document_that_is_not_a_mapping() {
    let err = parse_frontmatter("- just\n- a list\n").unwrap_err();
    assert!(err.contains("expected a mapping"), "got: {err}");
}

#[test]
fn parse_reads_an_empty_block_as_empty_frontmatter() {
    let fm = parse_frontmatter("").unwrap();
    assert!(fm.is_empty());
}

#[test]
fn regression_parse_reads_an_unquoted_date_as_a_string_rather_than_as_absent() {
    // SnakeYAML resolves `2026-07-28` to a Date; serde_yaml keeps it a
    // string. Either way, every date-valued field — OKF's stale_after,
    // intent's created and state_since — must read back as `YYYY-MM-DD`.
    let fm = parse_frontmatter("type: Intent\ncreated: 2026-07-28\n").unwrap();
    assert_eq!(fm.str_at("created").as_deref(), Some("2026-07-28"));

    let fm = parse_frontmatter("stale_after: 2026-07-28\n").unwrap();
    assert_eq!(fm.stale_after().as_deref(), Some("2026-07-28"));
}

#[test]
fn parse_coerces_scalars_to_strings() {
    let fm = parse_frontmatter("issue: 42\nbreaking: true\nweight: 1.5\n").unwrap();
    assert_eq!(fm.str_at("issue").as_deref(), Some("42"));
    assert_eq!(fm.str_at("breaking").as_deref(), Some("true"));
    assert_eq!(fm.str_at("weight").as_deref(), Some("1.5"));
    assert_eq!(fm.int_at("issue"), Some(42));
    assert_eq!(fm.bool_at("breaking"), Some(true));
}

#[test]
fn parse_coerces_scalars_inside_lists() {
    // `supersedes: [2]` must not silently drop the integer element.
    let fm = parse_frontmatter("supersedes: [2]\n").unwrap();
    assert_eq!(fm.list_at("supersedes"), vec!["2".to_string()]);
}

#[test]
fn parse_reads_tags_and_nested_sources() {
    let fm = parse_frontmatter(
        "type: Concept\ntags: [a, b]\nsources:\n  - id: s1\n    resource: https://example.com/x.md\n",
    )
    .unwrap();
    assert_eq!(fm.tags(), vec!["a".to_string(), "b".to_string()]);
    let sources = fm.sources();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].id.as_deref(), Some("s1"));
    assert_eq!(sources[0].resource, "https://example.com/x.md");
    assert_eq!(sources[0].title, None);
}

#[test]
fn parse_drops_a_source_without_a_resource() {
    let fm = parse_frontmatter("sources:\n  - id: s1\n").unwrap();
    assert!(fm.sources().is_empty());
}

#[test]
fn keys_preserve_document_order() {
    let fm = parse_frontmatter("zeta: 1\nalpha: 2\nmid: 3\n").unwrap();
    assert_eq!(fm.keys().collect::<Vec<_>>(), vec!["zeta", "alpha", "mid"]);
}

// --------------------------------------------------------------- markdown

#[test]
fn links_report_line_numbers_offset_past_the_frontmatter() {
    let links = extract_links("intro\n\n[one](/a.md) and [two](https://x)\n", 5);
    assert_eq!(
        links.iter().map(|l| l.dest.as_str()).collect::<Vec<_>>(),
        vec!["/a.md", "https://x"]
    );
    assert_eq!(links[0].line, 8);
    assert!(links[0].is_bundle_relative());
    assert!(links[1].is_external());
}

#[test]
fn links_inside_fenced_code_are_ignored() {
    let links = extract_links("```\n[not a link](/nope.md)\n```\n\n[real](/yes.md)\n", 0);
    assert_eq!(
        links.iter().map(|l| l.dest.as_str()).collect::<Vec<_>>(),
        vec!["/yes.md"]
    );
}

#[test]
fn links_carry_their_text() {
    let links = extract_links("See [the **deep** dive](/deep.md).\n", 0);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].text, "the deep dive");
}

#[test]
fn reference_links_resolve_to_their_definition() {
    let links = extract_links("[ref][r]\n\n[r]: /target.md\n", 0);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].dest, "/target.md");
    assert_eq!(links[0].line, 1);
}

#[test]
fn autolinks_are_links_and_images_are_not() {
    let links = extract_links("<https://example.com>\n\n![alt](/img.png)\n", 0);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].dest, "https://example.com");
    assert!(links[0].is_external());
}

#[test]
fn headings_skip_fenced_code() {
    let hs = extract_headings("# Real\n\n```bash\n# just a shell comment\n```\n\n## Also real\n");
    assert_eq!(
        hs.iter().map(|h| h.text.as_str()).collect::<Vec<_>>(),
        vec!["Real", "Also real"]
    );
}

#[test]
fn headings_carry_level_slug_and_line() {
    let hs = extract_headings("# One\n\n## Two Words!\n\n~~~\n# fenced\n~~~\n");
    assert_eq!(hs.len(), 2);
    assert_eq!(
        (hs[0].level, hs[0].line, hs[0].slug.as_str()),
        (1, 1, "one")
    );
    assert_eq!(
        (hs[1].level, hs[1].line, hs[1].slug.as_str()),
        (2, 3, "two-words")
    );
}

#[test]
fn heading_slug_matches_the_reference_rules() {
    assert_eq!(heading_slug("  Release Labels, v2! "), "release-labels-v2");
    assert_eq!(heading_slug("Already-Kebab"), "already-kebab");
}

// -------------------------------------------------------- index-entry regex

#[test]
fn index_entry_captures_all_groups() {
    let entry = parse_index_entry("  * [Naming](/naming.md) - How things are named.").unwrap();
    assert_eq!(entry.link, "  * [Naming](/naming.md)");
    assert_eq!(entry.title, "Naming");
    assert_eq!(entry.dest, "/naming.md");
    assert_eq!(entry.description.as_deref(), Some("How things are named."));
}

#[test]
fn index_entry_description_is_optional() {
    let entry = parse_index_entry("- [T](/p.md)").unwrap();
    assert_eq!(entry.title, "T");
    assert_eq!(entry.dest, "/p.md");
    assert_eq!(entry.description, None);
}

#[test]
fn index_entry_rejects_a_plain_line() {
    assert!(parse_index_entry("just prose with a [link](/x.md) in it").is_none());
    assert!(parse_index_entry("* not a link bullet").is_none());
}

// ------------------------------------------------- doc parsing and offsets

#[test]
fn parse_doc_tracks_frontmatter_lines_and_shifts_link_lines() {
    // Raw frontmatter "type: Concept\ntitle: T\n" has 2 newlines; +2 fences.
    let text = "---\ntype: Concept\ntitle: T\n---\n\n[a](/a.md)\n";
    let doc = store::parse_doc(
        Path::new("/kb/bundles/demo/x.md"),
        Path::new("/kb/bundles/demo"),
        text,
    );
    assert_eq!(doc.frontmatter_lines, 4);
    assert!(doc.has_frontmatter_block);
    assert_eq!(doc.frontmatter_error, None);
    // The link sits on body line 2, so file line 6.
    assert_eq!(doc.links.len(), 1);
    assert_eq!(doc.links[0].line, 6);
    assert_eq!(doc.fm().doc_type().as_deref(), Some("Concept"));
}

#[test]
fn parse_doc_records_a_frontmatter_error_without_dying() {
    let text = "---\ntype: [unclosed\n---\n\nbody\n";
    let doc = store::parse_doc(
        Path::new("/kb/bundles/demo/x.md"),
        Path::new("/kb/bundles/demo"),
        text,
    );
    assert!(doc.has_frontmatter_block);
    assert!(doc.frontmatter.is_none());
    assert!(doc.frontmatter_error.is_some());
    assert!(doc.body.contains("body"));
}

#[test]
fn parse_doc_without_frontmatter_has_no_block_and_no_offset() {
    let doc = store::parse_doc(
        Path::new("/kb/bundles/demo/sub/index.md"),
        Path::new("/kb/bundles/demo"),
        "# Sub\n\n[a](../a.md)\n",
    );
    assert!(!doc.has_frontmatter_block);
    assert_eq!(doc.frontmatter_lines, 0);
    assert_eq!(doc.links[0].line, 3);
}

#[test]
fn frontmatter_line_count_is_newlines_plus_two() {
    assert_eq!(frontmatter::frontmatter_line_count("type: Concept\n"), 3);
    assert_eq!(frontmatter::frontmatter_line_count(""), 2);
}
