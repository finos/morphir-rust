//! Tag handling shared by every suite.

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
