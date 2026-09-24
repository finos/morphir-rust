use morphir_common::loader::load_ir;
use serde_json::json;

#[test]
fn load_ir_normalizes_every_supported_classic_v3_release_string() {
    for version in ["3.0.0", "3.0.1", "3.1.0"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("morphir-ir.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({
                "formatVersion": version,
                "distribution": ["Library", [["local"]], [], {"modules": []}]
            }))
            .unwrap(),
        )
        .unwrap();

        let loaded = load_ir(&path).unwrap_or_else(|error| panic!("{version}: {error}"));

        assert_eq!(loaded["formatVersion"], 3, "{version}");
    }
}

#[test]
fn load_ir_does_not_normalize_unsupported_classic_version_strings() {
    for version in ["3.", "3.2.0"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("morphir-ir.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({
                "formatVersion": version,
                "distribution": ["Library", [["local"]], [], {"modules": []}]
            }))
            .unwrap(),
        )
        .unwrap();

        let error = load_ir(&path).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("Failed to parse distribution as either V4 or Classic IR"),
            "{version}: {error}"
        );
    }
}
