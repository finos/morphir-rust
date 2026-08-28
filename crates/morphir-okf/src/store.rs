//! Loading and parsing: filesystem to [`Kb`]. Ported from `KbStore.scala`.
//!
//! Frontmatter is parsed with serde_yaml (full YAML, so nested `sources`
//! entries survive) and the body with pulldown-cmark, so links come from a
//! real parser rather than a regex that would trip over code fences.

use std::fs;
use std::path::{Path, PathBuf};

use serde_yaml::Value;

use crate::error::{Error, Result};
use crate::frontmatter::{self, Frontmatter};
use crate::markdown;
use crate::model::{Asset, Bundle, Doc, DocKind, Kb, LinkRef};
use crate::paths;

// ------------------------------------------------------------- pure parsing

/// What role a file at the given bundle-relative segments plays.
pub fn kind_of(rel: &[String]) -> DocKind {
    match rel.last().map(String::as_str) {
        Some("log.md") => DocKind::Log,
        Some("index.md") => {
            if rel.len() == 1 {
                DocKind::RootIndex
            } else {
                DocKind::SubIndex
            }
        }
        _ => DocKind::Concept,
    }
}

/// Builds a [`Doc`] from already-read text — the pure core of [`load_doc`].
pub fn parse_doc(file: &Path, bundle_root: &Path, text: &str) -> Doc {
    let (raw_fm, body) = frontmatter::split_frontmatter(text);
    let parsed = raw_fm.as_deref().map(frontmatter::parse_frontmatter);
    let fm_lines = raw_fm
        .as_deref()
        .map(frontmatter::frontmatter_line_count)
        .unwrap_or(0);
    let rel = paths::segments_under(file, bundle_root).unwrap_or_else(|| last_segment(file));
    let (fm, fm_error): (Option<Frontmatter>, Option<String>) = match parsed {
        Some(Ok(f)) => (Some(f), None),
        Some(Err(e)) => (None, Some(e)),
        None => (None, None),
    };
    let kind = kind_of(&rel);
    let links = markdown::extract_links(&body, fm_lines);
    Doc {
        file: file.to_path_buf(),
        bundle_root: bundle_root.to_path_buf(),
        rel,
        kind,
        frontmatter: fm,
        has_frontmatter_block: raw_fm.is_some(),
        frontmatter_error: fm_error,
        body,
        links,
        vendored: false,
        frontmatter_lines: fm_lines,
    }
}

fn last_segment(p: &Path) -> Vec<String> {
    p.file_name()
        .map(|n| vec![n.to_string_lossy().into_owned()])
        .unwrap_or_default()
}

// --------------------------------------------------------------------- I/O

/// True when the path's file name ends with `.md`.
pub fn is_markdown(p: &Path) -> bool {
    p.file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with(".md"))
}

/// Reads and parses a single document.
pub fn load_doc(file: &Path, bundle_root: &Path) -> Result<Doc> {
    let text = fs::read_to_string(file)?;
    Ok(parse_doc(file, bundle_root, &text))
}

/// A directory is a bundle root when its `index.md` frontmatter carries
/// `okf_version`. A broken or missing frontmatter block means it is not one.
fn is_bundle_root(dir: &Path) -> Result<bool> {
    let idx = dir.join("index.md");
    if !idx.is_file() {
        return Ok(false);
    }
    let text = fs::read_to_string(&idx)?;
    let (raw_fm, _) = frontmatter::split_frontmatter(&text);
    Ok(raw_fm
        .as_deref()
        .and_then(|raw| frontmatter::parse_frontmatter(raw).ok())
        .is_some_and(|fm| fm.has("okf_version")))
}

/// Directory entries, sorted for deterministic traversal.
fn sorted_entries(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::io::Result<_>>()?;
    entries.sort();
    Ok(entries)
}

/// Finds bundle roots under `start`, not descending into a bundle once one is
/// found.
pub fn find_bundle_roots(start: &Path) -> Result<Vec<PathBuf>> {
    if !start.exists() {
        return Ok(Vec::new());
    }
    if is_bundle_root(start)? {
        return Ok(vec![start.to_path_buf()]);
    }
    let mut out = Vec::new();
    for entry in sorted_entries(start)? {
        if entry.is_dir() {
            out.extend(find_bundle_roots(&entry)?);
        }
    }
    Ok(out)
}

/// Every markdown file at or below `dir`.
pub fn markdown_under(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in sorted_entries(dir)? {
        if entry.is_dir() {
            out.extend(markdown_under(&entry)?);
        } else if is_markdown(&entry) {
            out.push(entry);
        }
    }
    Ok(out)
}

/// Every file at or below `dir` that is *not* markdown — the mirrored assets
/// of a sync bundle.
pub fn non_markdown_under(dir: &Path) -> Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in sorted_entries(dir)? {
        if entry.is_dir() {
            out.extend(non_markdown_under(&entry)?);
        } else if !is_markdown(&entry) {
            out.push(entry);
        }
    }
    Ok(out)
}

/// The `root:` of a bundle's `sync.yaml`, as path segments, when the bundle
/// mirrors an upstream repository.
///
/// Read here rather than in the sync layer because loading has to know which
/// files are vendored before anything else can. Any parse problem — or a
/// missing `root:` key — falls back to the default `sources`; the manifest's
/// own validation belongs to the sync layer.
pub fn mirror_segments(bundle_root: &Path) -> Result<Option<Vec<String>>> {
    let manifest = bundle_root.join("sync.yaml");
    if !manifest.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&manifest)?;
    let root = serde_yaml::from_str::<Value>(&raw)
        .ok()
        .and_then(|v| match v {
            Value::Mapping(m) => m
                .get(Value::String("root".to_string()))
                .and_then(|r| r.as_str().map(str::to_string)),
            _ => None,
        })
        .unwrap_or_else(|| "sources".to_string());
    Ok(Some(
        root.split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
    ))
}

/// Loads one bundle rooted at `root`, inside the kb rooted at `kb_root`.
pub fn load_bundle(root: &Path, kb_root: &Path) -> Result<Bundle> {
    let mirror = mirror_segments(root)?;
    let files = markdown_under(root)?;
    let mut docs = files
        .iter()
        .map(|f| load_doc(f, root))
        .collect::<Result<Vec<_>>>()?;

    let asset_files = match &mirror {
        None => Vec::new(),
        Some(segs) => {
            let mut mirror_root = root.to_path_buf();
            for s in segs {
                mirror_root.push(s);
            }
            non_markdown_under(&mirror_root)?
        }
    };

    // Inside the mirror, `index.md` and `log.md` are upstream's own files and
    // carry upstream's own frontmatter. OKF reserves those names for the
    // *bundle*, so the reservation stops at the mirror boundary.
    for d in docs.iter_mut() {
        let vendored = mirror
            .as_ref()
            .is_some_and(|m| d.rel.len() > m.len() && d.rel[..m.len()] == m[..]);
        if vendored {
            d.vendored = true;
            d.kind = DocKind::Concept;
        }
    }
    docs.sort_by_key(|d| d.rel.join("/"));

    let mut assets: Vec<Asset> = asset_files
        .into_iter()
        .map(|f| {
            let rel = paths::segments_under(&f, root).unwrap_or_else(|| last_segment(&f));
            Asset {
                file: f,
                bundle_root: root.to_path_buf(),
                rel,
            }
        })
        .collect();
    assets.sort_by_key(|a| a.rel.join("/"));

    let bundles_dir = kb_root.join("bundles");
    let segs = paths::segments_under(root, &bundles_dir).unwrap_or_else(|| last_segment(root));

    let mut index: Option<Doc> = None;
    let mut log: Option<Doc> = None;
    let mut sub_indexes = Vec::new();
    let mut concepts = Vec::new();
    for d in docs {
        match d.kind {
            DocKind::RootIndex if index.is_none() => index = Some(d),
            DocKind::Log if d.rel.len() == 1 && log.is_none() => log = Some(d),
            DocKind::SubIndex => sub_indexes.push(d),
            DocKind::Concept => concepts.push(d),
            // A nested, non-vendored log.md belongs to no collection, as in
            // the reference implementation.
            _ => {}
        }
    }
    let index = index.ok_or_else(|| Error::MissingRootIndex(paths::render(root)))?;

    Ok(Bundle {
        root: root.to_path_buf(),
        name: root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "?".to_string()),
        group: (segs.len() > 1).then(|| segs[..segs.len() - 1].join("/")),
        index,
        log,
        sub_indexes,
        concepts,
        assets,
        mirror,
    })
}

/// Loads the whole knowledge base rooted at `kb_root` — the directory holding
/// `bundles/`.
pub fn load(kb_root: &Path) -> Result<Kb> {
    let bundles_dir = kb_root.join("bundles");
    let mut roots = find_bundle_roots(&bundles_dir)?;
    roots.sort();
    let bundles = roots
        .iter()
        .map(|r| load_bundle(r, kb_root))
        .collect::<Result<Vec<_>>>()?;
    let all_md = if bundles_dir.exists() {
        markdown_under(&bundles_dir)?
    } else {
        Vec::new()
    };
    // A README.md in a grouping directory is expected and correct — it is how
    // a group announces itself without being mistaken for a bundle root.
    let strays = all_md
        .into_iter()
        .filter(|p| !bundles.iter().any(|b| paths::is_under(p, &b.root)))
        .filter(|p| p.file_name().is_none_or(|n| n != "README.md"))
        .collect();
    Ok(Kb {
        root: kb_root.to_path_buf(),
        bundles,
        strays,
    })
}

/// Resolves a link destination to a path, or `None` when it is external, an
/// anchor, or empty.
///
/// A `/`-prefixed destination resolves against the bundle root without
/// collapsing `..` segments, so an escape above the bundle stays visible to
/// the checks. A relative destination resolves against the containing
/// directory — the directory path itself, not a rebuilt segment list — with
/// `.` and `..` collapsed.
pub fn resolve_link(doc: &Doc, link: &LinkRef) -> Option<PathBuf> {
    if link.is_external() || link.is_anchor_only() || link.dest.is_empty() {
        return None;
    }
    let dest = link.dest.split('#').next().unwrap_or("");
    if dest.is_empty() {
        return None;
    }
    if let Some(rest) = dest.strip_prefix('/') {
        return Some(doc.bundle_root.join(rest));
    }
    let mut acc = doc.file.parent().map(Path::to_path_buf).unwrap_or_default();
    for seg in dest.split('/') {
        match seg {
            "." | "" => {}
            ".." => {
                acc.pop();
            }
            s => acc.push(s),
        }
    }
    Some(acc)
}
