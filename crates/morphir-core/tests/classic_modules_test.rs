use morphir_core::ir::classic::module::ModuleEntry;
use morphir_core::ir::classic::access::Access;
use morphir_core::ir::classic::naming::Name;
use morphir_core::intern;

#[test]
fn test_module_empty() {
    let json = r#"[
        [["my"],["mod"]],
        {
            "access": "Public",
            "value": {
                "types": [],
                "values": []
            }
        }
    ]"#;
    let entry: ModuleEntry<serde_json::Value, serde_json::Value> = serde_json::from_str(json).expect("Failed to parse empty module");
    let path = entry.path;
    let access_mod = entry.definition;
    assert_eq!(path.segments[0].words[0], intern("my"));
    assert!(matches!(access_mod.access, Access::Public));
     assert!(access_mod.value.types.is_empty());
    assert!(access_mod.value.values.is_empty());
}

#[test]
fn test_module_with_type() {
    // A module with one type alias
    // types: [ [ ["MyType"], { "access": "Public", "value": { "doc": "mydoc", "value": TypeAliasDef(...) } } ] ]
    // TypeAliasDef: ["TypeAliasDefinition", [], ["Variable",null,["int"]]]
    
    let json = r#"[
        [["info"]],
        {
            "access": "Private",
            "value": {
                "types": [
                    [
                        ["MyType"],
                        {
                            "access": "Public",
                            "value": {
                                "doc": "mydoc",
                                "value": ["TypeAliasDefinition", [], ["Variable", null, ["int"]]]
                            }
                        }
                    ]
                ],
                "values": []
            }
        }
    ]"#;

    let entry: ModuleEntry<serde_json::Value, serde_json::Value> = serde_json::from_str(json).expect("Failed to parse module with type");
    let access_mod = entry.definition;
    assert!(matches!(access_mod.access, Access::Private));
    assert_eq!(access_mod.value.types.len(), 1);
    let (name, acc_doc_def) = &access_mod.value.types[0];
    assert_eq!(name, &Name::new(vec!["MyType".to_string()]));
    assert!(matches!(acc_doc_def.access, Access::Public));
    assert_eq!(acc_doc_def.value.doc, "mydoc");
}
