use morphir_gherkin::markdown::{common_indent, parse_blocks};
use morphir_gherkin::{DescriptionBlock, Inline, ProseKind, SourceText, Span};

const TEXT: &str = "  The feature text with `code` and a [link](kb/x.md).\n\n  ```yaml morphir\n  outer:\n    inner: 1\n\n    after_blank: 2\n  ```\n\n  More **prose**.\n";

#[test]
fn a_description_keeps_its_fence_byte_for_byte_and_its_prose_parsed() {
    let source = SourceText::new(TEXT);
    let range = Span {
        start: 0,
        end: TEXT.len(),
    };
    assert_eq!(common_indent(&source, range), 2);
    let description = parse_blocks(&source, range, 2);

    let fences: Vec<_> = description.fences().collect();
    assert_eq!(fences.len(), 1);
    assert_eq!(fences[0].info.language, "yaml");
    assert_eq!(fences[0].info.words, vec!["morphir".to_owned()]);
    assert_eq!(fences[0].body, "outer:\n  inner: 1\n\n  after_blank: 2\n");
    assert_eq!(fences[0].position.line, 3);
    assert_eq!(
        source.slice(fences[0].span).lines().next(),
        Some("  ```yaml morphir")
    );

    let prose: Vec<_> = description.prose().collect();
    assert_eq!(prose.len(), 2);
    assert_eq!(prose[0].kind, ProseKind::Paragraph);
    assert!(prose[0].inlines.contains(&Inline::Code("code".to_owned())));
    assert!(prose[0].inlines.iter().any(
        |inline| matches!(inline, Inline::Link { destination, .. } if destination == "kb/x.md")
    ));
    assert_eq!(prose[1].position.line, 10);
    assert!(matches!(description.blocks[1], DescriptionBlock::Fence(_)));
}

#[test]
fn an_empty_range_gives_an_empty_description() {
    let source = SourceText::new("");
    assert!(
        parse_blocks(&source, Span { start: 0, end: 0 }, 0)
            .blocks
            .is_empty()
    );
}
