//! Rendering for the `kb` commands — text for humans, JSON for machines.
//! Ported from `KbRender.scala`.
//!
//! Takes primitives rather than CLI option structs, so this stays independent
//! of the argument-parsing layer. The JSON output matches the Scala CLI's
//! ujson output byte for byte — field names, key order, and the indent-2
//! layout — which is why the writer is hand-rolled ([`JVal`]) rather than
//! serde-derived: serde would alphabetize nothing, but it also cannot express
//! ujson's `[]`-on-one-line / objects-across-lines mix without ceremony.

use morphir_okf::model::{Bundle, Doc, DocKind, Kb};
use morphir_okf::paths;

use crate::scaffold::ScaffoldResult;

// -------------------------------------------------------------- JSON writer

/// A minimal JSON tree whose renderer reproduces ujson's `indent = 2` layout:
/// object fields and array elements one per line, empty containers as `{}` /
/// `[]`, a space after each colon, no trailing newline (callers add it).
pub(crate) enum JVal {
    /// An already-rendered scalar: string literal, number, bool, or null.
    Raw(String),
    Arr(Vec<JVal>),
    Obj(Vec<(String, JVal)>),
}

impl JVal {
    pub(crate) fn null() -> JVal {
        JVal::Raw("null".to_string())
    }

    pub(crate) fn str(s: &str) -> JVal {
        JVal::Raw(json_str(s))
    }

    pub(crate) fn opt_str(s: Option<&str>) -> JVal {
        match s {
            Some(v) => JVal::str(v),
            None => JVal::null(),
        }
    }

    pub(crate) fn num(n: usize) -> JVal {
        JVal::Raw(n.to_string())
    }

    pub(crate) fn bool(b: bool) -> JVal {
        JVal::Raw(b.to_string())
    }

    /// Renders with `indent` as the column of this value's closing bracket.
    pub(crate) fn render(&self, indent: usize) -> String {
        match self {
            JVal::Raw(s) => s.clone(),
            JVal::Arr(items) => {
                if items.is_empty() {
                    return "[]".to_string();
                }
                let inner = " ".repeat(indent + 2);
                let body = items
                    .iter()
                    .map(|v| format!("{inner}{}", v.render(indent + 2)))
                    .collect::<Vec<_>>()
                    .join(",\n");
                format!("[\n{body}\n{}]", " ".repeat(indent))
            }
            JVal::Obj(fields) => {
                if fields.is_empty() {
                    return "{}".to_string();
                }
                let inner = " ".repeat(indent + 2);
                let body = fields
                    .iter()
                    .map(|(k, v)| format!("{inner}{}: {}", json_str(k), v.render(indent + 2)))
                    .collect::<Vec<_>>()
                    .join(",\n");
                format!("{{\n{body}\n{}}}", " ".repeat(indent))
            }
        }
    }

    /// A whole document: rendered from column 0 with a trailing newline, as
    /// `ujson.write(…, indent = 2) + "\n"` produces.
    pub(crate) fn document(&self) -> String {
        self.render(0) + "\n"
    }
}

/// JSON string encoding matching ujson's default: standard escapes, no
/// unicode escaping of printable characters.
pub(crate) fn json_str(s: &str) -> String {
    serde_json::to_string(s).expect("a string always serializes")
}

fn obj(fields: Vec<(&str, JVal)>) -> JVal {
    JVal::Obj(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn str_arr(items: &[String]) -> JVal {
    JVal::Arr(items.iter().map(|s| JVal::str(s)).collect())
}

fn kind_str(k: DocKind) -> &'static str {
    match k {
        DocKind::RootIndex => "RootIndex",
        DocKind::SubIndex => "SubIndex",
        DocKind::Log => "Log",
        DocKind::Concept => "Concept",
    }
}

fn pad(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len >= n {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(n - len))
    }
}

// ------------------------------------------------------------------- list

/// Renders `kb list`: the bundle table (text) or `{root, bundles}` (JSON).
pub fn list_bundles(kb: &Kb, json: bool) -> String {
    if json {
        let bundles: Vec<JVal> = kb
            .bundles
            .iter()
            .map(|b| {
                obj(vec![
                    ("label", JVal::str(&b.label())),
                    ("name", JVal::str(&b.name)),
                    ("group", JVal::opt_str(b.group.as_deref())),
                    ("okfVersion", JVal::opt_str(b.okf_version().as_deref())),
                    ("title", JVal::opt_str(b.index.fm().title().as_deref())),
                    (
                        "description",
                        JVal::opt_str(b.index.fm().description().as_deref()),
                    ),
                    ("concepts", JVal::num(b.concepts.len())),
                    ("subIndexes", JVal::num(b.sub_indexes.len())),
                    ("hasLog", JVal::bool(b.log.is_some())),
                ])
            })
            .collect();
        obj(vec![
            ("root", JVal::str(&paths::render(&kb.root))),
            ("bundles", JVal::Arr(bundles)),
        ])
        .document()
    } else {
        let mut sb = String::new();
        let w = kb
            .bundles
            .iter()
            .map(|b| b.label().chars().count())
            .chain([6])
            .max()
            .unwrap_or(6);
        sb.push_str(&format!("{}  OKF   CONCEPTS  TITLE\n", pad("BUNDLE", w)));
        for b in &kb.bundles {
            sb.push_str(&format!(
                "{}  {}  {}  {}\n",
                pad(&b.label(), w),
                pad(b.okf_version().as_deref().unwrap_or("?"), 4),
                pad(&b.concepts.len().to_string(), 8),
                b.index.fm().title().unwrap_or_default()
            ));
        }
        sb.push_str(&format!(
            "\n{} bundle(s), {} concept(s)\n",
            kb.bundles.len(),
            kb.bundles.iter().map(|b| b.concepts.len()).sum::<usize>()
        ));
        if !kb.strays.is_empty() {
            sb.push_str(&format!(
                "{} stray markdown file(s) outside any bundle — run `kb check`\n",
                kb.strays.len()
            ));
        }
        sb
    }
}

/// Renders `kb list --bundle`: one bundle's concepts.
pub fn list_concepts(kb: &Kb, b: &Bundle, json: bool) -> String {
    let _ = kb; // parity with the Scala signature; nothing here needs the kb
    if json {
        obj(vec![
            ("bundle", JVal::str(&b.label())),
            (
                "concepts",
                JVal::Arr(b.concepts.iter().map(concept_json).collect()),
            ),
        ])
        .document()
    } else {
        let mut sb = String::new();
        sb.push_str(&format!(
            "{} — {}\n",
            b.label(),
            b.index.fm().title().unwrap_or_default()
        ));
        if let Some(d) = b.index.fm().description() {
            sb.push_str(&format!("{d}\n"));
        }
        sb.push('\n');
        let w = b
            .concepts
            .iter()
            .map(|c| c.bundle_path().chars().count())
            .chain([4])
            .max()
            .unwrap_or(4);
        for c in &b.concepts {
            let status = c
                .fm()
                .status()
                .map(|s| format!(" [{s}]"))
                .unwrap_or_default();
            sb.push_str(&format!(
                "{}  {}{status}\n",
                pad(&c.bundle_path(), w),
                c.fm().doc_type().as_deref().unwrap_or("?")
            ));
            sb.push_str(&format!(
                "{}  {}\n",
                " ".repeat(w),
                c.fm()
                    .description()
                    .as_deref()
                    .unwrap_or("(no description)")
            ));
        }
        sb.push_str(&format!("\n{} concept(s)", b.concepts.len()));
        if !b.sub_indexes.is_empty() {
            sb.push_str(&format!(", {} sub-index(es)", b.sub_indexes.len()));
        }
        sb.push('\n');
        sb
    }
}

/// The concept JSON shape shared by `list --bundle`, `show` and `search` —
/// field order matches the Scala `conceptJson`.
fn concept_json(c: &Doc) -> JVal {
    obj(vec![
        ("path", JVal::str(&c.bundle_path())),
        ("file", JVal::str(&paths::render(&c.file))),
        ("type", JVal::opt_str(c.fm().doc_type().as_deref())),
        ("title", JVal::opt_str(c.fm().title().as_deref())),
        (
            "description",
            JVal::opt_str(c.fm().description().as_deref()),
        ),
        ("status", JVal::opt_str(c.fm().status().as_deref())),
        ("tags", str_arr(&c.fm().tags())),
        (
            "sources",
            JVal::Arr(
                c.fm()
                    .sources()
                    .iter()
                    .map(|s| JVal::str(&s.resource))
                    .collect(),
            ),
        ),
    ])
}

// ------------------------------------------------------------------- show

/// Renders `kb show`: frontmatter fields, outbound links, outline, optional
/// body.
pub fn show(
    kb: &Kb,
    path: &str,
    bundle_hint: Option<&str>,
    include_body: bool,
    json: bool,
) -> String {
    match find_doc(kb, path, bundle_hint) {
        None if json => obj(vec![
            ("found", JVal::bool(false)),
            ("query", JVal::str(path)),
        ])
        .document(),
        None => format!("not found: {path}\n"),
        Some((b, d)) if json => {
            let JVal::Obj(mut fields) = concept_json(d) else {
                unreachable!("concept_json builds an object")
            };
            fields.push(("found".to_string(), JVal::bool(true)));
            fields.push(("bundle".to_string(), JVal::str(&b.label())));
            fields.push(("kind".to_string(), JVal::str(kind_str(d.kind))));
            fields.push((
                "links".to_string(),
                JVal::Arr(
                    d.links
                        .iter()
                        .map(|l| {
                            obj(vec![
                                ("dest", JVal::str(&l.dest)),
                                ("line", JVal::num(l.line)),
                                ("external", JVal::bool(l.is_external())),
                            ])
                        })
                        .collect(),
                ),
            ));
            if include_body {
                fields.push(("body".to_string(), JVal::str(&d.body)));
            }
            JVal::Obj(fields).document()
        }
        Some((b, d)) => {
            let mut sb = String::new();
            sb.push_str(&format!("{}{}\n", b.label(), d.bundle_path()));
            sb.push_str(&format!("file:        {}\n", paths::render(&d.file)));
            sb.push_str(&format!("kind:        {}\n", kind_str(d.kind)));
            if let Some(t) = d.fm().doc_type() {
                sb.push_str(&format!("type:        {t}\n"));
            }
            if let Some(t) = d.fm().title() {
                sb.push_str(&format!("title:       {t}\n"));
            }
            if let Some(t) = d.fm().description() {
                sb.push_str(&format!("description: {t}\n"));
            }
            if let Some(t) = d.fm().status() {
                sb.push_str(&format!("status:      {t}\n"));
            }
            if let Some(t) = d.fm().stale_after() {
                sb.push_str(&format!("stale_after: {t}\n"));
            }
            let tags = d.fm().tags();
            if !tags.is_empty() {
                sb.push_str(&format!("tags:        {}\n", tags.join(", ")));
            }
            let sources = d.fm().sources();
            if !sources.is_empty() {
                sb.push_str("sources:\n");
                for s in &sources {
                    let id = s.id.as_ref().map(|i| format!("{i}: ")).unwrap_or_default();
                    sb.push_str(&format!("  - {id}{}\n", s.resource));
                }
            }
            let outbound: Vec<_> = d
                .links
                .iter()
                .filter(|l| !l.is_external() && !l.is_anchor_only())
                .collect();
            if !outbound.is_empty() {
                sb.push_str(&format!("\noutbound links ({}):\n", outbound.len()));
                for l in &outbound {
                    sb.push_str(&format!("  {}\n", l.dest));
                }
            }
            let headings: Vec<&str> = d.body.lines().filter(|l| l.starts_with('#')).collect();
            if !headings.is_empty() {
                sb.push_str("\noutline:\n");
                for h in &headings {
                    sb.push_str(&format!("  {h}\n"));
                }
            }
            if include_body {
                sb.push_str(&format!("\n---\n{}\n", d.body));
            }
            sb
        }
    }
}

/// Resolves a `show` query. A `/`-prefixed path is bundle-relative (narrowed
/// by the hint); anything else is a path suffix matched across the whole kb,
/// as in the Scala `findDoc`.
fn find_doc<'a>(
    kb: &'a Kb,
    path: &str,
    bundle_hint: Option<&str>,
) -> Option<(&'a Bundle, &'a Doc)> {
    let candidates: Vec<&Bundle> = match bundle_hint.and_then(|h| kb.bundle(h)) {
        Some(b) => vec![b],
        None => kb.bundles.iter().collect(),
    };
    if path.starts_with('/') {
        candidates
            .iter()
            .find_map(|b| b.concept_at(path).map(|d| (*b, d)))
            .or_else(|| {
                candidates.iter().find_map(|b| {
                    b.all_docs()
                        .into_iter()
                        .find(|d| d.bundle_path() == path)
                        .map(|d| (*b, d))
                })
            })
    } else {
        let needle = path.strip_suffix('/').unwrap_or(path);
        kb.bundles
            .iter()
            .flat_map(|b| b.all_docs().into_iter().map(move |d| (b, d)))
            .find(|(_, d)| paths::render(&d.file).ends_with(needle) || d.rel.join("/") == needle)
    }
}

// ----------------------------------------------------------------- search

/// Renders the scan search: AND-combined filters, case-insensitive contains
/// over titles, descriptions, types, tags and paths, optional body search.
#[allow(clippy::too_many_arguments)]
pub fn search(
    kb: &Kb,
    query: Option<&str>,
    search_body: bool,
    type_filter: Option<&str>,
    tag_filters: &[String],
    status_filter: Option<&str>,
    bundle_filter: Option<&str>,
    json: bool,
) -> String {
    let scope: Vec<(&Bundle, &Doc)> = match bundle_filter.and_then(|l| kb.bundle(l)) {
        Some(b) => b.concepts.iter().map(|d| (b, d)).collect(),
        None => kb.concepts(),
    };
    let q = query.map(str::to_lowercase);

    let meta_hit = |d: &Doc| -> bool {
        q.as_deref().is_none_or(|needle| {
            [d.fm().title(), d.fm().description(), d.fm().doc_type()]
                .iter()
                .flatten()
                .any(|v| v.to_lowercase().contains(needle))
                || d.fm()
                    .tags()
                    .iter()
                    .any(|t| t.to_lowercase().contains(needle))
                || d.rel.join("/").to_lowercase().contains(needle)
        })
    };

    let body_hits = |d: &Doc| -> Vec<(usize, String)> {
        match q.as_deref().filter(|_| search_body) {
            None => Vec::new(),
            Some(needle) => d
                .body
                .lines()
                .enumerate()
                .filter(|(_, l)| l.to_lowercase().contains(needle))
                .map(|(i, l)| (i + 1, l.trim().to_string()))
                .collect(),
        }
    };

    let matched: Vec<(&Bundle, &Doc)> = scope
        .into_iter()
        .filter(|(_, d)| {
            type_filter.is_none_or(|t| d.fm().doc_type().is_some_and(|x| x.eq_ignore_ascii_case(t)))
                && status_filter
                    .is_none_or(|s| d.fm().status().is_some_and(|x| x.eq_ignore_ascii_case(s)))
                && tag_filters
                    .iter()
                    .all(|t| d.fm().tags().iter().any(|x| x.eq_ignore_ascii_case(t)))
                && (meta_hit(d) || !body_hits(d).is_empty())
        })
        .collect();

    if json {
        let results: Vec<JVal> = matched
            .iter()
            .map(|(b, d)| {
                let JVal::Obj(mut fields) = concept_json(d) else {
                    unreachable!("concept_json builds an object")
                };
                fields.push(("bundle".to_string(), JVal::str(&b.label())));
                fields.push((
                    "bodyHits".to_string(),
                    JVal::Arr(
                        body_hits(d)
                            .iter()
                            .map(|(n, l)| {
                                obj(vec![("line", JVal::num(*n)), ("text", JVal::str(l))])
                            })
                            .collect(),
                    ),
                ));
                JVal::Obj(fields)
            })
            .collect();
        obj(vec![
            ("matches", JVal::num(matched.len())),
            ("results", JVal::Arr(results)),
        ])
        .document()
    } else {
        let mut sb = String::new();
        for (b, d) in &matched {
            let status = d
                .fm()
                .status()
                .map(|s| format!(", {s}"))
                .unwrap_or_default();
            sb.push_str(&format!(
                "{}{}  [{}{status}]\n",
                b.label(),
                d.bundle_path(),
                d.fm().doc_type().as_deref().unwrap_or("?")
            ));
            sb.push_str(&format!(
                "  {} — {}\n",
                d.fm().title().unwrap_or_else(|| d.display_title()),
                d.fm().description().unwrap_or_default()
            ));
            let hits = body_hits(d);
            for (n, l) in hits.iter().take(3) {
                let clipped: String = l.chars().take(140).collect();
                sb.push_str(&format!("  {n}: {clipped}\n"));
            }
            if hits.len() > 3 {
                sb.push_str(&format!("  … {} more line(s)\n", hits.len() - 3));
            }
        }
        if matched.is_empty() {
            sb.push_str("no matches\n");
        } else {
            sb.push_str(&format!("\n{} match(es)\n", matched.len()));
        }
        sb
    }
}

// --------------------------------------------------------------- scaffold

/// Renders a scaffolding result: what was created and updated, plus notes.
pub fn scaffold(r: &ScaffoldResult, json: bool) -> String {
    if json {
        let rendered = |ps: &[std::path::PathBuf]| -> Vec<String> {
            ps.iter().map(|p| paths::render(p)).collect()
        };
        return obj(vec![
            ("created", str_arr(&rendered(&r.created))),
            ("updated", str_arr(&rendered(&r.updated))),
            ("notes", str_arr(&r.notes)),
        ])
        .document();
    }
    let mut sb = String::new();
    for p in &r.created {
        sb.push_str(&format!("created  {}\n", paths::render(p)));
    }
    for p in &r.updated {
        sb.push_str(&format!("updated  {}\n", paths::render(p)));
    }
    for n in &r.notes {
        sb.push_str(&format!("note     {n}\n"));
    }
    sb.push_str("\nnext: write the body, then run `kb check`\n");
    sb
}
