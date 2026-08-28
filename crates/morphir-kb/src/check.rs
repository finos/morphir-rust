//! Conformance and drift checks over a knowledge base. Ported from
//! `KbCheck.scala`, plus the check-command composition from `kb.scala`'s
//! `CheckCmd`.
//!
//! Two families:
//!
//!   - **structural** — does the KB obey OKF and the conventions in
//!     `kb/AGENTS.md`?
//!   - **provenance** — do the commit-pinned `sources` still line up with the
//!     reference checkouts under `.refs/`?
//!
//! Everything is offline. Provenance runs `git` against local checkouts; it
//! never reaches the network. `today` flows in as a parameter so results are
//! reproducible; the CLI supplies the clock.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::NaiveDate;
use regex::Regex;

use morphir_okf::model::{Bundle, Doc, DocKind, Finding, Kb, LinkRef, Severity, parse_index_entry};
use morphir_okf::paths;
use morphir_okf::profile::OkfProfile;
use morphir_okf::store::resolve_link;

use crate::error::Result;
use crate::render::JVal;
use crate::{decision, sync};

static PROFILE: LazyLock<OkfProfile> = LazyLock::new(OkfProfile::default);

static GITHUB_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https://github\.com/([^/]+)/([^/]+)/(?:blob|tree)/([0-9a-f]{7,40})/(.*)$")
        .expect("valid regex")
});

/// Options mirroring the Scala `CheckOpts` — everything `kb check` takes
/// beyond the shared `--kb`/`--json` pair.
#[derive(Debug, Clone, Default)]
pub struct CheckOpts {
    /// Reference checkout root for provenance checks; `None` means the
    /// convention `<repo>/.refs` beside the kb root.
    pub refs: Option<PathBuf>,
    /// Skip provenance checks against `.refs/`.
    pub no_provenance: bool,
    /// Include info-level findings in the text render.
    pub verbose: bool,
    /// Exit non-zero when warnings are present, not just errors.
    pub strict: bool,
    /// Report dangling links as warnings — OKF's stance that they mark
    /// not-yet-written knowledge.
    pub allow_dangling: bool,
    /// Write the report here instead of stdout (convention: under `.dev/`).
    pub out: Option<PathBuf>,
}

/// Everything the check command decides, surfaced as data so the CLI owns
/// process exit and printing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub findings: Vec<Finding>,
}

impl CheckReport {
    pub fn errors(&self) -> usize {
        count(&self.findings, Severity::Error)
    }

    pub fn warnings(&self) -> usize {
        count(&self.findings, Severity::Warn)
    }

    pub fn infos(&self) -> usize {
        count(&self.findings, Severity::Info)
    }

    /// The exit semantics of `kb check`: errors always fail; warnings fail
    /// only under `--strict`.
    pub fn should_fail(&self, strict: bool) -> bool {
        self.errors() > 0 || (strict && self.warnings() > 0)
    }
}

fn count(findings: &[Finding], severity: Severity) -> usize {
    findings.iter().filter(|f| f.severity == severity).count()
}

fn sort_findings(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        (a.severity, &a.path, a.line.unwrap_or(0)).cmp(&(b.severity, &b.path, b.line.unwrap_or(0)))
    });
}

// ---------------------------------------------------------------- top level

/// Runs every check owned by this module: structural per bundle, strays,
/// the decision register's findings ([`decision::findings`]), and — when a
/// refs root is given — provenance. Sorted by (severity, path, line).
///
/// Sync findings are *not* included here, matching the Scala split: the check
/// command composes them separately via [`sync::all_sync_findings`]; use
/// [`run_full`] for the whole composition.
pub fn run(
    kb: &Kb,
    refs_root: Option<&Path>,
    today: NaiveDate,
    allow_dangling: bool,
) -> Vec<Finding> {
    let mut out = Vec::new();
    for b in &kb.bundles {
        out.extend(bundle_findings(kb, b, today, allow_dangling));
    }
    for p in &kb.strays {
        out.push(Finding {
            severity: Severity::Error,
            check: "stray-markdown".to_string(),
            path: kb.rel(p),
            line: None,
            message: "markdown file under bundles/ that belongs to no bundle".to_string(),
            hint: Some(
                "a bundle root is a directory whose index.md carries okf_version; grouping directories use README.md"
                    .to_string(),
            ),
        });
    }
    out.extend(decision::findings(kb));
    if let Some(refs) = refs_root {
        for (_, d) in kb.concepts() {
            out.extend(source_findings(kb, d, refs));
        }
    }
    sort_findings(&mut out);
    out
}

/// The full `kb check` composition, exactly as the Scala `CheckCmd` wires it:
/// [`run`] plus [`sync::all_sync_findings`], re-sorted together.
///
/// The refs candidate is `opts.refs` or `<kb-root parent>/.refs`; provenance
/// and upstream comparison run only when it exists and `--no-provenance` is
/// not set.
pub fn run_full(kb: &Kb, opts: &CheckOpts, today: NaiveDate) -> Result<CheckReport> {
    let refs_candidate = opts
        .refs
        .clone()
        .unwrap_or_else(|| default_refs_root(&kb.root));
    let refs_present = !opts.no_provenance && refs_candidate.exists();
    let core = run(
        kb,
        refs_present.then_some(refs_candidate.as_path()),
        today,
        opts.allow_dangling,
    );
    let sync_findings = sync::all_sync_findings(kb, &refs_candidate, refs_present)?;
    let mut findings = core;
    findings.extend(sync_findings);
    sort_findings(&mut findings);
    Ok(CheckReport { findings })
}

/// `.refs/` sits beside `kb/`, which is the convention `kb check` follows for
/// provenance.
pub fn default_refs_root(kb_root: &Path) -> PathBuf {
    match kb_root.parent() {
        Some(p) => p.join(".refs"),
        None => PathBuf::from(".refs"),
    }
}

fn bundle_findings(kb: &Kb, b: &Bundle, today: NaiveDate, allow_dangling: bool) -> Vec<Finding> {
    let mut out = Vec::new();
    for d in b.all_docs() {
        out.extend(pure_doc_findings(kb, d, today));
        out.extend(link_findings(kb, d, allow_dangling));
    }
    out.extend(index_coverage(kb, b));
    out.extend(index_descriptions(kb, b));
    out.extend(duplicate_titles(kb, b));
    out.extend(readme_in_bundle(kb, b));
    out
}

// ------------------------------------------------------------- per document

/// Everything about a document that can be decided without touching the
/// filesystem.
pub fn pure_doc_findings(kb: &Kb, d: &Doc, today: NaiveDate) -> Vec<Finding> {
    let where_ = kb.rel(&d.file);
    let mut out = Vec::new();

    if let Some(msg) = &d.frontmatter_error {
        out.push(finding(
            Severity::Error,
            "frontmatter-invalid",
            &where_,
            Some(1),
            format!("YAML did not parse: {msg}"),
            None,
        ));
    }

    match d.kind {
        DocKind::SubIndex if d.has_frontmatter_block => out.push(finding(
            Severity::Error,
            "subindex-has-frontmatter",
            &where_,
            Some(1),
            "a non-root index.md must have no frontmatter".to_string(),
            Some(
                "only the bundle-root index.md carries frontmatter, and there only okf_version and friends"
                    .to_string(),
            ),
        )),
        DocKind::Concept if !d.has_frontmatter_block => out.push(finding(
            Severity::Error,
            "concept-no-frontmatter",
            &where_,
            Some(1),
            "concept document has no frontmatter block".to_string(),
            Some("every concept needs at least `type:`".to_string()),
        )),
        _ => {}
    }

    if d.is_concept() && d.has_frontmatter_block && !d.vendored {
        let fm = d.fm();
        if fm.doc_type().is_none() {
            out.push(finding(
                Severity::Error,
                "concept-missing-type",
                &where_,
                Some(1),
                "concept has no `type` — the one universally required OKF field".to_string(),
                None,
            ));
        }
        if fm.title().is_none() {
            out.push(finding(
                Severity::Warn,
                "concept-missing-title",
                &where_,
                Some(1),
                "concept has no `title`".to_string(),
                None,
            ));
        }
        if fm.description().is_none() {
            out.push(finding(
                Severity::Warn,
                "concept-missing-description",
                &where_,
                Some(1),
                "concept has no `description` — indexes and search snippets read it".to_string(),
                None,
            ));
        }
        if let Some(s) = fm.status().filter(|s| !PROFILE.is_known_status(s)) {
            let known: Vec<&str> = PROFILE.statuses.iter().map(String::as_str).collect();
            out.push(finding(
                Severity::Warn,
                "status-unknown",
                &where_,
                Some(1),
                format!("status `{s}` is not one of {}", known.join(", ")),
                None,
            ));
        }
        if let Some(dt) = fm
            .stale_after()
            .and_then(|s| parse_date(&s))
            .filter(|dt| *dt < today)
        {
            out.push(finding(
                Severity::Warn,
                "stale-after-passed",
                &where_,
                Some(1),
                format!("stale_after {dt} has passed — content is due for review"),
                None,
            ));
        }
        out.extend(unknown_keys(&where_, d));
    }

    // A mirrored document's frontmatter belongs to upstream. Demanding OKF's
    // `status` vocabulary of it, or reporting every `sidebar_position` as an
    // unknown key, would bury the findings that are actually ours to fix.
    // `type` still has to be there — `kb sync pull` injects it, so its absence
    // means the injection failed.
    if d.vendored && d.has_frontmatter_block && d.fm().doc_type().is_none() {
        out.push(finding(
            Severity::Error,
            "concept-missing-type",
            &where_,
            Some(1),
            "mirrored concept has no `type` — the kb frontmatter block is missing or damaged"
                .to_string(),
            Some("re-run `kb sync pull` for this bundle".to_string()),
        ));
    }

    out.extend(escaping_links(&where_, d));
    out.extend(figure_findings(&where_, d));
    out
}

fn finding(
    severity: Severity,
    check: &str,
    path: &str,
    line: Option<usize>,
    message: String,
    hint: Option<String>,
) -> Finding {
    Finding {
        severity,
        check: check.to_string(),
        path: path.to_string(),
        line,
        message,
        hint,
    }
}

fn unknown_keys(where_: &str, d: &Doc) -> Vec<Finding> {
    let mut keys: Vec<&str> = d
        .fm()
        .keys()
        .filter(|k| !PROFILE.is_recognized(k))
        .collect();
    keys.sort_unstable();
    keys.into_iter()
        .map(|k| {
            finding(
                Severity::Info,
                "frontmatter-unknown-key",
                where_,
                Some(1),
                format!("frontmatter key `{k}` is recognized by neither OKF v0.2 nor this tooling"),
                None,
            )
        })
        .collect()
}

fn parse_date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

// ----------------------------------------------------------------- figures

/// Figures — mermaid fences and standalone images — carry numbered captions:
/// a paragraph directly after the figure starting `**Figure N:**`, numbered
/// 1..N in document order. Only this flat per-document scheme is checked;
/// a section-aware caption (`Figure 2.1`) reports as out-of-sequence rather
/// than passing unvalidated.
static FIGURE_CAPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\*{0,2}[Ff]igure\s+(\d+)(?:\s*[:.]\s*\*{0,2}|\*{0,2}\s*[:.]).*$")
        .expect("valid regex")
});

fn figure_findings(where_: &str, d: &Doc) -> Vec<Finding> {
    if !d.is_concept() || d.vendored {
        return Vec::new();
    }
    let lines: Vec<&str> = d.body.lines().collect();
    let offset = d.frontmatter_lines;
    let mut out = Vec::new();
    let mut expected: usize = 1;

    let check_caption = |out: &mut Vec<Finding>,
                         expected: &mut usize,
                         figure_end_idx: usize,
                         label: &str| {
        let mut j = figure_end_idx + 1;
        while j < lines.len() && lines[j].trim().is_empty() {
            j += 1;
        }
        match lines
                .get(j)
                .and_then(|l| FIGURE_CAPTION.captures(l.trim()))
            {
                Some(caps) => {
                    let n: usize = caps[1].parse().unwrap_or(0);
                    if n != *expected {
                        out.push(finding(
                            Severity::Warn,
                            "figure-number-out-of-sequence",
                            where_,
                            Some(j + 1 + offset),
                            format!(
                                "{label} caption is numbered Figure {n} but this is figure {expected} of the document"
                            ),
                            Some(
                                "number figures 1..N in document order; section-aware schemes are not yet supported"
                                    .to_string(),
                            ),
                        ));
                    }
                }
                None => out.push(finding(
                    Severity::Warn,
                    "figure-caption-missing",
                    where_,
                    Some(figure_end_idx + 1 + offset),
                    format!("{label} has no numbered caption"),
                    Some(format!(
                        "follow the figure with a caption paragraph: `**Figure {expected}:** <what to notice>`"
                    )),
                )),
            }
        *expected += 1;
    };

    let mut open_fence: Option<String> = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        match &open_fence {
            Some(info) => {
                if t.starts_with("```") {
                    if info == "mermaid" {
                        check_caption(&mut out, &mut expected, i, "mermaid diagram");
                    }
                    open_fence = None;
                }
            }
            None => {
                if let Some(rest) = t.strip_prefix("```") {
                    let info: String = rest
                        .trim()
                        .chars()
                        .take_while(|c| !c.is_whitespace())
                        .collect();
                    open_fence = Some(info.to_lowercase());
                } else if t.starts_with("![") && t.ends_with(')') {
                    check_caption(&mut out, &mut expected, i, "image");
                }
            }
        }
    }
    out
}

// ------------------------------------------------------------------- links

/// A bundle-relative link starts at the bundle root, so a `..` segment in one
/// can only take it outside the bundle. This needs its own check because such
/// a link usually still *resolves* — the filesystem collapses the `..` when
/// existence is tested — so `link-broken` stays silent while the link means
/// something other than it says.
fn escaping_links(where_: &str, d: &Doc) -> Vec<Finding> {
    d.links
        .iter()
        .filter(|l| l.is_bundle_relative() && escapes_bundle(&l.dest))
        .map(|l| {
            finding(
                Severity::Error,
                "link-escapes-bundle",
                where_,
                Some(l.line),
                format!("bundle-relative link climbs above the bundle root: {}", l.dest),
                Some(
                    "bundle-relative paths start at the bundle root; use a normal relative path to reach another bundle"
                        .to_string(),
                ),
            )
        })
        .collect()
}

fn escapes_bundle(dest: &str) -> bool {
    let target = dest.split('#').next().unwrap_or("");
    let stripped = target.strip_prefix('/').unwrap_or(target);
    let mut depth: i64 = 0;
    let mut escaped = false;
    for seg in stripped.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if depth == 0 {
                    escaped = true;
                } else {
                    depth -= 1;
                }
            }
            _ => depth += 1,
        }
    }
    escaped
}

/// OKF says *consumers* must tolerate dangling links as not-yet-written
/// knowledge. This is a producer-side linter, where a dangling link is nearly
/// always a typo — so it is an error by default. `allow_dangling` restores
/// OKF's lenient stance.
fn link_findings(kb: &Kb, d: &Doc, allow_dangling: bool) -> Vec<Finding> {
    // A mirrored document's links are written for the *upstream site*, where
    // `../migration-guide/` and `/schemas/x.yaml` are routes rather than paths
    // on disk. Only relative links that name an actual file are checked; the
    // rest are left to whatever link checker upstream runs.
    let resolve_one = |link: &LinkRef| -> Option<PathBuf> {
        if d.vendored {
            let target = link.dest.split('#').next().unwrap_or("");
            let names_a_file = target
                .split('/')
                .rfind(|s| !s.is_empty())
                .is_some_and(|s| s.contains('.') && s != "." && s != "..");
            if names_a_file && !link.is_bundle_relative() {
                resolve_link(d, link)
            } else {
                None
            }
        } else {
            resolve_link(d, link)
        }
    };

    let severity = if d.vendored || allow_dangling {
        Severity::Warn
    } else {
        Severity::Error
    };
    let rule = if d.vendored {
        "link-broken-upstream"
    } else {
        "link-broken"
    };

    d.links
        .iter()
        .filter_map(|l| resolve_one(l).map(|t| (l, t)))
        .filter(|(_, target)| !target.exists())
        .map(|(link, _)| {
            let hint = if d.vendored {
                Some("upstream's own link; fix it there and export, or leave it".to_string())
            } else if link.is_bundle_relative() {
                Some("bundle-relative paths start at the bundle root, not the kb root".to_string())
            } else {
                None
            };
            finding(
                severity,
                rule,
                &kb.rel(&d.file),
                Some(link.line),
                format!("link target does not exist: {}", link.dest),
                hint,
            )
        })
        .collect()
}

// ----------------------------------------------------------------- indexes

/// Every concept should be reachable from an index in its own bundle.
fn index_coverage(kb: &Kb, b: &Bundle) -> Vec<Finding> {
    let linked: std::collections::HashSet<String> = b
        .all_indexes()
        .iter()
        .flat_map(|idx| {
            idx.links
                .iter()
                .filter(|l| l.is_bundle_relative())
                .map(|l| l.dest.split('#').next().unwrap_or("").to_string())
        })
        .collect();
    b.concepts
        .iter()
        .filter(|c| !linked.contains(&c.bundle_path()))
        .map(|c| {
            finding(
                Severity::Warn,
                "concept-not-indexed",
                &kb.rel(&c.file),
                None,
                format!("concept is not linked from any index in {}", b.label()),
                Some(format!(
                    "add `* [{}]({}) - {}` to an index",
                    c.display_title(),
                    c.bundle_path(),
                    c.fm().description().unwrap_or_else(|| "…".to_string())
                )),
            )
        })
        .collect()
}

/// Index bullets mirror the target's `description`; drift between them is a
/// real inconsistency.
fn index_descriptions(kb: &Kb, b: &Bundle) -> Vec<Finding> {
    let mut out = Vec::new();
    for idx in b.all_indexes() {
        for (i, line) in idx.body.lines().enumerate() {
            let Some(entry) = parse_index_entry(line) else {
                continue;
            };
            if !entry.dest.starts_with('/') {
                continue;
            }
            let text = entry
                .description
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty());
            let target = b.concept_at(entry.dest.split('#').next().unwrap_or(""));
            if let (Some(t), Some(dsc)) = (
                text,
                target
                    .and_then(|c| c.fm().description())
                    .map(|d| d.trim().to_string()),
            ) && normalize(t) != normalize(&dsc)
            {
                out.push(finding(
                    Severity::Warn,
                    "index-description-drift",
                    &kb.rel(&idx.file),
                    Some(i + 1),
                    format!(
                        "index entry text differs from the concept's description ({})",
                        entry.dest
                    ),
                    Some(format!("concept says: {dsc}")),
                ));
            }
        }
    }
    out
}

/// The drift comparison is lenient about case, whitespace and a trailing
/// period — not about wording.
fn normalize(s: &str) -> String {
    static WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").expect("valid regex"));
    let trimmed = s.trim();
    let stripped = trimmed.strip_suffix('.').unwrap_or(trimmed);
    WS.replace_all(stripped, " ").to_lowercase()
}

/// Mirrored documents are excluded: upstream reuses `index.md` and
/// `whats-new.md` across version directories, and that is upstream's naming
/// to answer for, not a defect in this bundle.
fn duplicate_titles(kb: &Kb, b: &Bundle) -> Vec<Finding> {
    let mut by_title: std::collections::BTreeMap<String, Vec<&Doc>> =
        std::collections::BTreeMap::new();
    for d in b.authored_concepts() {
        by_title.entry(d.display_title()).or_default().push(d);
    }
    by_title
        .into_iter()
        .filter(|(_, docs)| docs.len() > 1)
        .flat_map(|(title, docs)| {
            let n = docs.len();
            let label = b.label();
            docs.into_iter()
                .map(move |d| {
                    finding(
                        Severity::Warn,
                        "duplicate-title",
                        &kb.rel(&d.file),
                        Some(1),
                        format!("title `{title}` is used by {n} concepts in {label}"),
                        None,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn readme_in_bundle(kb: &Kb, b: &Bundle) -> Vec<Finding> {
    b.authored_concepts()
        .into_iter()
        .filter(|d| d.name().eq_ignore_ascii_case("README.md"))
        .map(|d| {
            finding(
                Severity::Error,
                "readme-in-bundle",
                &kb.rel(&d.file),
                None,
                "README.md inside a bundle is parsed as a concept document".to_string(),
                Some(
                    "put bundle orientation in index.md; README.md belongs to grouping directories only"
                        .to_string(),
                ),
            )
        })
        .collect()
}

// -------------------------------------------------------------- provenance

/// Current HEAD of a local checkout, or `None` when it is not a git
/// repository. Validated by shape — exactly one 40-character hex SHA —
/// rather than by exit code.
pub fn git_head(repo: &Path) -> Option<String> {
    sync::git_head(repo)
}

/// Compares each commit-pinned GitHub source against the matching checkout
/// under `.refs/<org>/<repo>`.
fn source_findings(kb: &Kb, d: &Doc, refs_root: &Path) -> Vec<Finding> {
    let where_ = kb.rel(&d.file);
    let refs_name = refs_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".refs".to_string());
    let mut out = Vec::new();
    for src in d.fm().sources() {
        let Some(caps) = GITHUB_URL.captures(&src.resource) else {
            continue;
        };
        let (org, repo, sha, path) = (&caps[1], &caps[2], &caps[3], &caps[4]);
        let checkout = refs_root.join(org).join(repo);
        if !checkout.exists() {
            out.push(finding(
                Severity::Info,
                "source-ref-missing",
                &where_,
                None,
                format!("no reference checkout for {org}/{repo} under {refs_name}/"),
                Some(
                    "add one so provenance can be verified: /squire reference repo add".to_string(),
                ),
            ));
            continue;
        }
        let clean_path = path.split('#').next().unwrap_or("");
        let head = git_head(&checkout);
        let path_ok = clean_path.is_empty() || checkout.join(clean_path).exists();
        if let Some(h) = head.filter(|h| !h.starts_with(sha) && !sha.starts_with(&h[..sha.len()])) {
            out.push(finding(
                Severity::Warn,
                "source-commit-drift",
                &where_,
                None,
                format!(
                    "source pinned at {} but {refs_name}/{org}/{repo} is at {}",
                    &sha[..8.min(sha.len())],
                    &h[..8]
                ),
                Some(
                    "re-read the source and update the concept, or leave the pin and accept it is historical"
                        .to_string(),
                ),
            ));
        }
        if !path_ok {
            out.push(finding(
                Severity::Warn,
                "source-path-missing",
                &where_,
                None,
                format!("source path no longer present at the checkout's HEAD: {clean_path}"),
                Some(
                    "the file moved or was deleted upstream; the pinned URL still resolves on GitHub"
                        .to_string(),
                ),
            ));
        }
    }
    out
}

// --------------------------------------------------------------- rendering

/// Text render: one block per finding (severity, check, `path:line`, message,
/// hint), then the `N error(s), M warning(s), K info` summary. Info findings
/// are hidden unless `verbose`, but still counted.
pub fn render_text(findings: &[Finding], verbose: bool) -> String {
    let shown: Vec<&Finding> = findings
        .iter()
        .filter(|f| verbose || f.severity != Severity::Info)
        .collect();
    let mut sb = String::new();
    for f in &shown {
        sb.push_str(&format!(
            "{:<5}  {:<26}  {}\n",
            f.severity.label(),
            f.check,
            f.location()
        ));
        sb.push_str(&format!("       {}\n", f.message));
        if let Some(h) = &f.hint {
            sb.push_str(&format!("       hint: {h}\n"));
        }
    }
    let e = count(findings, Severity::Error);
    let w = count(findings, Severity::Warn);
    let i = count(findings, Severity::Info);
    if shown.is_empty() {
        sb.push_str("no findings\n");
    }
    sb.push_str(&format!("\n{e} error(s), {w} warning(s), {i} info"));
    if !verbose && i > 0 {
        sb.push_str(" (hidden; pass --verbose)");
    }
    sb.push('\n');
    sb
}

/// JSON render matching the Scala shape byte for byte:
/// `{errors, warnings, infos, findings[{severity, check, path, line, message, hint}]}`.
pub fn render_json(findings: &[Finding]) -> String {
    let items: Vec<JVal> = findings
        .iter()
        .map(|f| {
            JVal::Obj(vec![
                ("severity".to_string(), JVal::str(f.severity.label())),
                ("check".to_string(), JVal::str(&f.check)),
                ("path".to_string(), JVal::str(&f.path)),
                (
                    "line".to_string(),
                    match f.line {
                        Some(l) => JVal::num(l),
                        None => JVal::null(),
                    },
                ),
                ("message".to_string(), JVal::str(&f.message)),
                ("hint".to_string(), JVal::opt_str(f.hint.as_deref())),
            ])
        })
        .collect();
    JVal::Obj(vec![
        (
            "errors".to_string(),
            JVal::num(count(findings, Severity::Error)),
        ),
        (
            "warnings".to_string(),
            JVal::num(count(findings, Severity::Warn)),
        ),
        (
            "infos".to_string(),
            JVal::num(count(findings, Severity::Info)),
        ),
        ("findings".to_string(), JVal::Arr(items)),
    ])
    .document()
}

/// The `--out` report writer: writes the rendered report to `out` (creating
/// parent directories) and returns the confirmation line the CLI prints in
/// place of the report — `wrote <path>`.
pub fn write_report(text: &str, out: &Path) -> Result<String> {
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(out, text)?;
    Ok(format!("wrote {}", paths::render(out)))
}
