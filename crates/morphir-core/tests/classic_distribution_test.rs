use morphir_core::ir::classic::distribution::{Distribution, DistributionBody};
use morphir_core::ir::classic::package::Package;
use morphir_core::ir::classic::module::{ModuleEntry, ModuleDefinition};
use morphir_core::ir::classic::naming::{Path, Name};
use morphir_core::ir::classic::access::{AccessControlled, Access};
use std::str::FromStr;
use serde_json::json;

#[test]
fn test_deserialize_minimal_package() {
    let json = r#"{
        "modules": []
    }"#;
    let pkg: Package<serde_json::Value, serde_json::Value> = serde_json::from_str(json).expect("Failed to parse minimal package");
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
    let pkg: Package<serde_json::Value, serde_json::Value> = serde_json::from_str(json).expect("Failed to parse package with module");
    assert_eq!(pkg.modules.len(), 1);
    let entry = &pkg.modules[0];
    assert_eq!(entry.path.segments[0].words[0], "my");
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
    
    let dist: Distribution = serde_json::from_str(json).expect("Failed to parse minimal distribution");
    assert_eq!(dist.format_version, 3);
    match dist.distribution {
        DistributionBody::Library(path, deps, pkg) => {
            assert_eq!(path.segments.len(), 2);
            assert!(deps.is_empty());
            assert!(pkg.modules.is_empty());
        }
    }
}
