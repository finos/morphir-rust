use morphir_okf::model::{Bundle, Kb};
use serde::Serialize;

use super::model::{Intent, IntentKind, IntentState};

// ----------------------------------------------------------------- rendering

/// JSON shape of one intent, with field names matching the Scala CLI output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntentJson {
    pub id: String,
    pub slug: String,
    pub path: String,
    pub title: String,
    pub description: Option<String>,
    pub state: Option<IntentState>,
    pub kind: Option<IntentKind>,
    #[serde(rename = "userVisible")]
    pub user_visible: Option<bool>,
    pub breaking: bool,
    pub created: Option<String>,
    #[serde(rename = "stateSince")]
    pub state_since: Option<String>,
    pub issue: Option<String>,
    pub capability: Option<String>,
    #[serde(rename = "supersededBy")]
    pub superseded_by: Option<String>,
    pub artifacts: Vec<String>,
}

/// JSON shape of `kb intent list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntentListJson {
    pub bundle: String,
    pub count: usize,
    pub intents: Vec<IntentJson>,
}

pub fn intent_json(i: &Intent<'_>) -> IntentJson {
    IntentJson {
        id: i.id(),
        slug: i.slug(),
        path: i.doc.bundle_path(),
        title: i.title(),
        description: i.description(),
        state: i.state(),
        kind: i.kind(),
        user_visible: i.kind().map(|k| k.user_visible()),
        breaking: i.breaking(),
        created: i.created().map(|d| d.to_string()),
        state_since: i.state_since().map(|d| d.to_string()),
        issue: i.issue(),
        capability: i.capability(),
        superseded_by: i.superseded_by(),
        artifacts: i.artifacts(),
    }
}

fn to_pretty_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("serializable") + "\n"
}

/// Renders `kb intent list`, grouped by state in display order.
pub fn render_list(b: &Bundle, items: &[Intent<'_>], json: bool) -> String {
    if json {
        return to_pretty_json(&IntentListJson {
            bundle: b.label(),
            count: items.len(),
            intents: items.iter().map(intent_json).collect(),
        });
    }
    if items.is_empty() {
        return "no matching intent\n".to_string();
    }
    let mut sb = String::new();
    for st in IntentState::DISPLAY_ORDER {
        let group: Vec<&Intent<'_>> = items.iter().filter(|i| i.state() == Some(st)).collect();
        if !group.is_empty() {
            sb.push_str(&format!(
                "\n{} ({})\n",
                st.as_str().to_uppercase(),
                group.len()
            ));
            for i in group {
                let mut flags: Vec<&str> = Vec::new();
                if i.breaking() {
                    flags.push("breaking");
                }
                if let Some(k) = i.kind() {
                    flags.push(k.label());
                }
                sb.push_str(&format!(
                    "  {:<6} {:<48} {}\n",
                    i.id(),
                    i.title(),
                    flags.join(", ")
                ));
            }
        }
    }
    let orphan: Vec<&Intent<'_>> = items.iter().filter(|i| i.state().is_none()).collect();
    if !orphan.is_empty() {
        sb.push_str(&format!("\nNO STATE ({})\n", orphan.len()));
        for i in orphan {
            sb.push_str(&format!("  {}   {}\n", i.id(), i.title()));
        }
    }
    sb.push_str(&format!("\n{} intent\n", items.len()));
    sb
}

/// Renders `kb intent show`.
pub fn render_show(kb: &Kb, i: &Intent<'_>, json: bool) -> String {
    if json {
        return to_pretty_json(&intent_json(i));
    }
    let mut sb = String::new();
    sb.push_str(&format!("intent {} — {}\n", i.id(), i.title()));
    if let Some(d) = i.description() {
        sb.push_str(&format!("{d}\n"));
    }
    sb.push_str(&format!(
        "\nstate        {}",
        i.state()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "(missing)".to_string())
    ));
    if let Some(d) = i.state_since() {
        sb.push_str(&format!("  since {d}"));
    }
    sb.push('\n');
    sb.push_str(&format!(
        "kind         {}",
        i.kind().map(|k| k.label()).unwrap_or("(missing)")
    ));
    if i.breaking() {
        sb.push_str("  BREAKING");
    }
    sb.push('\n');
    if let Some(d) = i.created() {
        sb.push_str(&format!("created      {d}\n"));
    }
    if let Some(x) = i.issue() {
        sb.push_str(&format!("issue        #{x}\n"));
    }
    if let Some(c) = i.capability() {
        sb.push_str(&format!("capability   {c}\n"));
    }
    if let Some(x) = i.superseded_by() {
        sb.push_str(&format!("superseded   by {x}\n"));
    }
    if let Some(r) = i.reason() {
        sb.push_str(&format!("reason       {r}\n"));
    }
    let artifacts = i.artifacts();
    if !artifacts.is_empty() {
        sb.push_str(&format!(
            "artifacts    {}\n",
            artifacts.join("\n             ")
        ));
    }
    sb.push_str(&format!("file         {}\n", kb.rel(&i.doc.file)));
    sb
}
