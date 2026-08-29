use std::{fmt, sync::LazyLock};

use chrono::NaiveDate;
use morphir_okf::{
    frontmatter::Frontmatter,
    model::{Bundle, Doc, Kb},
};
use regex::Regex;
use serde::Serialize;

// -------------------------------------------------------------------- states

/// Lifecycle state of an intent record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum IntentState {
    Backlog,
    Refinement,
    InProgress,
    Released,
    Cancelled,
    Superseded,
}

impl IntentState {
    /// Declaration order, as the Scala enum's `values`.
    pub const ALL: [IntentState; 6] = [
        IntentState::Backlog,
        IntentState::Refinement,
        IntentState::InProgress,
        IntentState::Released,
        IntentState::Cancelled,
        IntentState::Superseded,
    ];

    /// Display order: live work first, then terminal records.
    pub const DISPLAY_ORDER: [IntentState; 6] = [
        IntentState::InProgress,
        IntentState::Refinement,
        IntentState::Backlog,
        IntentState::Released,
        IntentState::Superseded,
        IntentState::Cancelled,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            IntentState::Backlog => "Backlog",
            IntentState::Refinement => "Refinement",
            IntentState::InProgress => "InProgress",
            IntentState::Released => "Released",
            IntentState::Cancelled => "Cancelled",
            IntentState::Superseded => "Superseded",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            IntentState::Released | IntentState::Cancelled | IntentState::Superseded
        )
    }

    /// States where nothing moving is a real signal. Backlog is excluded — a
    /// backlog is *meant* to sit.
    pub fn is_active(&self) -> bool {
        matches!(self, IntentState::Refinement | IntentState::InProgress)
    }

    /// Case-insensitive, hyphens stripped — `in-progress` parses.
    pub fn parse(s: &str) -> Option<IntentState> {
        let needle = s.trim().replace('-', "");
        Self::ALL
            .into_iter()
            .find(|st| st.as_str().eq_ignore_ascii_case(&needle))
    }

    /// The state names joined `", "`, for guard hints.
    pub fn names() -> String {
        Self::ALL
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl fmt::Display for IntentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// --------------------------------------------------------------------- kinds

/// What sort of work an intent records. The user-visible tier decides whether
/// releasing owes a capability link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IntentKind {
    Feature,
    Bug,
    Performance,
    Security,
    Deprecation,
    Removal,
    Refactor,
    Docs,
    Test,
    Build,
    Spike,
}

impl IntentKind {
    pub const ALL: [IntentKind; 11] = [
        IntentKind::Feature,
        IntentKind::Bug,
        IntentKind::Performance,
        IntentKind::Security,
        IntentKind::Deprecation,
        IntentKind::Removal,
        IntentKind::Refactor,
        IntentKind::Docs,
        IntentKind::Test,
        IntentKind::Build,
        IntentKind::Spike,
    ];

    pub fn user_visible(&self) -> bool {
        matches!(
            self,
            IntentKind::Feature
                | IntentKind::Bug
                | IntentKind::Performance
                | IntentKind::Security
                | IntentKind::Deprecation
                | IntentKind::Removal
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            IntentKind::Feature => "feature",
            IntentKind::Bug => "bug",
            IntentKind::Performance => "performance",
            IntentKind::Security => "security",
            IntentKind::Deprecation => "deprecation",
            IntentKind::Removal => "removal",
            IntentKind::Refactor => "refactor",
            IntentKind::Docs => "docs",
            IntentKind::Test => "test",
            IntentKind::Build => "build",
            IntentKind::Spike => "spike",
        }
    }

    pub fn parse(s: &str) -> Option<IntentKind> {
        let needle = s.trim().to_lowercase();
        Self::ALL.into_iter().find(|k| k.label() == needle)
    }

    /// The kind labels joined `", "`, for help text and guard hints.
    pub fn names() -> String {
        Self::ALL
            .iter()
            .map(|k| k.label())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

// -------------------------------------------------------------------- DocRef

static DOC_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([^:\s]+):(/[^\s]*)$").expect("valid regex"));

/// A reference to a document elsewhere in the knowledge base:
/// `bundle-label:/path.md`.
///
/// Deliberately not a Package URL — purl identifies registry-backed
/// artifacts, and a Capability is a markdown document no registry knows about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocRef {
    pub bundle: String,
    pub path: String,
}

impl DocRef {
    pub fn render(&self) -> String {
        format!("{}:{}", self.bundle, self.path)
    }

    pub fn parse(s: &str) -> Option<DocRef> {
        DOC_REF.captures(s.trim()).map(|caps| DocRef {
            bundle: caps[1].to_string(),
            path: caps[2].to_string(),
        })
    }
}

// -------------------------------------------------------------------- config

/// Settings declared in the intent bundle's own index frontmatter, so the
/// tooling is portable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentConfig {
    pub system: Option<String>,
    pub capability_bundle: Option<String>,
    pub stale_after_days: i64,
}

impl IntentConfig {
    pub const DEFAULT_STALE_AFTER_DAYS: i64 = 60;

    pub fn from(fm: &Frontmatter) -> IntentConfig {
        IntentConfig {
            system: fm.str_at("system"),
            capability_bundle: fm.str_at("capability_bundle"),
            stale_after_days: fm
                .str_at("stale_after_days")
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(Self::DEFAULT_STALE_AFTER_DAYS),
        }
    }
}

// -------------------------------------------------------------------- record

/// A view over a concept document with `type: Intent`.
#[derive(Debug, Clone, Copy)]
pub struct Intent<'a> {
    pub doc: &'a Doc,
}

impl<'a> Intent<'a> {
    pub(super) fn fm(&self) -> &Frontmatter {
        self.doc.fm()
    }

    /// Leading digits of the filename, e.g. `0004` from `0004-thing.md`.
    pub fn id(&self) -> String {
        self.doc
            .name()
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect()
    }

    pub fn slug(&self) -> String {
        self.doc
            .name()
            .strip_suffix(".md")
            .unwrap_or(self.doc.name())
            .to_string()
    }

    pub fn title(&self) -> String {
        self.doc.display_title()
    }

    pub fn description(&self) -> Option<String> {
        self.fm().description()
    }

    pub fn state(&self) -> Option<IntentState> {
        self.fm()
            .str_at("state")
            .and_then(|s| IntentState::parse(&s))
    }

    pub fn kind(&self) -> Option<IntentKind> {
        self.fm().str_at("kind").and_then(|s| IntentKind::parse(&s))
    }

    pub fn breaking(&self) -> bool {
        self.fm().bool_at("breaking").unwrap_or(false)
    }

    pub fn created(&self) -> Option<NaiveDate> {
        self.date("created")
    }

    pub fn state_since(&self) -> Option<NaiveDate> {
        self.date("state_since")
    }

    pub fn issue(&self) -> Option<String> {
        self.fm().str_at("issue")
    }

    pub fn capability(&self) -> Option<String> {
        self.fm().str_at("capability")
    }

    pub fn superseded_by(&self) -> Option<String> {
        self.fm().str_at("superseded_by")
    }

    pub fn reason(&self) -> Option<String> {
        self.fm().str_at("reason")
    }

    pub fn artifacts(&self) -> Vec<String> {
        self.fm().list_at("artifacts")
    }

    fn date(&self, key: &str) -> Option<NaiveDate> {
        self.fm()
            .str_at(key)
            .and_then(|s| NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok())
    }

    pub fn days_since_state_change(&self, today: NaiveDate) -> Option<i64> {
        self.state_since().map(|d| (today - d).num_days())
    }
}

// ----------------------------------------------------------------- discovery

/// The intent bundle is the one whose index frontmatter carries
/// `intent: true`. Nothing is hardcoded, so the tooling works in any
/// repository whatever the bundle is called.
pub fn find_bundle(kb: &Kb) -> Option<&Bundle> {
    kb.bundles
        .iter()
        .find(|b| b.index.fm().bool_at("intent").unwrap_or(false))
}

pub fn config(b: &Bundle) -> IntentConfig {
    IntentConfig::from(b.index.fm())
}

/// Every intent record in the bundle, sorted by slug.
pub fn intents(b: &Bundle) -> Vec<Intent<'_>> {
    let mut out: Vec<Intent<'_>> = b
        .concepts
        .iter()
        .filter(|d| {
            d.fm()
                .doc_type()
                .is_some_and(|t| t.eq_ignore_ascii_case("Intent"))
        })
        .map(|doc| Intent { doc })
        .collect();
    out.sort_by_key(|i| i.slug());
    out
}

/// The next free id: `max(existing numeric ids) + 1`, zero-padded to 4.
pub fn next_id(b: &Bundle) -> String {
    let max = intents(b)
        .iter()
        .filter_map(|i| i.id().parse::<i64>().ok())
        .max()
        .unwrap_or(0);
    format!("{:04}", max + 1)
}

/// Finds an intent by id (`0007` or bare `7`) or slug.
pub fn find<'a>(b: &'a Bundle, id_or_slug: &str) -> Option<Intent<'a>> {
    let needle = id_or_slug.trim();
    let padded = needle.parse::<i64>().ok().map(|n| format!("{n:04}"));
    intents(b).into_iter().find(|i| {
        let id = i.id();
        id == needle || i.slug() == needle || padded.as_deref() == Some(id.as_str())
    })
}

/// `None` when the reference resolves; `Some(message)` when it does not.
pub fn resolve_ref(kb: &Kb, r: &DocRef) -> Option<String> {
    match kb.bundle(&r.bundle) {
        None => Some(format!(
            "no bundle `{}` (known: {})",
            r.bundle,
            kb.bundles
                .iter()
                .map(|b| b.label())
                .collect::<Vec<_>>()
                .join(", ")
        )),
        Some(target) => {
            if target.concept_at(&r.path).is_none() {
                Some(format!(
                    "`{}` names no concept in {}",
                    r.render(),
                    target.label()
                ))
            } else {
                None
            }
        }
    }
}
