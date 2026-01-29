use morphir_core::ir::classic::types::{TypeSpecification, Type};
use morphir_core::ir::classic::naming::Name;
use serde_json::json;
use std::str::FromStr;

#[test]
fn test_type_alias_definition() {
    // ["TypeAliasDefinition", type_params, type_exp]
    let json = r#"["TypeAliasDefinition", [["a"]], ["Variable", null, ["a"]]]"#;
    let spec: TypeSpecification<()> = serde_json::from_str(json).expect("Failed to parse TypeAliasDefinition");
    
    match spec {
        TypeSpecification::TypeAliasDefinition(params, tpe) => {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0], Name::from_str("a"));
            match tpe {
                 Type::Variable(_, name) => assert_eq!(name, Name::from_str("a")),
                 _ => panic!("Expected Variable type"),
            }
        }
        _ => panic!("Expected TypeAliasDefinition"),
    }
}

#[test]
fn test_opaque_type_definition() {
    // ["OpaqueTypeDefinition", type_params]
    let json = r#"["OpaqueTypeDefinition", [["a"], ["b"]]]"#;
    let spec: TypeSpecification<()> = serde_json::from_str(json).expect("Failed to parse OpaqueTypeDefinition");
    
    match spec {
        TypeSpecification::OpaqueTypeDefinition(params) => {
            assert_eq!(params.len(), 2);
            assert_eq!(params[0], Name::from_str("a"));
        }
        _ => panic!("Expected OpaqueTypeDefinition"),
    }
}
