use morphir_core::intern;
use morphir_core::ir::classic::access::Access;
use morphir_core::ir::classic::distribution::{Distribution, DistributionBody};
use morphir_core::ir::classic::package::PackageDefinition;

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
