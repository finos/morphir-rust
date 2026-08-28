//! The OKF vocabulary as a value: known keys, producer keys, register-owned
//! types, and statuses.
//!
//! Ported from `Frontmatter.Known` / `ProducerKnown` / `Statuses` and
//! `KbRegisters` in `KbModel.scala`, lifted into a profile value so that
//! downstream producers can extend the vocabulary without forking the parser.
//! Checks and parsers take a `&OkfProfile` rather than hardcoding the sets.

use std::collections::BTreeSet;

/// The key sets and owned types a producer recognizes on top of OKF itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OkfProfile {
    /// Frontmatter keys OKF v0.2 defines. Anything else is producer-specific
    /// and merely reported, never rejected.
    pub known_keys: BTreeSet<String>,
    /// Keys the tooling defines on top of OKF, and therefore understands —
    /// the schema of the intent and decision registers. Kept separate from
    /// `known_keys` to keep the distinction honest: `known_keys` is what the
    /// spec says, `producer_known_keys` is what we added.
    pub producer_known_keys: BTreeSet<String>,
    /// The `type:` values a register claims, wherever a document carrying one
    /// sits. A mirrored document must never be injected with one of these — it
    /// would be pulled into the register and judged against a schema that is
    /// not its own.
    pub register_owned_types: Vec<String>,
    /// Recognized `status:` maturity values.
    pub statuses: BTreeSet<String>,
}

impl OkfProfile {
    /// Keys that neither OKF nor the producer defines are the only ones worth
    /// reporting.
    pub fn is_recognized(&self, key: &str) -> bool {
        self.known_keys.contains(key) || self.producer_known_keys.contains(key)
    }

    /// True when a register owns this `type:` value (case-insensitive,
    /// trimmed).
    pub fn owns_type(&self, t: &str) -> bool {
        let trimmed = t.trim();
        self.register_owned_types
            .iter()
            .any(|owned| owned.eq_ignore_ascii_case(trimmed))
    }

    /// True when `status` is a recognized maturity value.
    pub fn is_known_status(&self, status: &str) -> bool {
        self.statuses.contains(status)
    }
}

fn string_set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

impl Default for OkfProfile {
    /// The OKF v0.2 vocabulary plus the morphir producer keys, exactly as in
    /// `KbModel.scala`.
    fn default() -> Self {
        OkfProfile {
            known_keys: string_set(&[
                "type",
                "title",
                "description",
                "resource",
                "tags",
                "sources",
                "generated",
                "verified",
                "status",
                "stale_after",
                "runtime",
                "parameters",
                "computation",
                "executor",
                "attester",
                "okf_version",
                // Vendoring: `sync` marks a bundle that mirrors an upstream
                // repository, `kb_upstream` records which file a mirrored
                // concept came from. Both are stripped back out on export.
                "sync",
                "kb_upstream",
            ]),
            producer_known_keys: string_set(&[
                // intent register
                "state",
                "kind",
                "breaking",
                "created",
                "state_since",
                "issue",
                "capability",
                "superseded_by",
                "reason",
                "artifacts",
                "implementation_baselines",
                // intent bundle configuration, on the bundle-root index only
                "intent",
                "system",
                "capability_bundle",
                "stale_after_days",
                // decision register — `state`, `superseded_by` and `reason`
                // are shared with intent above
                "decided",
                "supersedes",
            ]),
            register_owned_types: vec!["Decision Record".to_string()],
            statuses: string_set(&["draft", "stable", "deprecated"]),
        }
    }
}
