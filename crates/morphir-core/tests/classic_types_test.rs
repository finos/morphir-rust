use morphir_core::ir::classic::naming::Name;
use morphir_core::ir::classic::types::Type;

#[test]
fn test_type_variable() {
    let json = r#"["Variable", null, ["a"]]"#;
    let t: Type<()> = serde_json::from_str(json).expect("Failed to parse Variable");
    match t {
        Type::Variable(_, name) => assert_eq!(name, Name::from_str("a")),
        _ => panic!("Expected Variable"),
    }
}

#[test]
fn test_type_reference_no_args() {
    // Reference(A, FQName, Vec<Type<A>>)
    // [ "Reference", attributes, FQName, [args] ]
    // FQName is [ [pkg], [mod], name ]

    let json = r#"["Reference", null, [[["my"],["pkg"]], [["my"],["mod"]], ["my","type"]], []]"#;
    let t: Type<()> = serde_json::from_str(json).expect("Failed to parse Reference");
    match t {
        Type::Reference(_, fqname, args) => {
            assert_eq!(fqname.package_path.segments[0], Name::from_str("my"));
            assert_eq!(fqname.package_path.segments[1], Name::from_str("pkg"));
            assert_eq!(fqname.module_path.segments[0], Name::from_str("my"));
            assert_eq!(fqname.module_path.segments[1], Name::from_str("mod"));
            assert_eq!(fqname.local_name, Name::from_str("my_type"));
            assert!(args.is_empty());
        }
        _ => panic!("Expected Reference"),
    }
}

#[test]
fn test_type_reference_with_args() {
    let json = r#"["Reference", null, [[["p"]],[["m"]],["t"]], [["Variable", null, ["a"]]]]"#;
    let t: Type<()> = serde_json::from_str(json).expect("Failed to parse Reference with args");
    match t {
        Type::Reference(_, _, args) => {
            assert_eq!(args.len(), 1);
            match &args[0] {
                Type::Variable(_, name) => assert_eq!(name, &Name::from_str("a")),
                _ => panic!("Expected Variable arg"),
            }
        }
        _ => panic!("Expected Reference"),
    }
}

#[test]
fn test_type_tuple() {
    let json = r#"["Tuple", null, [["Variable", null, ["a"]], ["Variable", null, ["b"]]]]"#;
    let t: Type<()> = serde_json::from_str(json).expect("Failed to parse Tuple");
    match t {
        Type::Tuple(_, elements) => {
            assert_eq!(elements.len(), 2);
        }
        _ => panic!("Expected Tuple"),
    }
}

#[test]
fn test_type_record() {
    // Record(A, Vec<Field<A>>)
    // Field is [name, type] or {"name":..., "tpe":...}

    // Test Array Format
    let json_array = r#"["Record", null, [ [["field", "1"], ["Variable", null, ["a"]]] ]]"#;
    let t: Type<()> =
        serde_json::from_str(json_array).expect("Failed to parse Record array format");
    match t {
        Type::Record(_, fields) => {
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].name, Name::from_str("field1"));
        }
        _ => panic!("Expected Record"),
    }

    // Test Object Format (common in some encodings)
    // Field visitor was updated to support map access but let's see if we need it here
    // based on previous errors, it seemed fields were failing.
}

#[test]
fn test_type_function() {
    let json = r#"["Function", null, ["Variable", null, ["a"]], ["Variable", null, ["b"]]]"#;
    let t: Type<()> = serde_json::from_str(json).expect("Failed to parse Function");
    match t {
        Type::Function(_, arg, ret) => {
            match *arg {
                Type::Variable(_, name) => assert_eq!(name, Name::from_str("a")),
                _ => panic!("Expected Variable arg"),
            }
            match *ret {
                Type::Variable(_, name) => assert_eq!(name, Name::from_str("b")),
                _ => panic!("Expected Variable ret"),
            }
        }
        _ => panic!("Expected Function"),
    }
}

#[test]
fn test_type_unit() {
    let json = r#"["Unit", null]"#;
    let t: Type<()> = serde_json::from_str(json).expect("Failed to parse Unit");
    match t {
        Type::Unit(_) => (),
        _ => panic!("Expected Unit"),
    }
}
