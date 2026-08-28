//! Ports of `KbCheckSpec` and `KbLinkSpec` from `KbTests.scala`, plus
//! coverage for the rest of the check catalogue: structural checks, link
//! checks with the vendored relaxations, index discipline, lifecycle,
//! figures, provenance against a real git checkout, the finding sort order,
//! and the text/JSON renders.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::NaiveDate;
use morphir_kb::check::{self, CheckOpts};
use morphir_kb::{refresh, scaffold, sync};
use morphir_okf::model::{Finding, Kb, Severity, SourceRef};
use morphir_okf::paths;
use tempfile::TempDir;

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 7, 28).unwrap()
}

/// A minimal knowledge base: one ordinary bundle.
fn fixture() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let kb_root = tmp.path().join("kb");
    scaffold::new_bundle(
        &kb_root,
        "demo",
        None,
        "Demo",
        "A scratch bundle.",
        "0.2",
        today(),
    )
    .unwrap();
    (tmp, kb_root)
}

fn load(kb_root: &Path) -> Kb {
    morphir_okf::store::load(kb_root).unwrap()
}

fn add_concept(kb_root: &Path, path: &str, title: &str, description: &str) {
    let kb = load(kb_root);
    let b = kb.bundle("demo").unwrap();
    scaffold::add_concept(
        b,
        path,
        "Concept",
        title,
        description,
        &[],
        None,
        &[],
        "Orientation",
        None,
        today(),
    )
    .unwrap();
}

fn run(kb_root: &Path) -> Vec<Finding> {
    check::run(&load(kb_root), None, today(), false)
}

fn checks(findings: &[Finding]) -> Vec<&str> {
    findings.iter().map(|f| f.check.as_str()).collect()
}

fn has(findings: &[Finding], check: &str) -> bool {
    findings.iter().any(|f| f.check == check)
}

fn demo_root(kb_root: &Path) -> PathBuf {
    kb_root.join("bundles").join("demo")
}

// -------------------------------------------------------------------- links

#[test]
fn broken_link_is_an_error_and_downgrades_under_allow_dangling() {
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "a.md", "A", "First.");
    let a = demo_root(&kb_root).join("a.md");
    let text = fs::read_to_string(&a).unwrap();
    fs::write(&a, format!("{text}\nSee [missing](/gone.md).\n")).unwrap();

    let kb = load(&kb_root);
    let strict = check::run(&kb, None, today(), false);
    let lenient = check::run(&kb, None, today(), true);
    assert!(
        strict
            .iter()
            .any(|f| f.check == "link-broken" && f.severity == Severity::Error),
        "got {:?}",
        checks(&strict)
    );
    assert!(
        lenient
            .iter()
            .any(|f| f.check == "link-broken" && f.severity == Severity::Warn),
        "downgraded, not suppressed"
    );
}

#[test]
fn a_bundle_relative_link_climbing_above_the_bundle_root_is_an_error() {
    // `/../demo/a.md` resolves on disk — bundleRoot + "../demo/a.md" is the
    // same file — so `link-broken` stays silent while the link means
    // something other than it says.
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "a.md", "A", "First.");
    let a = demo_root(&kb_root).join("a.md");
    let text = fs::read_to_string(&a).unwrap();
    fs::write(&a, format!("{text}\nSee [sideways](/../demo/a.md).\n")).unwrap();

    let findings = run(&kb_root);
    assert!(
        findings
            .iter()
            .any(|f| f.check == "link-escapes-bundle" && f.severity == Severity::Error),
        "got {:?}",
        checks(&findings)
    );
}

#[test]
fn an_ordinary_relative_link_resolves_against_the_containing_directory() {
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "a.md", "A", "First.");
    add_concept(&kb_root, "sub/b.md", "B", "Second.");
    let b = demo_root(&kb_root).join("sub").join("b.md");
    let text = fs::read_to_string(&b).unwrap();
    fs::write(&b, format!("{text}\nSee [a](../a.md).\n")).unwrap();

    let findings = run(&kb_root);
    let broken: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.check == "link-broken" && f.path.contains("b.md"))
        .collect();
    assert!(
        broken.is_empty(),
        "../a.md exists and should resolve, got {:?}",
        broken.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
}

// ----------------------------------------------------------------- concepts

#[test]
fn missing_type_and_unindexed_concept_are_reported() {
    let (_tmp, kb_root) = fixture();
    fs::write(
        demo_root(&kb_root).join("orphan.md"),
        "---\ntitle: Orphan\n---\n\nNo type, nobody links to it.\n",
    )
    .unwrap();
    let findings = run(&kb_root);
    assert!(has(&findings, "concept-missing-type"));
    assert!(has(&findings, "concept-not-indexed"));
}

#[test]
fn a_concept_without_frontmatter_is_an_error() {
    let (_tmp, kb_root) = fixture();
    fs::write(demo_root(&kb_root).join("bare.md"), "# Bare\n\nNo fence.\n").unwrap();
    let findings = run(&kb_root);
    assert!(
        findings
            .iter()
            .any(|f| f.check == "concept-no-frontmatter" && f.severity == Severity::Error),
        "got {:?}",
        checks(&findings)
    );
}

#[test]
fn broken_yaml_reports_frontmatter_invalid() {
    let (_tmp, kb_root) = fixture();
    fs::write(
        demo_root(&kb_root).join("dup.md"),
        "---\ntype: A\ntype: B\n---\n\nBody.\n",
    )
    .unwrap();
    let findings = run(&kb_root);
    assert!(
        findings.iter().any(|f| f.check == "frontmatter-invalid"
            && f.severity == Severity::Error
            && f.message.starts_with("YAML did not parse: ")),
        "got {:?}",
        checks(&findings)
    );
}

#[test]
fn a_sub_index_must_have_no_frontmatter() {
    let (_tmp, kb_root) = fixture();
    let sub = demo_root(&kb_root).join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(
        sub.join("index.md"),
        "---\ntitle: Sub\n---\n\n# Sub\n\n## Orientation\n",
    )
    .unwrap();
    let findings = run(&kb_root);
    assert!(has(&findings, "subindex-has-frontmatter"));

    fs::write(sub.join("index.md"), "# Sub\n\n## Orientation\n").unwrap();
    let clean = run(&kb_root);
    assert!(!has(&clean, "subindex-has-frontmatter"));
}

#[test]
fn a_readme_inside_a_bundle_is_an_error() {
    let (_tmp, kb_root) = fixture();
    fs::write(
        demo_root(&kb_root).join("README.md"),
        "---\ntype: Concept\n---\n\nWrong place.\n",
    )
    .unwrap();
    let findings = run(&kb_root);
    assert!(
        findings
            .iter()
            .any(|f| f.check == "readme-in-bundle" && f.severity == Severity::Error),
        "got {:?}",
        checks(&findings)
    );
}

#[test]
fn markdown_outside_any_bundle_is_a_stray() {
    let (_tmp, kb_root) = fixture();
    fs::write(kb_root.join("bundles").join("loose.md"), "# Loose\n").unwrap();
    // A grouping directory's README.md is expected and not a stray.
    fs::write(kb_root.join("bundles").join("README.md"), "# Bundles\n").unwrap();
    let findings = run(&kb_root);
    let strays: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.check == "stray-markdown")
        .collect();
    assert_eq!(strays.len(), 1, "got {:?}", checks(&findings));
    assert!(strays[0].path.ends_with("loose.md"));
    assert_eq!(strays[0].severity, Severity::Error);
}

#[test]
fn missing_title_and_description_are_warnings() {
    let (_tmp, kb_root) = fixture();
    fs::write(
        demo_root(&kb_root).join("thin.md"),
        "---\ntype: Concept\n---\n\nBody.\n",
    )
    .unwrap();
    let findings = run(&kb_root);
    assert!(
        findings
            .iter()
            .any(|f| f.check == "concept-missing-title" && f.severity == Severity::Warn)
    );
    assert!(
        findings
            .iter()
            .any(|f| f.check == "concept-missing-description" && f.severity == Severity::Warn)
    );
}

// ---------------------------------------------------------------- lifecycle

#[test]
fn an_unknown_status_is_flagged_and_a_known_one_is_not() {
    let (_tmp, kb_root) = fixture();
    fs::write(
        demo_root(&kb_root).join("odd.md"),
        "---\ntype: Concept\ntitle: Odd\ndescription: X.\nstatus: shiny\n---\n\nBody.\n",
    )
    .unwrap();
    fs::write(
        demo_root(&kb_root).join("fine.md"),
        "---\ntype: Concept\ntitle: Fine\ndescription: X.\nstatus: draft\n---\n\nBody.\n",
    )
    .unwrap();
    let findings = run(&kb_root);
    let status: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.check == "status-unknown")
        .collect();
    assert_eq!(status.len(), 1, "got {:?}", checks(&findings));
    assert!(status[0].path.ends_with("odd.md"));
    assert_eq!(
        status[0].message,
        "status `shiny` is not one of deprecated, draft, stable"
    );
}

#[test]
fn a_passed_stale_after_warns_and_a_future_one_does_not() {
    let (_tmp, kb_root) = fixture();
    fs::write(
        demo_root(&kb_root).join("old.md"),
        "---\ntype: Concept\ntitle: Old\ndescription: X.\nstale_after: 2026-01-01\n---\n\nBody.\n",
    )
    .unwrap();
    fs::write(
        demo_root(&kb_root).join("new.md"),
        "---\ntype: Concept\ntitle: New\ndescription: X.\nstale_after: 2027-01-01\n---\n\nBody.\n",
    )
    .unwrap();
    let findings = run(&kb_root);
    let stale: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.check == "stale-after-passed")
        .collect();
    assert_eq!(stale.len(), 1, "got {:?}", checks(&findings));
    assert!(stale[0].path.ends_with("old.md"));
    assert_eq!(
        stale[0].message,
        "stale_after 2026-01-01 has passed — content is due for review"
    );
}

#[test]
fn duplicate_titles_within_a_bundle_are_flagged_on_every_holder() {
    let (_tmp, kb_root) = fixture();
    fs::write(
        demo_root(&kb_root).join("one.md"),
        "---\ntype: Concept\ntitle: Same\ndescription: X.\n---\n\nBody.\n",
    )
    .unwrap();
    fs::write(
        demo_root(&kb_root).join("two.md"),
        "---\ntype: Concept\ntitle: Same\ndescription: Y.\n---\n\nBody.\n",
    )
    .unwrap();
    let findings = run(&kb_root);
    let dups: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.check == "duplicate-title")
        .collect();
    assert_eq!(dups.len(), 2, "got {:?}", checks(&findings));
    assert!(
        dups.iter()
            .all(|f| f.message == "title `Same` is used by 2 concepts in demo")
    );
}

// ------------------------------------------------------------------ figures

#[test]
fn numbered_captions_in_order_pass_while_missing_and_out_of_order_are_flagged() {
    let good = "---\ntype: Concept\ntitle: G\ndescription: Good figures.\n---\n\nIntro, see Figure 1.\n\n\
        ```mermaid\nflowchart LR\n  a --> b\n```\n\n**Figure 1:** a feeds b.\n\n\
        ![alt text](pic.svg)\n\nFigure 2: the picture.\n";
    let bad = "---\ntype: Concept\ntitle: B\ndescription: Bad figures.\n---\n\n\
        ```mermaid\nflowchart LR\n  a --> b\n```\n\nNot a caption.\n\n\
        ```mermaid\nflowchart LR\n  b --> c\n```\n\n**Figure 5:** wrong number.\n";
    let (_tmp, kb_root) = fixture();
    fs::write(demo_root(&kb_root).join("good.md"), good).unwrap();
    fs::write(demo_root(&kb_root).join("bad.md"), bad).unwrap();

    let findings = run(&kb_root);
    let figs: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.check.starts_with("figure-"))
        .collect();
    assert!(
        !figs.iter().any(|f| f.path.ends_with("good.md")),
        "good.md must be clean, got {:?}",
        figs.iter().map(|f| &f.check).collect::<Vec<_>>()
    );
    assert!(
        figs.iter()
            .any(|f| f.path.ends_with("bad.md") && f.check == "figure-caption-missing"),
        "uncaptioned figure flagged"
    );
    assert!(
        figs.iter()
            .any(|f| f.path.ends_with("bad.md") && f.check == "figure-number-out-of-sequence"),
        "misnumbered figure flagged"
    );
}

#[test]
fn non_mermaid_code_fences_and_their_contents_are_not_figures() {
    let doc = "---\ntype: Concept\ntitle: C\ndescription: Code only.\n---\n\n\
        ```scala\nval image = \"![not a figure](x.png)\"\n```\n\nProse, not a caption.\n\n\
        ```text\n```mermaid is mentioned here\n```\n";
    let (_tmp, kb_root) = fixture();
    fs::write(demo_root(&kb_root).join("code.md"), doc).unwrap();
    let findings = run(&kb_root);
    assert!(
        !findings
            .iter()
            .any(|f| f.check.starts_with("figure-") && f.path.ends_with("code.md")),
        "code fences must not count as figures, got {:?}",
        checks(&findings)
    );
}

// -------------------------------------------------------------- index drift

#[test]
fn index_drift_is_detected_and_repaired_by_refresh() {
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "a.md", "A", "The real description.");
    let index = demo_root(&kb_root).join("index.md");
    let text = fs::read_to_string(&index).unwrap();
    fs::write(&index, text.replace("The real description.", "stale text")).unwrap();

    let before = run(&kb_root);
    assert!(has(&before, "index-description-drift"), "drift detected");

    let actions =
        refresh::refresh_markdown(&load(&kb_root), false, "Orientation", false, today()).unwrap();
    assert!(
        actions
            .iter()
            .any(|a| a.kind == refresh::RefreshKind::DescriptionFixed),
        "refresh reports the fix"
    );

    let after = run(&kb_root);
    assert!(
        !has(&after, "index-description-drift"),
        "drift gone after refresh"
    );
}

#[test]
fn drift_normalization_is_lenient_about_case_whitespace_and_trailing_period() {
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "a.md", "A", "The real description.");
    let index = demo_root(&kb_root).join("index.md");
    let text = fs::read_to_string(&index).unwrap();
    // Case, collapsed whitespace and the trailing period differ; wording does not.
    fs::write(
        &index,
        text.replace("The real description.", "the REAL   description"),
    )
    .unwrap();
    let findings = run(&kb_root);
    assert!(
        !has(&findings, "index-description-drift"),
        "got {:?}",
        checks(&findings)
    );
}

#[test]
fn a_bullet_with_no_description_reports_no_drift() {
    let (_tmp, kb_root) = fixture();
    add_concept(&kb_root, "a.md", "A", "The real description.");
    let index = demo_root(&kb_root).join("index.md");
    let text = fs::read_to_string(&index).unwrap();
    fs::write(
        &index,
        text.replace("* [A](/a.md) - The real description.", "* [A](/a.md)"),
    )
    .unwrap();
    let findings = run(&kb_root);
    assert!(!has(&findings, "index-description-drift"));
}

// ----------------------------------------------------------------- vendored

/// A bundle with a mirror: everything under `sources/` is vendored.
fn vendored_fixture() -> (TempDir, PathBuf) {
    let (tmp, kb_root) = fixture();
    let broot = demo_root(&kb_root);
    fs::write(broot.join("sync.yaml"), "root: sources\n").unwrap();
    fs::create_dir_all(broot.join("sources").join("docs")).unwrap();
    (tmp, kb_root)
}

#[test]
fn vendored_docs_skip_the_authored_concept_checks() {
    let (_tmp, kb_root) = vendored_fixture();
    fs::write(
        demo_root(&kb_root)
            .join("sources")
            .join("docs")
            .join("upstream.md"),
        "---\ntype: Specification Source\ntitle: Upstream\nsidebar_position: 2.5\nstatus: partial\n---\n\nTheirs.\n",
    )
    .unwrap();
    let findings = run(&kb_root);
    let noisy: Vec<&Finding> = findings
        .iter()
        .filter(|f| {
            f.path.contains("upstream.md")
                && (f.check == "frontmatter-unknown-key"
                    || f.check == "status-unknown"
                    || f.check == "concept-missing-description")
        })
        .collect();
    assert!(
        noisy.is_empty(),
        "upstream frontmatter is not ours to lint, got {:?}",
        noisy.iter().map(|f| &f.check).collect::<Vec<_>>()
    );
}

#[test]
fn a_vendored_doc_without_type_reports_the_damaged_injection() {
    let (_tmp, kb_root) = vendored_fixture();
    fs::write(
        demo_root(&kb_root)
            .join("sources")
            .join("docs")
            .join("naked.md"),
        "---\ntitle: Naked\n---\n\nNo type at all.\n",
    )
    .unwrap();
    let findings = run(&kb_root);
    let f = findings
        .iter()
        .find(|f| f.check == "concept-missing-type" && f.path.contains("naked.md"))
        .expect("mirrored concept without type is flagged");
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(
        f.message,
        "mirrored concept has no `type` — the kb frontmatter block is missing or damaged"
    );
    assert_eq!(
        f.hint.as_deref(),
        Some("re-run `kb sync pull` for this bundle")
    );
}

#[test]
fn vendored_links_check_only_relative_file_paths_and_warn_as_upstream() {
    let (_tmp, kb_root) = vendored_fixture();
    let docs = demo_root(&kb_root).join("sources").join("docs");
    fs::write(
        docs.join("types.md"),
        "---\ntype: Specification Source\ntitle: Types\n---\n\nProse.\n",
    )
    .unwrap();
    fs::write(
        docs.join("guide.md"),
        "---\ntype: Specification Source\ntitle: Guide\n---\n\n\
         See [types](types.md), [gone](missing.md), [route](../migration-guide/) \
         and [site](/schemas/x.yaml).\n",
    )
    .unwrap();
    let findings = run(&kb_root);
    let broken: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.path.contains("guide.md") && f.check.starts_with("link-broken"))
        .collect();
    // Only the relative link that names a missing file is reported; the
    // directory route and the bundle-relative site path are upstream's own
    // routing, and the existing sibling resolves.
    assert_eq!(
        broken.len(),
        1,
        "got {:?}",
        broken.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
    assert_eq!(broken[0].check, "link-broken-upstream");
    assert_eq!(broken[0].severity, Severity::Warn);
    assert_eq!(
        broken[0].hint.as_deref(),
        Some("upstream's own link; fix it there and export, or leave it")
    );
}

// ----------------------------------------------------------- frontmatter keys

#[test]
fn unknown_frontmatter_keys_are_info_and_producer_keys_are_recognized() {
    let (_tmp, kb_root) = fixture();
    fs::write(
        demo_root(&kb_root).join("odd.md"),
        "---\ntype: Concept\ntitle: Odd\ndescription: X.\nmystery_key: 1\n---\n\nBody.\n",
    )
    .unwrap();
    let decisions = demo_root(&kb_root).join("decisions");
    fs::create_dir_all(&decisions).unwrap();
    fs::write(
        decisions.join("0001-first.md"),
        "---\ntype: Decision Record\ntitle: First\ndescription: \"Something was decided.\"\nstate: Accepted\ndecided: 2026-07-28\n---\n\nBody.\n",
    )
    .unwrap();

    let findings = run(&kb_root);
    let unknown: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.check == "frontmatter-unknown-key")
        .collect();
    assert_eq!(
        unknown.len(),
        1,
        "got {:?}",
        unknown.iter().map(|f| &f.message).collect::<Vec<_>>()
    );
    assert_eq!(unknown[0].severity, Severity::Info);
    assert!(unknown[0].path.ends_with("odd.md"));
    assert_eq!(
        unknown[0].message,
        "frontmatter key `mystery_key` is recognized by neither OKF v0.2 nor this tooling"
    );
}

// --------------------------------------------------------------- provenance

/// Initializes a git repository with one committed file, returning its HEAD.
/// `None` when git is unavailable, so the provenance tests can skip.
fn git_repo(dir: &Path) -> Option<String> {
    fs::create_dir_all(dir.join("docs")).ok()?;
    fs::write(dir.join("docs").join("x.md"), "content\n").ok()?;
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
    };
    git(&["init", "-q"])?;
    git(&["add", "-A"])?;
    git(&[
        "-c",
        "user.email=kb@test",
        "-c",
        "user.name=kb",
        "commit",
        "-q",
        "-m",
        "init",
        "--no-gpg-sign",
    ])?;
    sync::git_head(dir)
}

fn pinned_concept(kb_root: &Path, name: &str, url: &str) {
    let kb = load(kb_root);
    let b = kb.bundle("demo").unwrap();
    scaffold::add_concept(
        b,
        name,
        "Concept",
        "Pinned",
        "Pinned to upstream.",
        &[],
        None,
        &[SourceRef {
            id: None,
            resource: url.to_string(),
            title: None,
        }],
        "Orientation",
        None,
        today(),
    )
    .unwrap();
}

#[test]
fn provenance_reports_ref_missing_drift_and_path_missing() {
    let (tmp, kb_root) = fixture();
    let refs = tmp.path().join(".refs");
    let Some(head) = git_repo(&refs.join("acme").join("widget")) else {
        eprintln!("skipping: git is unavailable in this environment");
        return;
    };

    // Pinned at HEAD with an existing path: silent.
    pinned_concept(
        &kb_root,
        "clean.md",
        &format!("https://github.com/acme/widget/blob/{head}/docs/x.md"),
    );
    // Pinned at HEAD's own prefix: a short pin is not drift.
    pinned_concept(
        &kb_root,
        "short.md",
        &format!(
            "https://github.com/acme/widget/blob/{}/docs/x.md",
            &head[..12]
        ),
    );
    // Pinned at another commit: drift.
    let other = "0123456789abcdef0123456789abcdef01234567";
    pinned_concept(
        &kb_root,
        "drift.md",
        &format!("https://github.com/acme/widget/blob/{other}/docs/x.md"),
    );
    // Pinned path that no longer exists: path-missing.
    pinned_concept(
        &kb_root,
        "moved.md",
        &format!("https://github.com/acme/widget/blob/{head}/docs/gone.md"),
    );
    // A repo with no checkout under .refs: ref-missing, at info level.
    pinned_concept(
        &kb_root,
        "elsewhere.md",
        &format!("https://github.com/acme/other/blob/{head}/README.md"),
    );

    let findings = check::run(&load(&kb_root), Some(&refs), today(), false);
    let prov: Vec<&Finding> = findings
        .iter()
        .filter(|f| f.check.starts_with("source-"))
        .collect();

    assert!(
        !prov
            .iter()
            .any(|f| f.path.contains("clean.md") || f.path.contains("short.md")),
        "clean pins are silent, got {:?}",
        prov.iter().map(|f| (&f.check, &f.path)).collect::<Vec<_>>()
    );
    let drift = prov
        .iter()
        .find(|f| f.check == "source-commit-drift")
        .expect("drift reported");
    assert_eq!(drift.severity, Severity::Warn);
    assert!(drift.path.contains("drift.md"));
    assert_eq!(
        drift.message,
        format!(
            "source pinned at {} but .refs/acme/widget is at {}",
            &other[..8],
            &head[..8]
        )
    );
    let missing = prov
        .iter()
        .find(|f| f.check == "source-path-missing")
        .expect("path-missing reported");
    assert_eq!(missing.severity, Severity::Warn);
    assert!(missing.path.contains("moved.md"));
    assert_eq!(
        missing.message,
        "source path no longer present at the checkout's HEAD: docs/gone.md"
    );
    let ref_missing = prov
        .iter()
        .find(|f| f.check == "source-ref-missing")
        .expect("ref-missing reported");
    assert_eq!(ref_missing.severity, Severity::Info);
    assert_eq!(
        ref_missing.message,
        "no reference checkout for acme/other under .refs/"
    );
}

#[test]
fn provenance_is_skipped_without_a_refs_root() {
    let (_tmp, kb_root) = fixture();
    pinned_concept(
        &kb_root,
        "pinned.md",
        "https://github.com/acme/widget/blob/0123456789abcdef0123456789abcdef01234567/docs/x.md",
    );
    let findings = run(&kb_root);
    assert!(!findings.iter().any(|f| f.check.starts_with("source-")));
}

// -------------------------------------------------------------- composition

#[test]
fn run_full_folds_in_sync_findings_and_surfaces_exit_semantics() {
    let (_tmp, kb_root) = vendored_fixture();
    // `root: sources` alone is a manifest this tooling refuses: no
    // upstream.repo, no mappings.
    let kb = load(&kb_root);
    let report = check::run_full(&kb, &CheckOpts::default(), today()).unwrap();
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.check == "sync-manifest-invalid" && f.severity == Severity::Error),
        "got {:?}",
        checks(&report.findings)
    );
    assert_eq!(report.errors(), 1);
    assert!(report.should_fail(false), "errors always fail");
}

#[test]
fn strict_promotes_warnings_to_failure() {
    let (_tmp, kb_root) = fixture();
    fs::write(
        demo_root(&kb_root).join("thin.md"),
        "---\ntype: Concept\ntitle: Thin\ndescription: X.\n---\n\nBody.\n",
    )
    .unwrap();
    let kb = load(&kb_root);
    let report = check::run_full(&kb, &CheckOpts::default(), today()).unwrap();
    assert_eq!(report.errors(), 0, "got {:?}", checks(&report.findings));
    assert!(report.warnings() > 0, "concept-not-indexed warns");
    assert!(!report.should_fail(false));
    assert!(report.should_fail(true));
}

#[test]
fn findings_sort_by_severity_then_path_then_line() {
    let (_tmp, kb_root) = fixture();
    // zz-error.md sorts after warnings alphabetically but first by severity.
    fs::write(
        demo_root(&kb_root).join("zz-error.md"),
        "# No frontmatter\n",
    )
    .unwrap();
    fs::write(
        demo_root(&kb_root).join("aa-warn.md"),
        "---\ntype: Concept\ntitle: A\ndescription: X.\n---\n\nBody.\n",
    )
    .unwrap();
    let findings = run(&kb_root);
    let severities: Vec<Severity> = findings.iter().map(|f| f.severity).collect();
    let mut sorted = severities.clone();
    sorted.sort();
    assert_eq!(severities, sorted, "severity is the primary key");
    for pair in findings.windows(2) {
        if pair[0].severity == pair[1].severity {
            assert!(
                pair[0].path <= pair[1].path,
                "path is the secondary key: {} vs {}",
                pair[0].path,
                pair[1].path
            );
        }
    }
}

// ---------------------------------------------------------------- rendering

fn sample_findings() -> Vec<Finding> {
    vec![
        Finding {
            severity: Severity::Error,
            check: "concept-missing-type".to_string(),
            path: "kb/bundles/x/y.md".to_string(),
            line: Some(1),
            message: "concept has no `type` — the one universally required OKF field".to_string(),
            hint: None,
        },
        Finding {
            severity: Severity::Warn,
            check: "index-description-drift".to_string(),
            path: "kb/bundles/x/index.md".to_string(),
            line: Some(3),
            message: "index entry text differs from the concept's description (/y.md)".to_string(),
            hint: Some("concept says: The real one.".to_string()),
        },
        Finding {
            severity: Severity::Info,
            check: "frontmatter-unknown-key".to_string(),
            path: "kb/bundles/x/y.md".to_string(),
            line: Some(1),
            message: "frontmatter key `zz` is recognized by neither OKF v0.2 nor this tooling"
                .to_string(),
            hint: None,
        },
    ]
}

#[test]
fn render_text_matches_the_scala_layout_and_summary() {
    let text = check::render_text(&sample_findings(), false);
    let expected = "\
error  concept-missing-type        kb/bundles/x/y.md:1
       concept has no `type` — the one universally required OKF field
warn   index-description-drift     kb/bundles/x/index.md:3
       index entry text differs from the concept's description (/y.md)
       hint: concept says: The real one.

1 error(s), 1 warning(s), 1 info (hidden; pass --verbose)
";
    assert_eq!(text, expected);
}

#[test]
fn render_text_verbose_shows_info_and_drops_the_hidden_note() {
    let text = check::render_text(&sample_findings(), true);
    assert!(text.contains("frontmatter-unknown-key"));
    assert!(text.ends_with("\n1 error(s), 1 warning(s), 1 info\n"));
}

#[test]
fn render_text_with_no_findings_says_so() {
    assert_eq!(
        check::render_text(&[], false),
        "no findings\n\n0 error(s), 0 warning(s), 0 info\n"
    );
}

#[test]
fn render_json_matches_the_scala_shape_byte_for_byte() {
    let json = check::render_json(&sample_findings()[..2]);
    let expected = r#"{
  "errors": 1,
  "warnings": 1,
  "infos": 0,
  "findings": [
    {
      "severity": "error",
      "check": "concept-missing-type",
      "path": "kb/bundles/x/y.md",
      "line": 1,
      "message": "concept has no `type` — the one universally required OKF field",
      "hint": null
    },
    {
      "severity": "warn",
      "check": "index-description-drift",
      "path": "kb/bundles/x/index.md",
      "line": 3,
      "message": "index entry text differs from the concept's description (/y.md)",
      "hint": "concept says: The real one."
    }
  ]
}
"#;
    assert_eq!(json, expected);
}

#[test]
fn render_json_with_no_findings_uses_the_empty_array_form() {
    assert_eq!(
        check::render_json(&[]),
        "{\n  \"errors\": 0,\n  \"warnings\": 0,\n  \"infos\": 0,\n  \"findings\": []\n}\n"
    );
}

#[test]
fn write_report_writes_the_file_and_returns_the_confirmation_line() {
    let tmp = TempDir::new().unwrap();
    let out = tmp.path().join("nested").join("report.txt");
    let msg = check::write_report("the report\n", &out).unwrap();
    assert_eq!(fs::read_to_string(&out).unwrap(), "the report\n");
    assert_eq!(msg, format!("wrote {}", paths::render(&out)));
}
