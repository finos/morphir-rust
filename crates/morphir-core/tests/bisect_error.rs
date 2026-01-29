use morphir_core::ir::classic::module::ModuleEntry;
use serde_json::json;

#[test]
fn test_repro_trailing_chars() {
    // 1. Basic ModuleEntry (Empty)
    let json_empty = json!([
        [["my"],["pkg"]],
        { "access": "Public", "value": { "types": [], "values": [] } }
    ]);
    let res: Result<ModuleEntry, _> = serde_json::from_value(json_empty);
    assert!(res.is_ok(), "Empty module failed: {:?}", res.err());

    // 2. ModuleEntry with TypeAlias (Simple Variable)
    let json_alias_var = json!([
        [["my"],["pkg"]],
        { "access": "Public", "value": { 
            "types": [
                 [ ["MyType"], { "access": "Public", "value": { "doc": "", "value": ["TypeAliasDefinition", [], ["Variable", null, ["a"]]] } } ]
            ], 
            "values": [] 
        } }
    ]);
    let res: Result<ModuleEntry, _> = serde_json::from_value(json_alias_var);
    assert!(res.is_ok(), "Simple alias failed: {:?}", res.err());

    // 3. ModuleEntry with Record
    let json_record = json!([
        [["my"],["pkg"]],
        { "access": "Public", "value": { 
            "types": [
                 [ ["MyRecord"], { "access": "Public", "value": { "doc": "", "value": ["TypeAliasDefinition", [], 
                    ["Record", null, [ [["f1"], ["Variable", null, ["a"]]] ]]
                 ] } } ]
            ], 
            "values": [] 
        } }
    ]);
    let res: Result<ModuleEntry, _> = serde_json::from_value(json_record);
    assert!(res.is_ok(), "Record failed: {:?}", res.err());

    // 4. ModuleEntry with Reference (The suspect)
    // Reference: ["Reference", attrs, FQName, Args]
    let json_ref = json!([
        [["my"],["pkg"]],
        { "access": "Public", "value": { 
            "types": [
                 [ ["MyRef"], { "access": "Public", "value": { "doc": "", "value": ["TypeAliasDefinition", [], 
                    ["Reference", null, [[["p"]],[["m"]],["n"]], []]
                 ] } } ]
            ], 
            "values": [] 
        } }
    ]);
    let res: Result<ModuleEntry, _> = serde_json::from_value(json_ref);
    assert!(res.is_ok(), "Reference failed: {:?}", res.err());

    // 5. ModuleEntry with CustomTypeDefinition
    // CustomTypeDefinition: ["CustomTypeDefinition", params, AccessControlled<Vec<Constructor>>]
    // Constructor: ["C", args]
    let json_custom = json!([
        [["my"],["pkg"]],
        { "access": "Public", "value": { 
            "types": [
                 [ ["MyCustom"], { "access": "Public", "value": { "doc": "", "value": ["CustomTypeDefinition", [], 
                    { "access": "Public", "value": [ [["c"], []] ] }
                 ] } } ]
            ], 
            "values": [] 
        } }
    ]);
    let res: Result<ModuleEntry, _> = serde_json::from_value(json_custom);
    assert!(res.is_ok(), "CustomType failed: {:?}", res.err());

    // 6. ModuleEntry with Value containing Type (LetDefinition)
    // LetDefinition: ["LetDefinition", attrs, name, definition, body]
    // Definition: { "inputTypes": [...], "outputType": ..., "body": ... }
    let json_let = json!([
        [["my"],["pkg"]],
        { "access": "Public", "value": { 
            "types": [], 
            "values": [
                [ ["myVal"], { "access": "Public", "value": { "doc": "", "value": 
                    {
                         "inputTypes": [], 
                         "outputType": ["Variable", null, ["a"]], 
                         "body": ["LetDefinition", null, ["v"], 
                            { 
                                "inputTypes": [], 
                                "outputType": ["Variable", null, ["a"]], 
                                "body": ["Literal", null, ["WholeNumberLiteral", 1]] 
                            },
                            ["Variable", null, ["v"]]
                        ]
                    }
                 } } ]
            ] 
        } }
    ]);
    let res: Result<ModuleEntry, _> = serde_json::from_value(json_let);
    assert!(res.is_ok(), "LetDefinition failed: {:?}", res.err());

    // 7. Deep Nesting (Recursion Check)
    // Create a chain of Applies: Apply(null, Apply(null, f, arg), arg) ...
    let mut val = json!(["Variable", null, ["x"]]);
    for _ in 0..50 {
        val = json!(["Apply", null, val, ["Variable", null, ["y"]]]);
    }

    let json_deep = json!([
        [["my"],["pkg"]],
        { "access": "Public", "value": { 
            "types": [], 
            "values": [
                [ ["deep"], { "access": "Public", "value": { "doc": "", "value": 
                    {
                        "inputTypes": [], 
                        "outputType": ["Variable", null, ["a"]], 
                        "body": val
                    }
                 } } ]
            ] 
        } }
    ]);
    let res: Result<ModuleEntry, _> = serde_json::from_value(json_deep);
    // Explicitly check if it fails due to recursion
    if let Err(e) = &res {
        eprintln!("Deep recursion error: {}", e);
    }
    assert!(res.is_ok(), "Deep nesting failed: {:?}", res.err());
}
