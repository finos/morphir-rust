//! Bundle discovery, stray detection, vendoring, and link resolution —
//! ported from the store-facing cases of `KbTests.scala` (`KbLinkSpec` and
//! the loading behavior the other suites rely on), against temp-dir fixtures.

use morphir_okf::model::DocKind;
use morphir_okf::store;
use std::fs;
use std::path::{Component, Path, PathBuf};
use tempfile::TempDir;

const BUNDLE_INDEX: &str = "---\nokf_version: \"0.2\"\ntitle: Demo\ndescription: A scratch bundle.\n---\n\n# Demo\n\nA scratch bundle.\n\n## Orientation\n\n* [A](/a.md) - First.\n";

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// A minimal kb: one bundle `demo` with a log, two concepts, and a sub-index.
fn demo_kb() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let kb = tmp.path().join("kb");
    write(&kb, "bundles/demo/index.md", BUNDLE_INDEX);
    write(
        &kb,
        "bundles/demo/log.md",
        "# Log\n\n## 2026-07-28\n\n* **Creation**: Bundle created.\n",
    );
    write(
        &kb,
        "bundles/demo/a.md",
        "---\ntype: Concept\ntitle: A\ndescription: First.\n---\n\n# A\n\nFirst.\n",
    );
    write(&kb, "bundles/demo/sub/index.md", "# Sub\n");
    write(
        &kb,
        "bundles/demo/sub/b.md",
        "---\ntype: Concept\ntitle: B\ndescription: Second.\n---\n\n# B\n\nSee [a](../a.md).\n",
    );
    tmp
}

fn kb_root(tmp: &TempDir) -> PathBuf {
    tmp.path().join("kb")
}

// ------------------------------------------------------ kind classification

#[test]
fn kind_of_classifies_by_name_and_depth() {
    let segs = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(store::kind_of(&segs(&["index.md"])), DocKind::RootIndex);
    assert_eq!(
        store::kind_of(&segs(&["sub", "index.md"])),
        DocKind::SubIndex
    );
    assert_eq!(store::kind_of(&segs(&["log.md"])), DocKind::Log);
    assert_eq!(store::kind_of(&segs(&["sub", "log.md"])), DocKind::Log);
    assert_eq!(store::kind_of(&segs(&["naming.md"])), DocKind::Concept);
}

#[test]
fn loaded_bundle_partitions_docs_by_kind() {
    let tmp = demo_kb();
    let kb = store::load(&kb_root(&tmp)).unwrap();
    let b = kb.bundle("demo").expect("demo bundle");
    assert_eq!(b.index.kind, DocKind::RootIndex);
    assert!(b.log.is_some());
    assert_eq!(b.sub_indexes.len(), 1);
    assert_eq!(b.sub_indexes[0].bundle_path(), "/sub/index.md");
    let mut concepts: Vec<_> = b.concepts.iter().map(|d| d.bundle_path()).collect();
    concepts.sort();
    assert_eq!(concepts, vec!["/a.md", "/sub/b.md"]);
    assert_eq!(b.okf_version().as_deref(), Some("0.2"));
    assert_eq!(b.label(), "demo");
    assert!(b.group.is_none());
}

// --------------------------------------------------------- bundle discovery

#[test]
fn discovery_does_not_descend_into_a_found_bundle() {
    let tmp = demo_kb();
    let kb = kb_root(&tmp);
    // An index.md with okf_version *inside* the demo bundle must not create
    // a second bundle: discovery stops at the outer root.
    write(
        &kb,
        "bundles/demo/inner/index.md",
        "---\nokf_version: \"0.2\"\n---\n\n# Inner\n",
    );
    let roots = store::find_bundle_roots(&kb.join("bundles")).unwrap();
    assert_eq!(roots, vec![kb.join("bundles/demo")]);
}

#[test]
fn discovery_finds_grouped_bundles_and_sets_group_and_label() {
    let tmp = demo_kb();
    let kb = kb_root(&tmp);
    write(&kb, "bundles/grp/one/index.md", BUNDLE_INDEX);
    let loaded = store::load(&kb).unwrap();
    let labels: Vec<_> = loaded.bundles.iter().map(|b| b.label()).collect();
    assert_eq!(labels, vec!["demo".to_string(), "grp/one".to_string()]);
    let grouped = loaded.bundle("grp/one").expect("grouped bundle");
    assert_eq!(grouped.group.as_deref(), Some("grp"));
    assert_eq!(grouped.name, "one");
    // The bare name also resolves.
    assert!(loaded.bundle("one").is_some());
}

#[test]
fn a_directory_without_okf_version_is_not_a_bundle() {
    let tmp = TempDir::new().unwrap();
    let kb = tmp.path().join("kb");
    write(
        &kb,
        "bundles/notabundle/index.md",
        "# Just an index, no frontmatter\n",
    );
    let roots = store::find_bundle_roots(&kb.join("bundles")).unwrap();
    assert!(roots.is_empty());
}

#[test]
fn loading_a_missing_bundles_dir_yields_an_empty_kb() {
    let tmp = TempDir::new().unwrap();
    let kb = store::load(&tmp.path().join("kb")).unwrap();
    assert!(kb.bundles.is_empty());
    assert!(kb.strays.is_empty());
}

#[test]
fn load_bundle_without_a_root_index_is_an_error() {
    let tmp = TempDir::new().unwrap();
    let kb = tmp.path().join("kb");
    write(&kb, "bundles/broken/a.md", "# A\n");
    let err = store::load_bundle(&kb.join("bundles/broken"), &kb).unwrap_err();
    assert!(
        err.to_string().contains("has no root index.md"),
        "got: {err}"
    );
}

// ------------------------------------------------------------------ strays

#[test]
fn markdown_outside_any_bundle_is_a_stray_except_readme() {
    let tmp = demo_kb();
    let kb = kb_root(&tmp);
    write(&kb, "bundles/stray.md", "# Stray\n");
    write(&kb, "bundles/README.md", "# Bundles\n");
    write(&kb, "bundles/grp/README.md", "# Group\n");
    write(&kb, "bundles/grp/one/index.md", BUNDLE_INDEX);
    let loaded = store::load(&kb).unwrap();
    assert_eq!(loaded.strays, vec![kb.join("bundles/stray.md")]);
}

// -------------------------------------------------------- mirror / vendored

#[test]
fn sync_yaml_marks_the_mirror_and_vendors_its_docs() {
    let tmp = demo_kb();
    let kb = kb_root(&tmp);
    write(
        &kb,
        "bundles/demo/sync.yaml",
        "upstream:\n  repo: org/x\nroot: sources\nmappings:\n  - \"docs/**\"\n",
    );
    // Upstream's own index.md and log.md: the OKF reservation stops at the
    // mirror boundary, so both load as vendored concepts.
    write(&kb, "bundles/demo/sources/index.md", "# Upstream index\n");
    write(&kb, "bundles/demo/sources/log.md", "# Upstream log\n");
    write(
        &kb,
        "bundles/demo/sources/spec/types.md",
        "---\ntitle: Types\n---\n\n# Types\n",
    );
    write(&kb, "bundles/demo/sources/spec/schema.json", "{}\n");
    let loaded = store::load(&kb).unwrap();
    let b = loaded.bundle("demo").expect("demo bundle");

    assert_eq!(b.mirror, Some(vec!["sources".to_string()]));
    assert_eq!(b.mirror_root(), Some(kb.join("bundles/demo/sources")));

    let vendored: Vec<_> = b
        .concepts
        .iter()
        .filter(|d| d.vendored)
        .map(|d| d.bundle_path())
        .collect();
    assert_eq!(
        vendored,
        vec![
            "/sources/index.md",
            "/sources/log.md",
            "/sources/spec/types.md"
        ]
    );
    assert!(
        b.concepts
            .iter()
            .filter(|d| d.vendored)
            .all(|d| d.kind == DocKind::Concept)
    );

    // The bundle's own index and log stay what they are.
    assert_eq!(b.index.kind, DocKind::RootIndex);
    assert!(b.log.is_some());

    // Authored concepts exclude the mirrored ones.
    let mut authored: Vec<_> = b
        .authored_concepts()
        .iter()
        .map(|d| d.bundle_path())
        .collect();
    authored.sort();
    assert_eq!(authored, vec!["/a.md", "/sub/b.md"]);

    // Non-markdown files under the mirror are assets, never parsed.
    assert_eq!(b.assets.len(), 1);
    assert_eq!(b.assets[0].bundle_path(), "/sources/spec/schema.json");
    assert_eq!(b.assets[0].name(), "schema.json");
}

#[test]
fn sync_yaml_root_defaults_to_sources_and_honours_a_custom_path() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    write(root, "sync.yaml", "upstream:\n  repo: org/x\n");
    assert_eq!(
        store::mirror_segments(root).unwrap(),
        Some(vec!["sources".to_string()])
    );

    fs::write(root.join("sync.yaml"), "root: vendor/docs\n").unwrap();
    assert_eq!(
        store::mirror_segments(root).unwrap(),
        Some(vec!["vendor".to_string(), "docs".to_string()])
    );
}

#[test]
fn a_bundle_without_sync_yaml_has_no_mirror() {
    let tmp = demo_kb();
    let loaded = store::load(&kb_root(&tmp)).unwrap();
    let b = loaded.bundle("demo").unwrap();
    assert_eq!(b.mirror, None);
    assert!(b.assets.is_empty());
}

// --------------------------------------------------------- link resolution

#[test]
fn external_anchor_and_empty_destinations_do_not_resolve() {
    let tmp = demo_kb();
    let loaded = store::load(&kb_root(&tmp)).unwrap();
    let b = loaded.bundle("demo").unwrap();
    let doc = b.concept_at("/a.md").unwrap();
    let link = |dest: &str| morphir_okf::LinkRef {
        text: "t".into(),
        dest: dest.into(),
        line: 1,
    };
    assert_eq!(
        store::resolve_link(doc, &link("https://example.com/x")),
        None
    );
    assert_eq!(
        store::resolve_link(doc, &link("mailto:x@example.com")),
        None
    );
    assert_eq!(store::resolve_link(doc, &link("#section")), None);
    assert_eq!(store::resolve_link(doc, &link("")), None);
}

#[test]
fn a_bundle_relative_link_resolves_against_the_bundle_root() {
    let tmp = demo_kb();
    let loaded = store::load(&kb_root(&tmp)).unwrap();
    let b = loaded.bundle("demo").unwrap();
    let doc = b.concept_at("/sub/b.md").unwrap();
    let link = morphir_okf::LinkRef {
        text: "a".into(),
        dest: "/a.md".into(),
        line: 1,
    };
    assert_eq!(store::resolve_link(doc, &link), Some(b.root.join("a.md")));
}

#[test]
fn regression_an_ordinary_relative_link_resolves_against_the_containing_directory() {
    // Folding over a rebuilt segment list once dropped the leading root
    // marker, so every relative link resolved against the working directory
    // and was reported broken. Resolution must start from the containing
    // directory path itself.
    let tmp = demo_kb();
    let loaded = store::load(&kb_root(&tmp)).unwrap();
    let b = loaded.bundle("demo").unwrap();
    let doc = b.concept_at("/sub/b.md").unwrap();
    let link = doc
        .links
        .iter()
        .find(|l| l.dest == "../a.md")
        .expect("../a.md link");
    let resolved = store::resolve_link(doc, link).expect("resolves");
    assert_eq!(resolved, b.root.join("a.md"));
    assert!(resolved.is_file(), "../a.md exists and should resolve");
}

#[test]
fn dot_segments_and_fragments_are_handled() {
    let tmp = demo_kb();
    let kb = kb_root(&tmp);
    write(
        &kb,
        "bundles/demo/sub/c.md",
        "---\ntype: Concept\n---\n\n# C\n",
    );
    let loaded = store::load(&kb).unwrap();
    let b = loaded.bundle("demo").unwrap();
    let doc = b.concept_at("/sub/b.md").unwrap();
    let link = |dest: &str| morphir_okf::LinkRef {
        text: "t".into(),
        dest: dest.into(),
        line: 1,
    };
    assert_eq!(
        store::resolve_link(doc, &link("./c.md")),
        Some(b.root.join("sub").join("c.md"))
    );
    assert_eq!(
        store::resolve_link(doc, &link("../a.md#heading")),
        Some(b.root.join("a.md"))
    );
}

#[test]
fn regression_a_bundle_relative_link_climbing_above_the_root_keeps_its_dot_dot() {
    // `/../demo/a.md` resolves on disk — bundleRoot + "../demo/a.md" is the
    // same file — so the escape must stay visible in the resolved path for
    // the `link-escapes-bundle` check to see, rather than being collapsed.
    let tmp = demo_kb();
    let loaded = store::load(&kb_root(&tmp)).unwrap();
    let b = loaded.bundle("demo").unwrap();
    let doc = b.concept_at("/a.md").unwrap();
    let link = morphir_okf::LinkRef {
        text: "sideways".into(),
        dest: "/../demo/a.md".into(),
        line: 1,
    };
    let resolved = store::resolve_link(doc, &link).expect("resolves");
    assert_eq!(resolved, b.root.join("../demo/a.md"));
    assert!(
        resolved.components().any(|c| c == Component::ParentDir),
        "`..` must survive"
    );
}

// ------------------------------------------------------------ misc surface

#[test]
fn kb_rel_renders_paths_under_the_root_with_the_root_name() {
    let tmp = demo_kb();
    let loaded = store::load(&kb_root(&tmp)).unwrap();
    let b = loaded.bundle("demo").unwrap();
    assert_eq!(loaded.rel(&b.root.join("a.md")), "kb/bundles/demo/a.md");
}

#[test]
fn display_title_falls_back_to_the_filename() {
    let tmp = demo_kb();
    let kb = kb_root(&tmp);
    write(
        &kb,
        "bundles/demo/untitled.md",
        "---\ntype: Concept\n---\n\nbody\n",
    );
    let loaded = store::load(&kb).unwrap();
    let b = loaded.bundle("demo").unwrap();
    assert_eq!(b.concept_at("/a.md").unwrap().display_title(), "A");
    assert_eq!(
        b.concept_at("/untitled.md").unwrap().display_title(),
        "untitled"
    );
}
