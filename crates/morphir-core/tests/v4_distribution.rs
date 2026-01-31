//! V4 Distribution loading tests
//!
//! Tests for loading V4 format distributions with V4-specific constructs.

use morphir_core::ir::v4::{Distribution, FormatVersion, IRFile};

const V4_FIXTURE: &str =
    include_str!("../../integration-tests/fixtures/ir/v4/v4-library-distribution.json");

#[test]
fn test_load_v4_distribution_fixture() {
    let ir: IRFile =
        serde_json::from_str(V4_FIXTURE).expect("Failed to parse V4 distribution fixture");

    // format_version can be 4 (integer) or "4.0.0" (string)
    match ir.format_version {
        FormatVersion::Integer(n) => assert_eq!(n, 4),
        FormatVersion::String(s) => assert!(s.starts_with("4")),
    }
}

#[test]
fn test_v4_distribution_is_library() {
    let ir: IRFile = serde_json::from_str(V4_FIXTURE).unwrap();

    match &ir.distribution {
        Distribution::Library(content) => {
            // package_name is a PackageName type which serializes as string "example/v4-test"
            assert_eq!(content.package_name.to_string(), "example/v4-test");
        }
        _ => panic!("Expected Library distribution"),
    }
}

#[test]
fn test_v4_distribution_has_modules() {
    let ir: IRFile = serde_json::from_str(V4_FIXTURE).unwrap();

    match &ir.distribution {
        Distribution::Library(content) => {
            assert!(!content.def.modules.is_empty());
            // Check for the "domain" module
            assert!(content.def.modules.contains_key("domain"));
        }
        _ => panic!("Expected Library distribution"),
    }
}

#[test]
fn test_minimal_v4_distribution_serialize_deserialize() {
    use indexmap::IndexMap;
    use morphir_core::ir::v4::{Distribution, LibraryContent, PackageDefinition};

    let dist = Distribution::Library(LibraryContent {
        package_name: "test/pkg".parse().unwrap(),
        dependencies: IndexMap::new(),
        def: PackageDefinition {
            modules: IndexMap::new(),
        },
    });

    let json = serde_json::to_string(&dist).unwrap();
    assert!(json.contains("\"Library\""));

    let parsed: Distribution = serde_json::from_str(&json).unwrap();
    match parsed {
        Distribution::Library(content) => {
            assert_eq!(content.package_name.to_string(), "test/pkg");
        }
        _ => panic!("Expected Library distribution"),
    }
}
