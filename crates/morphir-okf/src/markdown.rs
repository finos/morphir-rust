//! Markdown extraction: links via a real CommonMark parser, headings via a
//! line scan that skips fenced code blocks.
//!
//! Link extraction is ported from `KbStore.extractLinks` (commonmark-java
//! visitor) and matches its semantics: inline, reference, and autolink links
//! are collected; images are not; link text is the concatenation of the text
//! literals inside the link. Heading extraction is ported from
//! `KbIndex.headings`.

use std::sync::LazyLock;

use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use regex::Regex;
use serde::Serialize;

use crate::model::LinkRef;

/// Extracts every link from a markdown body, with 1-based line numbers.
///
/// `frontmatter_lines` shifts line numbers so they refer to the original file
/// rather than the stripped body. Links inside fenced code blocks are ignored
/// because the parser never sees them as links.
pub fn extract_links(body: &str, frontmatter_lines: usize) -> Vec<LinkRef> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    // Links cannot nest in CommonMark, so one open link at a time suffices.
    let mut current: Option<(String, String, usize)> = None;
    for (event, range) in Parser::new(body).into_offset_iter() {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                let line = 1 + bytes[..range.start].iter().filter(|&&b| b == b'\n').count();
                current = Some((String::new(), dest_url.to_string(), line));
            }
            Event::End(TagEnd::Link) => {
                if let Some((text, dest, line)) = current.take() {
                    out.push(LinkRef {
                        text,
                        dest,
                        line: line + frontmatter_lines,
                    });
                }
            }
            Event::Text(t) => {
                if let Some((buf, _, _)) = current.as_mut() {
                    buf.push_str(&t);
                }
            }
            _ => {}
        }
    }
    out
}

/// A heading found in a markdown body. `line` is 1-based and body-relative,
/// matching the reference implementation (the store shifts link lines past
/// the frontmatter; heading lines it does not).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Heading {
    pub level: usize,
    pub text: String,
    pub slug: String,
    pub line: usize,
}

static HEADING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,6})\s+(.*?)\s*#*$").expect("valid regex"));

/// Headings in a body, with 1-based body-relative line numbers. Fenced code
/// blocks (toggled by ``` and ~~~) are skipped so that a `# comment` inside a
/// shell example is not mistaken for a section.
pub fn extract_headings(body: &str) -> Vec<Heading> {
    let mut in_fence = false;
    let mut out = Vec::new();
    for (i, line) in body.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(caps) = HEADING.captures(line) {
            let text = caps.get(2).map(|m| m.as_str().trim()).unwrap_or("");
            out.push(Heading {
                level: caps.get(1).map(|m| m.as_str().len()).unwrap_or(0),
                text: text.to_string(),
                slug: heading_slug(text),
                line: i + 1,
            });
        }
    }
    out
}

/// Slugifies a heading: trim, lowercase, runs of anything outside `[a-z0-9]`
/// become a single `-`, leading/trailing `-` stripped.
pub fn heading_slug(s: &str) -> String {
    let lowered = s.trim().to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    let mut gap = false;
    for c in lowered.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}
