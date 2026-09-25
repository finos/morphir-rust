use morphir_gherkin::extension::*;
use morphir_gherkin::{Fence, NodePath, ProseBlock, Tag, read_str};

#[derive(Debug, PartialEq)]
struct Syntax(String);

struct SyntaxTags;
impl TagExtension for SyntaxTags {
    fn namespace(&self) -> Option<&str> {
        Some("syntax")
    }
    fn apply(&self, tag: &Tag, _scope: Scope, ctx: &mut Context) -> Result<Effect, String> {
        match tag.namespaced() {
            Some((_, value @ ("elm" | "gleam"))) => {
                ctx.insert(Syntax(value.to_owned()));
                Ok(Effect::Continue)
            }
            Some((_, other)) => Err(format!("unknown syntax `{other}`")),
            None => Ok(Effect::Continue),
        }
    }
}

struct Wip;
impl TagExtension for Wip {
    fn namespace(&self) -> Option<&str> {
        None
    }
    fn matches(&self, tag: &Tag) -> bool {
        tag.name == "wip"
    }
    fn apply(&self, _tag: &Tag, _scope: Scope, _ctx: &mut Context) -> Result<Effect, String> {
        Ok(Effect::Skip("work in progress".to_owned()))
    }
}

#[derive(Debug, PartialEq)]
struct Options(String);
struct OptionsFence;
impl FenceExtension for OptionsFence {
    fn matches(&self, fence: &Fence) -> bool {
        fence.info.language == "yaml" && fence.info.words == ["morphir"]
    }
    fn apply(&self, fence: &Fence, _scope: Scope, ctx: &mut Context) -> Result<(), String> {
        ctx.insert(Options(fence.body.clone()));
        Ok(())
    }
}

#[derive(Debug, PartialEq, Default)]
struct Musts(Vec<String>);
struct MustSentences;
impl ProseExtension for MustSentences {
    fn apply(&self, prose: &ProseBlock, _scope: Scope, ctx: &mut Context) -> Result<(), String> {
        if prose.markdown.contains("MUST") {
            if ctx.get::<Musts>().is_none() {
                ctx.insert(Musts::default());
            }
            ctx.get_mut::<Musts>()
                .unwrap()
                .0
                .push(prose.markdown.trim().to_owned());
        }
        Ok(())
    }
}

const TEXT: &str = "@syntax:elm\nFeature: F\n  It MUST keep signatures.\n\n  ```yaml morphir\n  a: 1\n  ```\n\n  Scenario: inherits\n    Given a step\n\n  @syntax:gleam\n  Scenario: overrides\n    Given a step\n\n  @syntax:elmm\n  Scenario: typo\n    Given a step\n\n  @wip\n  Scenario: skipped\n    Given a step\n";

fn extensions() -> Extensions {
    Extensions::new()
        .with_tags(SyntaxTags)
        .with_tags(Wip)
        .with_fences(OptionsFence)
        .with_prose(MustSentences)
}

fn scenario(i: usize) -> NodePath {
    NodePath::feature().push(morphir_gherkin::Segment::Scenario(i))
}

#[test]
fn feature_level_extensions_reach_every_scenario() {
    let (doc, _) = read_str("f.feature", TEXT).unwrap();
    let (ctx, effect) = extensions().context_for(&doc, &scenario(0)).unwrap();
    assert_eq!(effect, Effect::Continue);
    assert_eq!(ctx.get::<Syntax>(), Some(&Syntax("elm".to_owned())));
    assert_eq!(ctx.get::<Options>(), Some(&Options("a: 1\n".to_owned())));
    assert_eq!(
        ctx.get::<Musts>().unwrap().0,
        vec!["It MUST keep signatures.".to_owned()]
    );
}

#[test]
fn a_scenario_tag_overrides_the_feature_tag() {
    let (doc, _) = read_str("f.feature", TEXT).unwrap();
    let (ctx, _) = extensions().context_for(&doc, &scenario(1)).unwrap();
    assert_eq!(ctx.get::<Syntax>(), Some(&Syntax("gleam".to_owned())));
}

#[test]
fn an_unknown_value_in_an_owned_namespace_fails_with_the_tag_span() {
    let (doc, source) = read_str("f.feature", TEXT).unwrap();
    let errors = extensions().context_for(&doc, &scenario(2)).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(source.slice(errors[0].span), "@syntax:elmm");
    assert_eq!(errors[0].path.to_string(), "feature/scenario[2]");
    assert!(errors[0].to_string().contains("unknown syntax `elmm`"));
}

#[test]
fn a_skip_effect_is_reported() {
    let (doc, _) = read_str("f.feature", TEXT).unwrap();
    let (_, effect) = extensions().context_for(&doc, &scenario(3)).unwrap();
    assert_eq!(effect, Effect::Skip("work in progress".to_owned()));
}

#[test]
fn an_unclaimed_tag_is_a_label() {
    let (doc, _) = read_str(
        "f.feature",
        "@just-a-label\nFeature: F\n  Scenario: s\n    Given a step\n",
    )
    .unwrap();
    assert!(extensions().context_for(&doc, &scenario(0)).is_ok());
}
