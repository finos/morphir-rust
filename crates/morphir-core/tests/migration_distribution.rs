use morphir_core::ir::{classic, v4};
use morphir_core::migration::{MigrationOptions, migrate_distribution};

#[test]
fn migrates_real_morphir_elm_v3_distribution() {
    let source: classic::Distribution =
        serde_json::from_str(include_str!("fixtures/ir/classic/greeting-example.json")).unwrap();

    let migrated = migrate_distribution(&source, MigrationOptions::default()).unwrap();
    let v4::Distribution::Library(library) = &migrated.value.distribution else {
        panic!("v3 libraries must migrate to v4 libraries");
    };

    assert_eq!(migrated.value.format_version, v4::FormatVersion::Integer(4));
    assert_eq!(library.package_name.to_string(), "elm-compat");
    assert!(!library.def.modules.is_empty());
    assert!(migrated.report.can_publish());

    let api = library.def.modules.get("api").unwrap();
    assert_eq!(
        api.value.doc.as_ref().unwrap().text(),
        "\u{20}API module demonstrating request/response patterns.\n\nThis module provides types \
         and functions for a simple API layer,\ndemonstrating how Morphir handles common API \
         patterns.\n"
    );
    assert!(!api.value.types.is_empty());
    assert!(!api.value.values.is_empty());
}

#[test]
fn rejects_a_non_v3_classic_distribution_at_the_typed_boundary() {
    let mut source: classic::Distribution =
        serde_json::from_str(include_str!("fixtures/ir/classic/greeting-example.json")).unwrap();
    source.format_version = 2;

    let error = migrate_distribution(&source, MigrationOptions::default()).unwrap_err();

    assert_eq!(error.code, "unsupported-source-version");
}

#[test]
fn refuses_a_classic_name_with_no_words_rather_than_migrating_it_to_nothing() {
    // `[[]]` is a Classic path holding one name with no words. A v4 name is one or more
    // segments, and every v4 reader refuses the empty string where a name belongs, so this has
    // no v4 spelling: the migration owes a diagnostic rather than a document nothing will read.
    let source: classic::Distribution = serde_json::from_str(
        r#"{"formatVersion": 3, "distribution": ["Library", [[]], [], {"modules": []}]}"#,
    )
    .unwrap();

    let error = migrate_distribution(&source, MigrationOptions::default()).unwrap_err();

    assert_eq!(error.code, "empty-name");
}
