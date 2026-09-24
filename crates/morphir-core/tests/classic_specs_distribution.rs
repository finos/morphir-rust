use morphir_core::ir::classic::{Distribution, DistributionBody};

const SPECS: &str = r#"{
  "formatVersion": "3.1.0",
  "distribution": ["Specs", [["my"], ["pkg"]], [],
    { "modules": [ [ [["basics"]], { "types": [ [["int"], { "doc": "", "value": ["OpaqueTypeSpecification", []] }] ], "values": [], "doc": null } ] ] }]
}"#;

#[test]
fn a_specs_distribution_reads_and_writes_as_3_1_0() {
    let file: Distribution = serde_json::from_str(SPECS).unwrap();
    assert!(matches!(file.distribution, DistributionBody::Specs(..)));
    let written = serde_json::to_value(&file).unwrap();
    assert_eq!(written["formatVersion"], "3.1.0");
    assert_eq!(written["distribution"][0], "Specs");
    assert_eq!(
        serde_json::from_value::<Distribution>(written).unwrap(),
        file
    );
}

#[test]
fn a_library_read_as_3_1_0_is_written_as_3() {
    let text = r#"{"formatVersion":"3.1.0","distribution":["Library",[["my"]],[],{"modules":[]}]}"#;
    let file: Distribution = serde_json::from_str(text).unwrap();
    assert_eq!(serde_json::to_value(&file).unwrap()["formatVersion"], 3);
}

#[test]
fn an_unknown_distribution_tag_names_both_kinds() {
    let text = r#"{"formatVersion":3,"distribution":["Application",[["my"]],[],{"modules":[]}]}"#;
    let error = serde_json::from_str::<Distribution>(text)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("Library") && error.contains("Specs"),
        "{error}"
    );
}

#[test]
fn a_specs_distribution_migrates_to_a_v4_specs_distribution() {
    use morphir_core::ir::v4;
    use morphir_core::migration::{MigrationOptions, migrate_distribution};

    let file: Distribution = serde_json::from_str(SPECS).unwrap();
    let migrated = migrate_distribution(&file, MigrationOptions::default()).unwrap();
    assert!(migrated.report.can_publish());
    assert_eq!(
        migrated.value.format_version,
        v4::FormatVersion::String("4.0.0".to_owned())
    );
    let v4::Distribution::Specs(content) = migrated.value.distribution else {
        panic!("a v4 Specs distribution");
    };
    assert_eq!(content.package_name.to_string(), "my/pkg");
    assert!(content.dependencies.is_empty());
    let basics = &content.spec.modules["basics"];
    assert!(basics.types.contains_key("int"), "{basics:?}");
}
