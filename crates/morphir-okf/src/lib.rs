//! Open Knowledge Format (OKF) support: the pure format model and parsing layer.
//!
//! This crate is a Rust port of the domain-model and loading layer of the
//! morphir-scala `kb` CLI (`KbModel.scala` / `KbStore.scala`). It knows how to
//! split and parse YAML frontmatter, extract links and headings from markdown
//! bodies, discover bundles on disk, and resolve links between documents.
//!
//! It deliberately knows nothing about checks, registers (intent/decision),
//! the SQLite index, or upstream sync — those belong to downstream crates.

pub mod error;
pub mod frontmatter;
pub mod markdown;
pub mod model;
pub mod paths;
pub mod profile;
pub mod store;

pub use error::{Error, Result};
pub use frontmatter::{Frontmatter, parse_frontmatter, split_frontmatter};
pub use markdown::{Heading, extract_headings, extract_links, heading_slug};
pub use model::{
    Asset, Bundle, Doc, DocKind, Finding, IndexEntry, Kb, LinkRef, Severity, SourceRef,
    parse_index_entry,
};
pub use profile::OkfProfile;
