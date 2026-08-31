use morphir_common::loader::load_ir;
use serde_json::json;

#[test]
fn load_ir_normalizes_the_exact_classic_v3_release_string() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("morphir-ir.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({
            "formatVersion": "3.0.0",
            "distribution": ["Library", [["local"]], [], {"modules": []}]
        }))
        .unwrap(),
    )
    .unwrap();

    let loaded = load_ir(&path).unwrap();

    assert_eq!(loaded["formatVersion"], 3);
}

#[test]
fn load_ir_does_not_normalize_other_classic_version_strings() {
    for version in ["3.", "3.0.1"] {
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
