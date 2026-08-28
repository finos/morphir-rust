use morphir_core::intern;
use morphir_core::ir::classic::access::Access;
use morphir_core::ir::classic::distribution::{Distribution, DistributionBody};
use morphir_core::ir::classic::package::{PackageDefinition, PackageSpecification};

#[test]
fn test_deserialize_minimal_package() {
    let json = r#"{
        "modules": []
    }"#;
    let pkg: PackageDefinition<serde_json::Value, serde_json::Value> =
        serde_json::from_str(json).expect("Failed to parse minimal package");
    assert!(pkg.modules.is_empty());
}

#[test]
fn test_deserialize_package_with_one_module() {
    // ModuleEntry is structural [Path, AccessControlled<ModuleDefinition>]
    // ModuleDefinition has {types: [], values: []}
    let json = r#"{
        "modules": [
            [
                [["my"],["mod"]],
                {
                    "access": "Public",
                    "value": {
                        "types": [],
                        "values": []
                    }
                }
            ]
        ]
    }"#;
    let pkg: PackageDefinition<serde_json::Value, serde_json::Value> =
        serde_json::from_str(json).expect("Failed to parse package with module");
    assert_eq!(pkg.modules.len(), 1);
    let entry = &pkg.modules[0];
    assert_eq!(entry.path.segments[0].words[0], intern("my"));
    assert!(matches!(entry.definition.access, Access::Public));
}

#[test]
fn test_deserialize_minimal_distribution() {
    let json = r#"{
        "formatVersion": 3,
        "distribution": [
            "Library",
            [["my"],["pkg"]],
            [],
            {
                "modules": []
            }
        ]
    }"#;

    let dist: Distribution =
        serde_json::from_str(json).expect("Failed to parse minimal distribution");
    assert_eq!(dist.format_version, 3);
    match dist.distribution {
        DistributionBody::Library(path, deps, pkg) => {
            assert_eq!(path.segments.len(), 2);
            assert!(deps.is_empty());
            assert!(pkg.modules.is_empty());
        }
    }
}

#[test]
fn module_entry_serializes_in_canonical_distribution_shape_and_round_trips() {
    let canonical = serde_json::json!({
        "formatVersion": 3,
        "distribution": [
            "Library",
            [["my"], ["package"]],
            [],
            {
                "modules": [
                    [
                        [["my"], ["module"]],
                        {
                            "access": "Public",
                            "value": {
                                "types": [],
                                "values": [],
                                "doc": "Module documentation"
                            }
                        }
                    ]
                ]
            }
        ]
    });

    let distribution: Distribution =
        serde_json::from_value(canonical.clone()).expect("canonical distribution should parse");
    let serialized =
        serde_json::to_value(&distribution).expect("typed distribution should serialize");

    assert!(serialized.get("formatVersion").is_some());
    assert_eq!(serialized["distribution"][0], "Library");
    assert!(serialized["distribution"][3]["modules"][0].is_array());
    assert_eq!(serialized, canonical);

    let round_tripped: Distribution =
        serde_json::from_value(serialized).expect("serialized distribution should parse");
    assert_eq!(round_tripped, distribution);
}

#[test]
fn module_spec_entry_serializes_as_canonical_array_and_round_trips() {
    let canonical = serde_json::json!({
        "modules": [
            [
                [["dependency"], ["module"]],
                {
                    "types": [],
                    "values": [],
                    "doc": "Public dependency interface"
                }
            ]
        ]
    });

    let specification: PackageSpecification<serde_json::Value> =
        serde_json::from_value(canonical.clone())
            .expect("canonical package specification should parse");
    let serialized =
        serde_json::to_value(&specification).expect("typed package specification should serialize");

    assert!(serialized["modules"][0].is_array());
    assert_eq!(serialized["modules"][0].as_array().unwrap().len(), 2);
    assert_eq!(serialized, canonical);

    let round_tripped: PackageSpecification<serde_json::Value> =
        serde_json::from_value(serialized).expect("serialized package specification should parse");
    assert_eq!(round_tripped, specification);
}
