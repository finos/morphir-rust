//! Intent management — features, enhancements and bugs recorded as prose with
//! a lifecycle. Ported from `KbIntent.scala` and `KbIntentEdit.scala`.
//!
//! The distinction this rests on: an **Intent** is future-tense and has a
//! lifecycle; a **Capability** is present-tense and is simply either true or
//! stale. Releasing is where they meet, which is why marking an Intent
//! Released demands a link to the Capability it produced.
//!
//! Obligations are checked wherever a record currently sits, never against the
//! path it took to get there — work genuinely jumps stages, and a tool that
//! fights that gets worked around.

/// Everything below this marker in the intent bundle's index is regenerated.
pub const MARKER: &str = "<!-- intent:index -->";

mod check;
mod edit;
mod index;
mod model;
mod render;

pub use check::check;
pub use edit::{Transition, create, init_bundle, set_keys, transition};
pub use index::generate_index;
pub use model::{
    DocRef, Intent, IntentConfig, IntentKind, IntentState, config, find, find_bundle, intents,
    next_id, resolve_ref,
};
pub use render::{IntentJson, IntentListJson, intent_json, render_list, render_show};
