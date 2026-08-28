//! Domain model for an Open Knowledge Format knowledge base.
//!
//! Pure data and pure functions only — see [`crate::store`] for anything that
//! touches the filesystem. Ported from `KbModel.scala`.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::frontmatter::Frontmatter;
use crate::paths;

/// Severity of a check finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warn,
    Info,
}

impl Severity {
    pub fn label(&self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warn => "warn",
            Severity::Info => "info",
        }
    }
}

/// One problem found by a check. Pure data, so that any module producing
/// findings — structural checks, provenance, the intent and decision
/// registers — can do so without depending on a check runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    pub check: String,
    pub path: String,
    pub line: Option<usize>,
    pub message: String,
    pub hint: Option<String>,
}

impl Finding {
    /// `path:line` when a line is known, else just the path.
    pub fn location(&self) -> String {
        match self.line {
            Some(l) => format!("{}:{}", self.path, l),
            None => self.path.clone(),
        }
    }
}

/// What role a markdown file plays inside a bundle. Only `index.md` and
/// `log.md` are reserved by OKF; every other `.md` file is a concept document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum DocKind {
    RootIndex,
    SubIndex,
    Log,
    Concept,
}

/// A link found in a document body. `line` is 1-based and refers to the
/// original file (shifted past any frontmatter).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkRef {
    pub text: String,
    pub dest: String,
    pub line: usize,
}

impl LinkRef {
    pub fn is_external(&self) -> bool {
        self.dest.starts_with("http://")
            || self.dest.starts_with("https://")
            || self.dest.starts_with("mailto:")
    }

    pub fn is_anchor_only(&self) -> bool {
        self.dest.starts_with('#')
    }

    pub fn is_bundle_relative(&self) -> bool {
        self.dest.starts_with('/')
    }
}

/// A provenance entry from the `sources` frontmatter family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceRef {
    pub id: Option<String>,
    pub resource: String,
    pub title: Option<String>,
}

/// A non-markdown file mirrored into a bundle: a schema, a fixture, a sidebar
/// descriptor. Assets are carried and synced but never parsed — they have no
/// frontmatter to hold a `type`, so treating them as concepts would mean every
/// one of them failing the checks that make concepts useful.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub file: PathBuf,
    pub bundle_root: PathBuf,
    /// Path segments below the bundle root.
    pub rel: Vec<String>,
}

impl Asset {
    /// Bundle-relative path in OKF link form, e.g. `/sources/schema.json`.
    pub fn bundle_path(&self) -> String {
        join_bundle_path(&self.rel)
    }

    pub fn name(&self) -> &str {
        self.rel.last().map(String::as_str).unwrap_or("")
    }
}

fn join_bundle_path(rel: &[String]) -> String {
    let joined = format!("/{}", rel.join("/"));
    joined
        .strip_suffix('/')
        .map(str::to_string)
        .unwrap_or(joined)
}

static EMPTY_FRONTMATTER: LazyLock<Frontmatter> = LazyLock::new(Frontmatter::empty);

/// A single markdown file within a bundle. `rel` holds its path segments
/// below the bundle root.
#[derive(Debug, Clone, PartialEq)]
pub struct Doc {
    pub file: PathBuf,
    pub bundle_root: PathBuf,
    pub rel: Vec<String>,
    pub kind: DocKind,
    pub frontmatter: Option<Frontmatter>,
    /// True when a leading `---` block was present, whether or not it parsed.
    pub has_frontmatter_block: bool,
    /// Set when a frontmatter block was present but did not parse as YAML.
    pub frontmatter_error: Option<String>,
    pub body: String,
    pub links: Vec<LinkRef>,
    /// True when the file is mirrored from upstream rather than authored here.
    pub vendored: bool,
    /// Lines the stripped frontmatter block occupied, so body-relative line
    /// numbers can be shifted to file ones.
    pub frontmatter_lines: usize,
}

impl Doc {
    /// Bundle-relative path in OKF link form, e.g. `/design/annotations.md`.
    pub fn bundle_path(&self) -> String {
        join_bundle_path(&self.rel)
    }

    pub fn name(&self) -> &str {
        self.rel.last().map(String::as_str).unwrap_or("")
    }

    pub fn is_concept(&self) -> bool {
        self.kind == DocKind::Concept
    }

    /// The parsed frontmatter, or empty frontmatter when absent or broken.
    pub fn fm(&self) -> &Frontmatter {
        self.frontmatter.as_ref().unwrap_or(&EMPTY_FRONTMATTER)
    }

    /// The frontmatter title, falling back to the filename without `.md`.
    pub fn display_title(&self) -> String {
        self.fm().title().unwrap_or_else(|| {
            self.name()
                .strip_suffix(".md")
                .unwrap_or(self.name())
                .to_string()
        })
    }
}

/// An OKF bundle: a directory whose root `index.md` carries `okf_version`.
#[derive(Debug, Clone, PartialEq)]
pub struct Bundle {
    pub root: PathBuf,
    pub name: String,
    pub group: Option<String>,
    pub index: Doc,
    pub log: Option<Doc>,
    pub sub_indexes: Vec<Doc>,
    pub concepts: Vec<Doc>,
    pub assets: Vec<Asset>,
    /// Segments of the mirrored subtree, when the bundle vendors an upstream
    /// repository.
    pub mirror: Option<Vec<String>>,
}

impl Bundle {
    pub fn mirror_root(&self) -> Option<PathBuf> {
        self.mirror.as_ref().map(|segs| {
            let mut p = self.root.clone();
            for s in segs {
                p.push(s);
            }
            p
        })
    }

    /// `group/name` when grouped, else just the name.
    pub fn label(&self) -> String {
        match &self.group {
            Some(g) => format!("{g}/{}", self.name),
            None => self.name.clone(),
        }
    }

    pub fn all_docs(&self) -> Vec<&Doc> {
        let mut out = vec![&self.index];
        out.extend(self.log.iter());
        out.extend(self.sub_indexes.iter());
        out.extend(self.concepts.iter());
        out
    }

    /// Concepts written here rather than mirrored — what most checks and
    /// reports mean by "the bundle's concepts".
    pub fn authored_concepts(&self) -> Vec<&Doc> {
        self.concepts.iter().filter(|d| !d.vendored).collect()
    }

    pub fn all_indexes(&self) -> Vec<&Doc> {
        let mut out = vec![&self.index];
        out.extend(self.sub_indexes.iter());
        out
    }

    pub fn okf_version(&self) -> Option<String> {
        self.index.fm().okf_version()
    }

    pub fn concept_at(&self, bundle_path: &str) -> Option<&Doc> {
        self.concepts
            .iter()
            .find(|d| d.bundle_path() == bundle_path)
    }
}

/// A loaded knowledge base.
#[derive(Debug, Clone, PartialEq)]
pub struct Kb {
    pub root: PathBuf,
    pub bundles: Vec<Bundle>,
    pub strays: Vec<PathBuf>,
}

impl Kb {
    /// Finds a bundle by its label (`group/name`) or bare name.
    pub fn bundle(&self, label: &str) -> Option<&Bundle> {
        self.bundles
            .iter()
            .find(|b| b.label() == label || b.name == label)
    }

    /// Every concept in the knowledge base, with its bundle.
    pub fn concepts(&self) -> Vec<(&Bundle, &Doc)> {
        self.bundles
            .iter()
            .flat_map(|b| b.concepts.iter().map(move |d| (b, d)))
            .collect()
    }

    /// Renders `p` relative to the kb root (prefixed with the root's own
    /// name, e.g. `kb/bundles/x/y.md`), or as a rendered absolute path when
    /// it is not under the root.
    pub fn rel(&self, p: &Path) -> String {
        match paths::segments_under(p, &self.root) {
            Some(segs) => {
                let root_name = self
                    .root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .into_iter()
                    .chain(segs)
                    .collect::<Vec<_>>();
                root_name.join("/")
            }
            None => paths::render(p),
        }
    }
}

/// An index entry bullet: `* [Title](/path.md) - description`.
///
/// `link` holds everything through the closing paren (bullet included) so a
/// rewrite can keep the link untouched; `description` may be absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexEntry {
    pub link: String,
    pub title: String,
    pub dest: String,
    pub description: Option<String>,
}

static INDEX_ENTRY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\s*[*-]\s+\[([^\]]*)\]\(([^)]+)\))\s*(?:-\s*(.*))?$").expect("valid regex")
});

/// Parses an index-entry bullet line, or returns `None` when the line is not
/// one. Scaffold, refresh, and the drift checks all share this shape.
pub fn parse_index_entry(line: &str) -> Option<IndexEntry> {
    let caps = INDEX_ENTRY.captures(line)?;
    Some(IndexEntry {
        link: caps.get(1).map(|m| m.as_str().to_string())?,
        title: caps.get(2).map(|m| m.as_str().to_string())?,
        dest: caps.get(3).map(|m| m.as_str().to_string())?,
        description: caps.get(4).map(|m| m.as_str().to_string()),
    })
}
