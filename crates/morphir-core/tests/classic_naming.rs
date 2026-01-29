use morphir_core::ir::classic::naming::{Name, Path, FQName};

#[test]
fn test_classic_name_serialization() {
    let name = Name::from_str("camel_case");
    let json = serde_json::to_string(&name).unwrap();
    assert_eq!(json, r#"["camel","case"]"#);

    let deserialized: Name = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, name);
}

#[test]
fn test_classic_path_serialization() {
    let path = Path::new(vec![
        Name::from_str("morphir"),
        Name::from_str("sdk"),
    ]);
    let json = serde_json::to_string(&path).unwrap();
    assert_eq!(json, r#"[["morphir"],["sdk"]]"#);

    let deserialized: Path = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, path);
}

#[test]
fn test_classic_fqname_serialization() {
    let pkg = Path::new(vec![Name::from_str("my_pkg")]);
    let mod_ = Path::new(vec![Name::from_str("my_mod")]);
    let name = Name::from_str("my_type");
    let fqname = FQName::new(pkg, mod_, name);

    let json = serde_json::to_string(&fqname).unwrap();
    // [package, module, name]
    assert_eq!(json, r#"[[["my","pkg"]],[["my","mod"]],["my","type"]]"#);

    let deserialized: FQName = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized, fqname);
}
