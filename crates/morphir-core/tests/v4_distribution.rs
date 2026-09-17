//! V4 Distribution loading tests
//!
//! Tests for loading V4 format distributions with V4-specific constructs.

use morphir_core::ir::v4::{Distribution, FormatVersion, IRFile};

const V4_FIXTURE: &str = include_str!("fixtures/ir/v4/v4-library-distribution.json");

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

/// The canonical document of MCK distributions-0010.
const APPLICATION_WITH_DEFINITION_DEPENDENCY: &str = r#"{ "formatVersion": 4, "distribution": { "Application": { "packageName": "example", "dependencies": { "my-org/shared": { "modules": { "util": { "Public": { "types": {}, "values": { "identity": { "Public": { "ExpressionBody": { "inputTypes": { "x": "morphir/SDK:basics#int" }, "outputType": "morphir/SDK:basics#int", "body": { "Variable": "x" } } } } } } } } } }, "def": { "modules": { "main": { "Public": { "types": {}, "values": { "run": { "Public": { "ExpressionBody": { "inputTypes": {}, "outputType": "morphir/SDK:basics#unit", "body": { "Unit": {} } } } } } } } } }, "entryPoints": { "start": { "target": "example:main#run", "kind": "main" } } } } }"#;

/// The canonical document of MCK distributions-0007, whose dependencies are empty.
const APPLICATION_WITHOUT_DEPENDENCIES: &str = r#"{ "formatVersion": 4, "distribution": { "Application": { "packageName": "example", "dependencies": {}, "def": { "modules": { "main": { "Public": { "types": {}, "values": { "run": { "Public": { "ExpressionBody": { "inputTypes": {}, "outputType": "morphir/SDK:basics#unit", "body": { "Unit": {} } } } } } } } } }, "entryPoints": { "start": { "target": "example:main#run", "kind": "main", "doc": "The application entry point" }, "build": { "target": "example:main#run", "kind": "command" } } } } }"#;

/// Decodes a document and writes it back in its canonical spelling, byte for byte.
fn round_trips(canonical: &str) -> IRFile {
    let expected: serde_json::Value = serde_json::from_str(canonical).unwrap();
    let file: IRFile = serde_json::from_str(canonical).expect("the canonical document decodes");
    let written = morphir_core::ir::v4::with_type_encoding(
        morphir_core::ir::v4::TypeEncoding::Compact,
        || serde_json::to_value(&file).unwrap(),
    );
    assert_eq!(
        serde_json::to_string(&written).unwrap(),
        serde_json::to_string(&expected).unwrap()
    );
    file
}

#[test]
fn an_applications_dependencies_are_package_definitions() {
    let file = round_trips(APPLICATION_WITH_DEFINITION_DEPENDENCY);
    let Distribution::Application(content) = &file.distribution else {
        panic!("expected an Application distribution");
    };
    let dependency = content
        .dependencies
        .get("my-org/shared")
        .expect("the dependency is keyed by its canonical package name");
    assert_eq!(dependency.modules.len(), 1);
    let module = dependency
        .modules
        .get("util")
        .expect("the dependency carries its util module");
    assert_eq!(module.access, morphir_core::ir::v4::Access::Public);
    assert!(module.value.values.contains_key("identity"));
}

#[test]
fn an_application_without_dependencies_round_trips() {
    let file = round_trips(APPLICATION_WITHOUT_DEPENDENCIES);
    let Distribution::Application(content) = &file.distribution else {
        panic!("expected an Application distribution");
    };
    assert!(content.dependencies.is_empty());
}

#[test]
fn a_library_dependency_is_a_specification_not_a_definition() {
    let written = serde_json::json!({
        "Library": {
            "packageName": "example",
            "dependencies": {
                "my-org/shared": {
                    "modules": {
                        "util": { "Public": { "types": {}, "values": {} } }
                    }
                }
            },
            "def": { "modules": {} }
        }
    });

    let error = serde_json::from_value::<Distribution>(written).unwrap_err();
    let diagnostic = morphir_core::ir::Diagnostic::from_serde_error(&error)
        .expect("a dependency refusal carries a diagnostic");
    assert_eq!(
        diagnostic.code,
        morphir_core::ir::DiagnosticCode::UnknownMember
    );
}

#[test]
fn distribution_wrapper_rejects_extra_entries() {
    let source = serde_json::json!({
        "Library": {
            "packageName": "example",
            "dependencies": {},
            "def": { "modules": {} }
        },
        "Specs": {
            "packageName": "example",
            "dependencies": {},
            "spec": { "modules": {} }
        }
    });

    let error = serde_json::from_value::<Distribution>(source).unwrap_err();
    let diagnostic = morphir_core::ir::Diagnostic::from_serde_error(&error)
        .expect("a distribution refusal carries a diagnostic");
    assert_eq!(
        diagnostic.code,
        morphir_core::ir::DiagnosticCode::InvalidDistributionShape
    );
}
