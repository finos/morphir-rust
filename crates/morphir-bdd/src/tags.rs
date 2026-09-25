//! Tag handling shared by every suite.

use cucumber::gherkin::tagexpr::TagOperation;
use morphir_gherkin::Tag;
use morphir_gherkin::extension::{Context, Effect, Scope, TagExtension};

/// `@wip` skips a scenario before it starts.
pub struct WipTag;

impl TagExtension for WipTag {
    fn namespace(&self) -> Option<&str> {
        None
    }
    fn matches(&self, tag: &Tag) -> bool {
        tag.name == "wip"
    }
    fn apply(&self, _tag: &Tag, _scope: Scope, _ctx: &mut Context) -> Result<Effect, String> {
        Ok(Effect::Skip("work in progress (@wip)".to_owned()))
    }
}

/// A Cucumber tag expression (`@a and not @b`). `gherkin` 0.16 parses these but has no evaluator.
#[derive(Debug, Clone)]
pub struct TagExpr(TagOperation);

impl TagExpr {
    /// Parses a tag expression such as `@p0 and not @wip`.
    ///
    /// # Errors
    ///
    /// Returns an error if `text` is not a valid tag expression.
    pub fn parse(text: &str) -> Result<Self, String> {
        text.parse::<TagOperation>()
            .map(Self)
            .map_err(|e| format!("not a tag expression `{text}`: {e}"))
    }

    /// Reports whether this expression is satisfied by the given tag names (without a leading `@`).
    #[must_use]
    pub fn matches(&self, tags: &[String]) -> bool {
        fn eval(op: &TagOperation, tags: &[String]) -> bool {
            match op {
                TagOperation::And(a, b) => eval(a, tags) && eval(b, tags),
                TagOperation::Or(a, b) => eval(a, tags) || eval(b, tags),
                TagOperation::Not(a) => !eval(a, tags),
                TagOperation::Tag(name) => tags.iter().any(|t| t == name.trim_start_matches('@')),
            }
        }
        eval(&self.0, tags)
    }
}

#[cfg(test)]
mod tests {
    use super::TagExpr;

    fn tags(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn expressions_evaluate_against_a_scenarios_tags() {
        let expr = TagExpr::parse("@p0 and not @wip").unwrap();
        assert!(expr.matches(&tags(&["p0"])));
        assert!(!expr.matches(&tags(&["p0", "wip"])));
        assert!(TagExpr::parse("@a or @b").unwrap().matches(&tags(&["b"])));
        assert!(
            TagExpr::parse("@node:Value")
                .unwrap()
                .matches(&tags(&["node:Value"]))
        );
        assert!(TagExpr::parse("@a and").is_err());
    }
}
