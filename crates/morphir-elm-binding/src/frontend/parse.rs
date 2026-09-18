use crate::span::Span;
use tree_sitter::{Language, Node, Parser, Tree};
use tree_sitter_language::LanguageFn;

unsafe extern "C" {
    fn tree_sitter_elm() -> *const ();
}

/// The tree-sitter [`LanguageFn`] for the vendored grammar (see grammar/UPSTREAM).
const LANGUAGE_FN: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_elm) };

/// The vendored Elm grammar (see grammar/UPSTREAM).
pub fn language() -> Language {
    LANGUAGE_FN.into()
}

pub struct ParsedTree {
    pub tree: Tree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub span: Span,
    pub message: String,
}

pub fn parse(source: &str) -> ParsedTree {
    let mut parser = Parser::new();
    parser
        .set_language(&language())
        .expect("the vendored Elm grammar is ABI compatible");
    let tree = parser
        .parse(source, None)
        .expect("parsing without a cancellation flag always yields a tree");
    ParsedTree { tree }
}

/// Every ERROR or MISSING node, outermost first, as a syntax error with its byte span.
pub fn syntax_errors(parsed: &ParsedTree, source: &str) -> Vec<SyntaxError> {
    let mut errors = Vec::new();
    collect(parsed.tree.root_node(), source, &mut errors);
    errors
}

fn collect(node: Node, source: &str, errors: &mut Vec<SyntaxError>) {
    if node.is_error() || node.is_missing() {
        let span = Span {
            start: node.start_byte(),
            end: node.end_byte(),
        };
        let what = if node.is_missing() {
            format!("syntax error: missing {}", node.kind())
        } else {
            format!(
                "syntax error near `{}`",
                source[span.start..span.end]
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
            )
        };
        errors.push(SyntaxError {
            span,
            message: what,
        });
        return; // children of an ERROR node are noise
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, source, errors);
    }
}
