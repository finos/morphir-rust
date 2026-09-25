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

#[derive(Debug, PartialEq)]
struct First;
struct FirstWip;
impl TagExtension for FirstWip {
    fn namespace(&self) -> Option<&str> {
        None
    }
    fn matches(&self, tag: &Tag) -> bool {
        tag.name == "wip"
    }
    fn apply(&self, _tag: &Tag, _scope: Scope, ctx: &mut Context) -> Result<Effect, String> {
        ctx.insert(First);
        Ok(Effect::Continue)
    }
}

#[derive(Debug, PartialEq)]
struct Second;
struct SecondWip;
impl TagExtension for SecondWip {
    fn namespace(&self) -> Option<&str> {
        None
    }
    fn matches(&self, tag: &Tag) -> bool {
        tag.name == "wip"
    }
    fn apply(&self, _tag: &Tag, _scope: Scope, ctx: &mut Context) -> Result<Effect, String> {
        ctx.insert(Second);
        Ok(Effect::Continue)
    }
}

#[test]
fn when_two_tag_extensions_both_match_the_first_registered_handles_it() {
    let (doc, _) = read_str("f.feature", TEXT).unwrap();
    let (ctx, _) = Extensions::new()
        .with_tags(FirstWip)
        .with_tags(SecondWip)
        .context_for(&doc, &scenario(3))
        .unwrap();
    assert_eq!(ctx.get::<First>(), Some(&First));
    assert_eq!(ctx.get::<Second>(), None);
}

#[derive(Debug, PartialEq)]
struct FirstFence;
struct FirstOptionsFence;
impl FenceExtension for FirstOptionsFence {
    fn matches(&self, fence: &Fence) -> bool {
        fence.info.language == "yaml" && fence.info.words == ["morphir"]
    }
    fn apply(&self, _fence: &Fence, _scope: Scope, ctx: &mut Context) -> Result<(), String> {
        ctx.insert(FirstFence);
        Ok(())
    }
}

#[derive(Debug, PartialEq)]
struct SecondFence;
struct SecondOptionsFence;
impl FenceExtension for SecondOptionsFence {
    fn matches(&self, fence: &Fence) -> bool {
        fence.info.language == "yaml" && fence.info.words == ["morphir"]
    }
    fn apply(&self, _fence: &Fence, _scope: Scope, ctx: &mut Context) -> Result<(), String> {
        ctx.insert(SecondFence);
        Ok(())
    }
}

#[test]
fn when_two_fence_extensions_both_match_the_first_registered_handles_it() {
    let (doc, _) = read_str("f.feature", TEXT).unwrap();
    let (ctx, _) = Extensions::new()
        .with_fences(FirstOptionsFence)
        .with_fences(SecondOptionsFence)
        .context_for(&doc, &scenario(0))
        .unwrap();
    assert_eq!(ctx.get::<FirstFence>(), Some(&FirstFence));
    assert_eq!(ctx.get::<SecondFence>(), None);
}

const RULE_TEXT: &str = "Feature: F\n  Rule: R\n    Scenario: s\n      Given a step\n";

#[test]
fn a_rule_path_is_not_a_scenario() {
    let (doc, _) = read_str("f.feature", RULE_TEXT).unwrap();
    let rule = NodePath::feature().push(morphir_gherkin::Segment::Rule(0));
    let errors = extensions().context_for(&doc, &rule).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].path, rule);
    assert!(errors[0].to_string().contains("not a scenario"));
}

#[test]
fn a_step_path_is_not_a_scenario() {
    let (doc, _) = read_str("f.feature", TEXT).unwrap();
    let step = scenario(0).push(morphir_gherkin::Segment::Step(0));
    let errors = extensions().context_for(&doc, &step).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].path, step);
    assert!(errors[0].to_string().contains("not a scenario"));
}

const OUTLINE_TEXT: &str = "@syntax:elm\nFeature: F\n  Scenario Outline: o\n    Given <x>\n\n    @wip\n    Examples: first\n      | x |\n      | 1 |\n\n    @syntax:gleam\n    Examples: second\n      | x |\n      | 2 |\n";

fn examples(i: usize) -> NodePath {
    scenario(0).push(morphir_gherkin::Segment::Examples(i))
}

#[test]
fn an_examples_path_applies_only_its_own_blocks_tags() {
    let (doc, _) = read_str("f.feature", OUTLINE_TEXT).unwrap();

    let (ctx, effect) = extensions().context_for(&doc, &examples(1)).unwrap();
    assert_eq!(
        effect,
        Effect::Continue,
        "block 0's @wip must not reach block 1"
    );
    assert_eq!(ctx.get::<Syntax>(), Some(&Syntax("gleam".to_owned())));

    let (ctx, effect) = extensions().context_for(&doc, &examples(0)).unwrap();
    assert_eq!(effect, Effect::Skip("work in progress".to_owned()));
    assert_eq!(
        ctx.get::<Syntax>(),
        Some(&Syntax("elm".to_owned())),
        "block 1's @syntax:gleam must not reach block 0"
    );
}

#[test]
fn a_scenario_path_still_applies_every_examples_block() {
    let (doc, _) = read_str("f.feature", OUTLINE_TEXT).unwrap();
    let (ctx, effect) = extensions().context_for(&doc, &scenario(0)).unwrap();
    assert_eq!(effect, Effect::Skip("work in progress".to_owned()));
    assert_eq!(ctx.get::<Syntax>(), Some(&Syntax("gleam".to_owned())));
}

struct RecordPath;
#[derive(Debug, PartialEq)]
struct Seen(NodePath);
impl Processor for RecordPath {
    fn process(
        &self,
        _doc: &morphir_gherkin::Document,
        at: &NodePath,
        ctx: &mut Context,
    ) -> Result<(), String> {
        ctx.insert(Seen(at.clone()));
        Ok(())
    }
}

#[test]
fn a_processor_gets_the_examples_path_that_was_passed_in() {
    let (doc, _) = read_str("f.feature", OUTLINE_TEXT).unwrap();
    let (ctx, _) = Extensions::new()
        .with_processor(RecordPath)
        .context_for(&doc, &examples(1))
        .unwrap();
    assert_eq!(ctx.get::<Seen>(), Some(&Seen(examples(1))));
}

#[test]
fn an_examples_path_that_does_not_exist_is_not_a_scenario() {
    let (doc, _) = read_str("f.feature", OUTLINE_TEXT).unwrap();
    let errors = extensions().context_for(&doc, &examples(5)).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].to_string().contains("not a scenario"));
}
