//! The whole-file entry points: `yaml::read_ir_file`, `yaml::write_ir_file` and the decode they
//! are built on, `v4::decode_ir_file_with_warnings`.
//!
//! These are what a caller with a `.morphir-ir.yaml` in hand uses, so they are held to the two
//! promises the parts already make separately: a document the reader accepts decodes to the same
//! `IRFile` serde decodes, carrying the warnings the serde path leaves to its caller's scope, and
//! what the writer emits for a file is the canonical spelling of that file's value tree.

use morphir_core::ir::DiagnosticCode;
use morphir_core::ir::v4::{
    IRFile, TypeEncoding, decode_ir_file_with_warnings, with_type_encoding,
};
use morphir_core::ir::yaml::{read_ir_file, write_canonical, write_ir_file};
use serde_json::{Value, json};

/// The v4 library distribution the rest of the v4 tests read.
fn library_fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/ir/v4/v4-library-distribution.json"))
        .expect("the v4 library fixture is JSON")
}

/// A v4 file whose one value body spells `IfThenElse`'s branch the way decision 0006's window
/// still accepts: `thenBranch` for `then`.
fn legacy_spelling_document() -> Value {
    json!({
        "formatVersion": 4,
        "distribution": { "Library": {
            "packageName": "example/legacy",
            "dependencies": {},
            "def": { "modules": { "domain": {
                "access": "Public",
                "value": {
                    "types": {},
                    "values": { "pick": {
                        "access": "Public",
                        "ExpressionBody": {
                            "inputTypes": {},
                            "outputType": "morphir/SDK:basics#int",
                            "body": { "IfThenElse": {
                                "condition": false,
                                "thenBranch": 21,
                                "else": 22
                            } }
                        }
                    } }
                }
            } } }
        } }
    })
}

#[test]
fn read_ir_file_decodes_what_serde_decodes() {
    let fixture = library_fixture();
    let document = write_canonical(&fixture);

    let (from_yaml, warnings) = read_ir_file(&document).expect("the fixture is profile YAML");
    let from_serde: IRFile = serde_json::from_value(fixture).expect("the fixture is a v4 file");

    assert_eq!(from_yaml, from_serde);
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn read_ir_file_reports_the_decoder_warnings() {
    let document = write_canonical(&legacy_spelling_document());

    let (_file, warnings) = read_ir_file(&document).expect("a legacy spelling still decodes");

    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].code, DiagnosticCode::LegacySpelling);
    assert!(
        warnings[0].cursor.ends_with("/IfThenElse/thenBranch"),
        "{:?}",
        warnings[0].cursor
    );
}

#[test]
fn read_ir_file_answers_a_profile_violation_with_the_readers_diagnostic() {
    let error = read_ir_file("formatVersion: 4\nformatVersion: 4\n")
        .expect_err("a repeated member is not profile YAML");

    assert_eq!(error.0.code, DiagnosticCode::DuplicateMember);
}

#[test]
fn decode_ir_file_with_warnings_reports_a_legacy_spelling() {
    let (file, warnings) =
        decode_ir_file_with_warnings(&legacy_spelling_document()).expect("the document decodes");

    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].code, DiagnosticCode::LegacySpelling);
    // The decode is the one `Deserialize for IRFile` performs, warnings aside.
    let from_serde: IRFile =
        serde_json::from_value(legacy_spelling_document()).expect("the document decodes");
    assert_eq!(file, from_serde);
}

#[test]
fn write_ir_file_is_the_canonical_spelling_of_the_files_value_tree() {
    let file: IRFile = serde_json::from_value(library_fixture()).expect("the fixture is a v4 file");

    let expected = write_canonical(
        &with_type_encoding(TypeEncoding::Compact, || serde_json::to_value(&file))
            .expect("an IRFile serialises"),
    );

    assert_eq!(write_ir_file(&file), expected);
}

/// A type expression carrying no attributes is written in the canonical compact spelling, which
/// is what a reader sees as `morphir/SDK:basics#int`. The encoding is a thread-local defaulting
/// to `Expanded`, so a writer that does not select it emits the long wrapper for every type in
/// the file.
#[test]
fn write_ir_file_writes_type_expressions_compactly() {
    let file: IRFile = serde_json::from_value(library_fixture()).expect("the fixture is a v4 file");

    let document = write_ir_file(&file);

    assert!(
        document.contains("typeExp: morphir/SDK:string#string"),
        "expected the compact spelling:\n{document}"
    );
    assert!(
        document.contains("outputType: morphir/SDK:string#string"),
        "expected the compact spelling:\n{document}"
    );
    // `fqname` is the expanded `Reference` wrapper's member, and nothing else writes it.
    assert!(!document.contains("fqname:"), "{document}");
}

#[test]
fn a_file_written_as_yaml_reads_back_as_itself() {
    let file: IRFile = serde_json::from_value(library_fixture()).expect("the fixture is a v4 file");

    let (round_tripped, warnings) =
        read_ir_file(&write_ir_file(&file)).expect("the writer writes profile YAML");

    assert_eq!(round_tripped, file);
    assert!(warnings.is_empty(), "{warnings:?}");
}
