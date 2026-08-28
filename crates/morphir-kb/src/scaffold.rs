//! Creating bundles and concepts, and keeping indexes and logs in step.
//!
//! Ported from `KbScaffold.scala` (plus `KbCli.parseSource` in `kb.scala`).
//! Scaffolding writes the frontmatter skeleton and wires the concept into its
//! index and log. It deliberately does not write prose — that is the agent's
//! job, and a stub that reads as finished is worse than an obvious stub.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use morphir_okf::model::{Bundle, SourceRef};
use morphir_okf::paths;

use crate::error::{Error, Result};
use crate::util::{slugify, yaml_str};

/// What a scaffolding operation touched, for the CLI to report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScaffoldResult {
    pub created: Vec<PathBuf>,
    pub updated: Vec<PathBuf>,
    pub notes: Vec<String>,
}

/// Joins `rel_path` onto `base` one non-empty segment at a time, mirroring the
/// Scala `descend` helper so `a//b/` and `a/b` land in the same place.
fn descend(base: &Path, rel_path: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for seg in rel_path.split('/').filter(|s| !s.is_empty()) {
        out.push(seg);
    }
    out
}

/// Writes `text` to `file`, creating parent directories as needed (the Scala
/// implementation's `Path.write` does the same implicitly).
fn write_creating_dirs(file: &Path, text: &str) -> Result<()> {
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(file, text)?;
    Ok(())
}

// ------------------------------------------------------------- new bundle

/// Scaffolds a new bundle directory with its `index.md` and `log.md`.
pub fn new_bundle(
    kb_root: &Path,
    name: &str,
    group: Option<&str>,
    title: &str,
    description: &str,
    okf_version: &str,
    today: NaiveDate,
) -> Result<ScaffoldResult> {
    let slug = slugify(name);
    let bundles = kb_root.join("bundles");
    let group_dir = group.map(|g| descend(&bundles, g));
    let dir = match &group_dir {
        Some(g) => g.join(&slug),
        None => bundles.join(&slug),
    };
    if dir.exists() {
        return Err(Error::msg(format!(
            "{} already exists",
            paths::render(&dir)
        )));
    }
    let index = dir.join("index.md");
    let log = dir.join("log.md");
    write_creating_dirs(&index, &index_template(okf_version, title, description))?;
    write_creating_dirs(&log, &log_template(today))?;
    let mut notes = Vec::new();
    if let Some(g) = &group_dir
        && !g.join("README.md").exists()
    {
        notes.push(format!(
            "grouping directory has no README.md yet: {}",
            paths::render(g)
        ));
    }
    notes.push(format!(
        "add the bundle to the Bundles table in {}",
        paths::render(&kb_root.join("README.md"))
    ));
    Ok(ScaffoldResult {
        created: vec![index, log],
        updated: Vec::new(),
        notes,
    })
}

fn index_template(okf_version: &str, title: &str, description: &str) -> String {
    format!(
        "---\nokf_version: \"{okf_version}\"\ntitle: {}\ndescription: {}\n---\n\n# {title}\n\n{description}\n\n## Orientation\n\n",
        yaml_str(title),
        yaml_str(description)
    )
}

fn log_template(today: NaiveDate) -> String {
    format!("# Log\n\n## {today}\n\n* **Creation**: Bundle created.\n")
}

// ------------------------------------------------------------ add concept

/// Scaffolds a concept file inside `bundle` and wires it into the nearest
/// index and the bundle log.
#[allow(clippy::too_many_arguments)]
pub fn add_concept(
    bundle: &Bundle,
    rel_path: &str,
    concept_type: &str,
    title: &str,
    description: &str,
    tags: &[String],
    status: Option<&str>,
    sources: &[SourceRef],
    section: &str,
    generated_by: Option<&str>,
    today: NaiveDate,
) -> Result<ScaffoldResult> {
    let raw = if rel_path.ends_with(".md") {
        rel_path.to_string()
    } else {
        format!("{rel_path}.md")
    };
    let rel_segs: Vec<&str> = raw.split('/').filter(|s| !s.is_empty()).collect();
    let leaf = rel_segs.last().copied().unwrap_or("");
    // A concept path names a location *inside* the bundle. Without this,
    // `--path ../escaped.md` writes outside it while still adding a
    // bundle-relative index entry pointing at nothing.
    if raw.starts_with('/') || rel_segs.iter().any(|s| *s == ".." || *s == ".") {
        return Err(Error::msg(format!(
            "`{rel_path}` must stay inside the bundle — no leading /, `.` or `..` segments"
        )));
    }
    if rel_segs.is_empty() {
        return Err(Error::msg("give a path within the bundle"));
    }
    if leaf == "index.md" || leaf == "log.md" {
        return Err(Error::msg(format!("{leaf} is a reserved OKF filename")));
    }
    let mut target = bundle.root.clone();
    for seg in &rel_segs {
        target.push(seg);
    }
    if target.exists() {
        return Err(Error::msg(format!(
            "{} already exists",
            paths::render(&target)
        )));
    }
    let bundle_path = format!("/{}", rel_segs.join("/"));
    // A concept in a subdirectory belongs in that subdirectory's index when
    // one exists.
    let idx_doc = bundle
        .sub_indexes
        .iter()
        .find(|i| {
            rel_segs.len() > 1
                && i.rel.len() == rel_segs.len()
                && i.rel[..i.rel.len() - 1]
                    .iter()
                    .map(String::as_str)
                    .eq(rel_segs[..rel_segs.len() - 1].iter().copied())
        })
        .unwrap_or(&bundle.index);
    write_creating_dirs(
        &target,
        &concept_template(
            concept_type,
            title,
            description,
            tags,
            status,
            sources,
            generated_by,
            today,
        ),
    )?;
    insert_index_entry(&idx_doc.file, section, title, &bundle_path, description)?;
    if let Some(log) = &bundle.log {
        append_log_entry(
            &log.file,
            today,
            &format!("**Creation**: Added [{title}]({bundle_path})."),
        )?;
    }
    let mut updated = vec![idx_doc.file.clone()];
    updated.extend(bundle.log.iter().map(|l| l.file.clone()));
    let notes = if bundle.log.is_none() {
        vec!["bundle has no log.md — creation was not recorded".to_string()]
    } else {
        Vec::new()
    };
    Ok(ScaffoldResult {
        created: vec![target],
        updated,
        notes,
    })
}

#[allow(clippy::too_many_arguments)]
fn concept_template(
    concept_type: &str,
    title: &str,
    description: &str,
    tags: &[String],
    status: Option<&str>,
    sources: &[SourceRef],
    generated_by: Option<&str>,
    today: NaiveDate,
) -> String {
    let mut sb = String::new();
    sb.push_str("---\n");
    sb.push_str(&format!("type: {}\n", yaml_str(concept_type)));
    sb.push_str(&format!("title: {}\n", yaml_str(title)));
    sb.push_str(&format!("description: {}\n", yaml_str(description)));
    if !tags.is_empty() {
        let slugged: Vec<String> = tags.iter().map(|t| slugify(t)).collect();
        sb.push_str(&format!("tags: [{}]\n", slugged.join(", ")));
    }
    if let Some(s) = status {
        sb.push_str(&format!("status: {s}\n"));
    }
    if !sources.is_empty() {
        sb.push_str("sources:\n");
        for src in sources {
            sb.push_str("  - ");
            if let Some(id) = &src.id {
                sb.push_str(&format!("id: {id}\n    "));
            }
            sb.push_str(&format!("resource: {}\n", src.resource));
            if let Some(t) = &src.title {
                sb.push_str(&format!("    title: {}\n", yaml_str(t)));
            }
        }
    }
    if let Some(by) = generated_by {
        sb.push_str("generated:\n");
        sb.push_str(&format!("  by: {by}\n"));
        sb.push_str(&format!("  at: {today}T00:00:00Z\n"));
    }
    sb.push_str("---\n\n");
    sb.push_str(&format!("# {title}\n\n"));
    sb.push_str(&format!("{description}\n\n"));
    sb.push_str("<!-- TODO: write the concept body. Delete this comment when done. -->\n");
    sb
}

/// Parses a `--source` argument: `id=URL`, `id=URL=Title`, or a bare URL.
/// The URL must start with `http` for the `id=` forms to be recognized.
pub fn parse_source(s: &str) -> SourceRef {
    let parts: Vec<&str> = s.splitn(3, '=').collect();
    match parts.as_slice() {
        [id, url, title] if url.starts_with("http") => SourceRef {
            id: Some((*id).to_string()),
            resource: (*url).to_string(),
            title: Some((*title).to_string()),
        },
        [id, url] if url.starts_with("http") => SourceRef {
            id: Some((*id).to_string()),
            resource: (*url).to_string(),
            title: None,
        },
        _ => SourceRef {
            id: None,
            resource: s.to_string(),
            title: None,
        },
    }
}

// ------------------------------------------------------------ index & log

/// Adds `* [title](path) - description` under `## section`, creating the
/// section at the end of the file when it is absent.
pub fn insert_index_entry(
    index: &Path,
    section: &str,
    title: &str,
    bundle_path: &str,
    description: &str,
) -> Result<()> {
    let text = fs::read_to_string(index)?;
    let entry = format!("* [{title}]({bundle_path}) - {}", description.trim());
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.lines().collect();
    let head = format!("## {section}");
    let head_idx = lines
        .iter()
        .position(|l| l.trim().to_lowercase() == head.to_lowercase());
    let updated: Vec<&str> = match head_idx {
        None => {
            let keep = lines.len() - trailing_blank_count(&lines);
            let mut out: Vec<&str> = lines[..keep].to_vec();
            out.extend(["", head.as_str(), "", entry.as_str()]);
            out
        }
        Some(hi) => {
            // Append after the last bullet of that section, before the next
            // heading.
            let next_head = lines[hi + 1..]
                .iter()
                .position(|l| l.starts_with("## "))
                .map(|n| hi + 1 + n)
                .unwrap_or(lines.len());
            let body = &lines[hi + 1..next_head];
            let last_bullet = body
                .iter()
                .rposition(|l| l.trim_start().starts_with('*') || l.trim_start().starts_with('-'));
            // With no bullets yet, land at the end of the section so the
            // blank line after the heading survives.
            let at = match last_bullet {
                None => next_head,
                Some(b) => hi + 1 + b + 1,
            };
            let mut out: Vec<&str> = lines[..at].to_vec();
            out.push(entry.as_str());
            out.extend(&lines[at..]);
            out
        }
    };
    fs::write(index, format!("{}\n", updated.join("\n").trim_end()))?;
    Ok(())
}

fn trailing_blank_count(lines: &[&str]) -> usize {
    lines
        .iter()
        .rev()
        .take_while(|l| l.trim().is_empty())
        .count()
}

/// Adds a bullet under `## <today>`, creating that date section at the top
/// (newest first) when it is absent.
pub fn append_log_entry(log: &Path, today: NaiveDate, entry: &str) -> Result<()> {
    let text = fs::read_to_string(log)?;
    let normalized = text.replace("\r\n", "\n");
    let lines: Vec<&str> = normalized.lines().collect();
    let bullet = format!("* {entry}");
    let date_head = format!("## {today}");
    let updated: Vec<&str> = match lines.iter().position(|l| l.trim() == date_head) {
        Some(idx) => {
            let next_head = lines[idx + 1..]
                .iter()
                .position(|l| l.starts_with("## "))
                .map(|n| idx + 1 + n)
                .unwrap_or(lines.len());
            let last_bullet = lines[idx + 1..next_head]
                .iter()
                .rposition(|l| l.trim_start().starts_with('*'));
            let at = match last_bullet {
                None => next_head,
                Some(b) => idx + 1 + b + 1,
            };
            let mut out: Vec<&str> = lines[..at].to_vec();
            out.push(bullet.as_str());
            out.extend(&lines[at..]);
            out
        }
        None => {
            let at = lines
                .iter()
                .position(|l| l.starts_with("## "))
                .unwrap_or(lines.len());
            let keep = at
                - lines[..at]
                    .iter()
                    .rev()
                    .take_while(|l| l.trim().is_empty())
                    .count();
            let mut out: Vec<&str> = lines[..keep].to_vec();
            out.extend(["", date_head.as_str(), "", bullet.as_str(), ""]);
            out.extend(&lines[at..]);
            out
        }
    };
    fs::write(log, format!("{}\n", updated.join("\n").trim_end()))?;
    Ok(())
}
