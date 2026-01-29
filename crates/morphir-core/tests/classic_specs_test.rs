use morphir_core::ir::classic::naming::Name;
use morphir_core::ir::classic::types::{Type, TypeSpecification};

#[test]
fn test_type_alias_specification() {
    // ["TypeAliasSpecification", type_params, type_exp]
    let json = r#"["TypeAliasSpecification", [["a"]], ["Variable", null, ["a"]]]"#;
    let spec: TypeSpecification<()> =
        serde_json::from_str(json).expect("Failed to parse TypeAliasSpecification");

    match spec {
        TypeSpecification::Alias(params, ty) => {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0], Name::from_str("a"));
            match ty {
                Type::Variable(_, name) => assert_eq!(name, Name::from_str("a")),
                _ => panic!("Expected Variable type"),
            }
        }
        _ => panic!("Expected Alias specification"),
    }
}

#[test]
fn test_opaque_type_specification() {
    // ["OpaqueTypeSpecification", type_params]
    let json = r#"["OpaqueTypeSpecification", [["a"], ["b"]]]"#;
    let spec: TypeSpecification<()> =
        serde_json::from_str(json).expect("Failed to parse OpaqueTypeSpecification");

    match spec {
        TypeSpecification::Opaque(params) => {
            assert_eq!(params.len(), 2);
            assert_eq!(params[0], Name::from_str("a"));
        }
        _ => panic!("Expected Opaque specification"),
    }
}
