//! The two round trips a document tree has to make: a tree reads to a document and writes back to
//! the same tree, and a document writes to a tree and reads back to the same document.
//!
//! The reader sorts modules and the writer does not, so a round trip is stable only because the
//! sort happens once, on the way in. A `mode=read` set (`document-tree-0005`) has only the first
//! half: its `$meta` members are stripped on the way in and a writer never puts them back.

mod common;

use common::CASES;
use morphir_core::ir::layout::{read_tree, write_tree};
use morphir_core::ir::{IRFile, json};

fn canonical(file: &IRFile) -> String {
    json::write_ir_file(file)
}

#[test]
fn every_writable_kit_set_writes_back_to_itself_after_a_read() {
    for case in CASES.iter().filter(|case| case.writable) {
        let files = case.tree();
        let (file, warnings) = read_tree(&files, case.profile)
            .unwrap_or_else(|error| panic!("{} reads: {error:?}", case.id));
        assert!(warnings.is_empty(), "{} warns about nothing", case.id);

        let written = write_tree(&file, &case.policy())
            .unwrap_or_else(|error| panic!("{} writes: {error:?}", case.id));
        assert_eq!(
            written
                .iter()
                .map(|(path, text)| (path.as_str(), text.as_str()))
                .collect::<Vec<_>>(),
            case.files.to_vec(),
            "{}: write_tree(read_tree(files)) is files",
            case.id
        );
    }
}

#[test]
fn every_writable_kit_document_reads_back_to_itself_after_a_write() {
    for case in CASES.iter().filter(|case| case.writable) {
        let document = case.ir_file();
        let written = write_tree(&document, &case.policy())
            .unwrap_or_else(|error| panic!("{} writes: {error:?}", case.id));
        let files = written.into_iter().collect();

        let (read_back, warnings) = read_tree(&files, case.profile)
            .unwrap_or_else(|error| panic!("{} reads back: {error:?}", case.id));
        assert!(warnings.is_empty(), "{} warns about nothing", case.id);
        assert_eq!(
            canonical(&read_back),
            canonical(&document),
            "{}: read_tree(write_tree(doc)) is doc",
            case.id
        );
    }
}

#[test]
fn a_read_only_kit_set_still_reads_to_its_document() {
    for case in CASES.iter().filter(|case| !case.writable) {
        let (file, _) = read_tree(&case.tree(), case.profile)
            .unwrap_or_else(|error| panic!("{} reads: {error:?}", case.id));
        assert_eq!(canonical(&file), canonical(&case.ir_file()), "{}", case.id);
    }
}

/// Modules come back sorted, so writing that result emits them sorted too — the writer itself
/// never reorders anything.
#[test]
fn a_tree_whose_modules_are_out_of_order_writes_back_sorted() {
    let files = common::tree(&[
        (
            "manifest",
            "formatVersion: 4\ndistribution: Library\npackage: example\npathBudget: 4000\n",
        ),
        (
            "pkg/example/zeta/module",
            "formatVersion: 4\npath: zeta\ntypes: []\nvalues: []\n",
        ),
        (
            "pkg/example/alpha/module",
            "formatVersion: 4\npath: alpha\ntypes: []\nvalues: []\n",
        ),
    ]);
    let (file, _) = read_tree(&files, morphir_core::ir::layout::Profile::Yaml).expect("it reads");
    let written = write_tree(
        &file,
        &morphir_core::ir::layout::TreePolicy {
            profile: morphir_core::ir::layout::Profile::Yaml,
            path_budget: 4000,
        },
    )
    .expect("it writes");

    assert_eq!(
        written
            .iter()
            .map(|(path, _)| path.as_str())
            .collect::<Vec<_>>(),
        vec![
            "manifest",
            "pkg/example/alpha/module",
            "pkg/example/zeta/module",
        ],
    );
    assert_eq!(
        written
            .into_iter()
            .collect::<morphir_core::ir::layout::Tree>(),
        files
    );
}
