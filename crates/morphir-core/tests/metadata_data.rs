use indexmap::IndexMap;
use morphir_core::data_value::{DataValueErrorKind, DataValueValidator};
use morphir_core::ir::classic;
use morphir_core::ir::v4::{self, TypeAttributes};
use morphir_core::metadata::ObjectTerm;
use morphir_core::naming::{FQName, Name, PackageName, Path};
use morphir_core::node_address::NodeUri;
use serde_json::json;

fn reference(name: &str, arguments: Vec<v4::Type>) -> v4::Type {
    v4::Type::reference(
        TypeAttributes::default(),
        FQName::from_canonical_string(name).unwrap(),
        arguments,
    )
}

fn string() -> v4::Type {
    reference("morphir/SDK:string#string", vec![])
}

#[test]
fn metadata_sdk_int_accepts_integer_lexemes_beyond_i64() {
    let validator = DataValueValidator::v4(&definitions()).unwrap();
    let integer = reference("morphir/SDK:basics#int", vec![]);

    for lexeme in ["18446744073709551616", "-9223372036854775809"] {
        let value = serde_json::from_str(lexeme).unwrap();
        assert!(
            validator.validate_v4_data(&integer, &value).is_ok(),
            "integer lexeme {lexeme} should be accepted"
        );
    }
    for lexeme in ["1.0", "1e2"] {
        let value = serde_json::from_str(lexeme).unwrap();
        assert!(validator.validate_v4_data(&integer, &value).is_err());
    }
}

fn definitions() -> v4::Distribution {
    let dict = || reference("morphir/SDK:dict#dict", vec![string(), string()]);
    let alias = |type_expr| v4::AccessControlled {
        access: v4::Access::Public,
        value: v4::Documented::new(
            None,
            v4::TypeDefinition::TypeAliasDefinition {
                type_params: vec![],
                type_expr,
            },
        ),
    };
    let generic_alias = |type_params, type_expr| v4::AccessControlled {
        access: v4::Access::Public,
        value: v4::Documented::new(
            None,
            v4::TypeDefinition::TypeAliasDefinition {
                type_params,
                type_expr,
            },
        ),
    };
    let target_names = v4::Type::record(
        TypeAttributes::default(),
        vec![
            v4::Field::new(Name::from("frontend"), dict()),
            v4::Field::new(Name::from("backend"), dict()),
        ],
    );
    let review_state = v4::AccessControlled {
        access: v4::Access::Public,
        value: v4::Documented::new(
            None,
            v4::TypeDefinition::CustomTypeDefinition {
                type_params: vec![],
                constructors: v4::AccessControlled {
                    access: v4::Access::Public,
                    value: vec![
                        v4::ConstructorDefinition {
                            name: Name::from("pending"),
                            args: vec![],
                        },
                        v4::ConstructorDefinition {
                            name: Name::from("approved"),
                            args: vec![v4::ConstructorArg {
                                name: Name::from("reviewer"),
                                arg_type: string(),
                            }],
                        },
                    ],
                },
            },
        ),
    };
    v4::Distribution::Library(v4::LibraryContent {
        package_name: PackageName::new(Path::new("acme/metadata")),
        dependencies: IndexMap::new(),
        def: v4::PackageDefinition {
            modules: IndexMap::from([(
                "naming".into(),
                v4::AccessControlled {
                    access: v4::Access::Public,
                    value: v4::ModuleDefinition {
                        types: IndexMap::from([
                            ("target-names".into(), alias(target_names)),
                            ("review-state".into(), review_state),
                            (
                                "optional-label".into(),
                                alias(reference("morphir/SDK:maybe#maybe", vec![string()])),
                            ),
                            ("key-text".into(), alias(string())),
                            (
                                "generic-map".into(),
                                generic_alias(
                                    vec![Name::from("key")],
                                    reference(
                                        "morphir/SDK:dict#dict",
                                        vec![
                                            v4::Type::variable(
                                                TypeAttributes::default(),
                                                Name::from("key"),
                                            ),
                                            string(),
                                        ],
                                    ),
                                ),
                            ),
                            (
                                "identity".into(),
                                generic_alias(
                                    vec![Name::from("key")],
                                    v4::Type::variable(
                                        TypeAttributes::default(),
                                        Name::from("key"),
                                    ),
                                ),
                            ),
                            (
                                "loop-key".into(),
                                alias(reference("acme/metadata:naming#loop-key", vec![])),
                            ),
                        ]),
                        values: IndexMap::new(),
                        doc: None,
                    },
                },
            )]),
        },
    })
}

#[test]
fn metadata_target_names_dict_accepts_arbitrary_string_keys_and_uri_looking_data() {
    let distribution = definitions();
    let validator = DataValueValidator::v4(&distribution).unwrap();
    let ty = reference("acme/metadata:naming#target-names", vec![]);
    let value = json!({
        "frontend": {"rescript": "morphir://ir/pkg/acme/orders?format=4.0.0#/module/api/value/submit-order-v2", "f-sharp": "SubmitOrder"},
        "backend": {"sql": "SUBMIT_ORDER"}
    });
    assert!(validator.validate_v4_data(&ty, &value).is_ok());
    let err = validator
        .validate_v4_data(&ty, &json!({"frontend": {"gleam": 42}, "backend": {}}))
        .unwrap_err();
    assert_eq!(err.kind, DataValueErrorKind::Mismatch);
    assert_eq!(err.path, "$.frontend.gleam");
}

#[test]
fn metadata_dict_error_paths_quote_ambiguous_json_member_names() {
    let distribution = definitions();
    let validator = DataValueValidator::v4(&distribution).unwrap();
    let ty = reference("acme/metadata:naming#target-names", vec![]);
    for (key, expected_path) in [
        ("plain_2", "$.frontend.plain_2"),
        ("a.b", r#"$.frontend["a.b"]"#),
        ("", r#"$.frontend[""]"#),
        ("x[y]", r#"$.frontend["x[y]"]"#),
        ("a\"b\\c", r#"$.frontend["a\"b\\c"]"#),
    ] {
        let mut frontend = serde_json::Map::new();
        frontend.insert(key.to_owned(), json!(42));
        let value = json!({"frontend": frontend, "backend": {}});
        assert_eq!(
            validator.validate_v4_data(&ty, &value).unwrap_err().path,
            expected_path,
            "key {key:?}"
        );
    }
}

#[test]
fn metadata_closed_shapes_report_paths_and_unsupported_dict_keys() {
    let distribution = definitions();
    let validator = DataValueValidator::v4(&distribution).unwrap();
    let list = reference("morphir/SDK:list#list", vec![string()]);
    assert_eq!(
        validator
            .validate_v4_data(&list, &json!(["alpha", 42]))
            .unwrap_err()
            .path,
        "$[1]"
    );
    let record = v4::Type::record(
        TypeAttributes::default(),
        vec![v4::Field::new(Name::from("title"), string())],
    );
    assert_eq!(
        validator
            .validate_v4_data(&record, &json!({}))
            .unwrap_err()
            .path,
        "$"
    );
    assert!(
        validator
            .validate_v4_data(&record, &json!({"title":"Order", "extra":true}))
            .is_err()
    );
    let integer_dict = reference(
        "morphir/SDK:dict#dict",
        vec![reference("morphir/SDK:basics#int", vec![]), string()],
    );
    let err = validator
        .validate_v4_data(&integer_dict, &json!({"1":"one"}))
        .unwrap_err();
    assert_eq!(err.kind, DataValueErrorKind::UnsupportedType);
}

#[test]
fn metadata_custom_constructor_and_typed_null_keep_their_data_meaning() {
    let distribution = definitions();
    let validator = DataValueValidator::v4(&distribution).unwrap();
    let state = reference("acme/metadata:naming#review-state", vec![]);
    assert!(
        validator
            .validate_v4_data(&state, &json!(["pending"]))
            .is_ok()
    );
    assert!(
        validator
            .validate_v4_data(&state, &json!(["approved", "Ada"]))
            .is_ok()
    );
    assert_eq!(
        validator
            .validate_v4_data(&state, &json!(["approved", 42]))
            .unwrap_err()
            .path,
        "$[1]"
    );
    assert_eq!(
        validator
            .validate_v4_data(&state, &json!(["unknown"]))
            .unwrap_err()
            .kind,
        DataValueErrorKind::UnknownConstructor
    );
    let optional = reference("acme/metadata:naming#optional-label", vec![]);
    assert!(validator.validate_v4_data(&optional, &json!(null)).is_ok());
    assert!(
        validator
            .validate_v4_data(&optional, &json!("reviewed"))
            .is_ok()
    );
    assert!(
        validator
            .validate_v4_data(&v4::Type::unit(TypeAttributes::default()), &json!(null))
            .is_ok()
    );
    assert!(
        validator
            .validate_v4_data(&v4::Type::unit(TypeAttributes::default()), &json!("null"))
            .is_err()
    );
    let node: ObjectTerm = ObjectTerm::NodeRef(
        NodeUri::parse(
            "morphir://ir/pkg/acme/orders?format=4.0.0#/module/api/value/submit-order-v2",
        )
        .unwrap(),
    );
    assert_eq!(
        validator
            .validate_v4_object(&string(), &node)
            .unwrap_err()
            .kind,
        DataValueErrorKind::Mismatch
    );
}

#[test]
fn metadata_v3_sidecar_entry_point_preserves_custom_constructor_validation() {
    let distribution: classic::Distribution =
        serde_json::from_str(include_str!("fixtures/ir/classic/greeting-example.json")).unwrap();
    let validator = DataValueValidator::v3(&distribution).unwrap();
    let entry = FQName::from_canonical_string("elm-compat:api#api-error").unwrap();
    assert!(
        validator
            .validate_reference(&entry, &json!(["internalError"]))
            .is_ok()
    );
    assert!(
        validator
            .validate_reference(&entry, &json!(["invalidRequest", "bad request"]))
            .is_ok()
    );
    assert_eq!(
        validator
            .validate_reference(&entry, &json!(["invalidRequest", 42]))
            .unwrap_err()
            .path,
        "$[1]"
    );
    assert_eq!(
        validator
            .validate_reference(&entry, &json!(["unknown"]))
            .unwrap_err()
            .kind,
        DataValueErrorKind::UnknownConstructor
    );

    let classic_string = classic::Type::Reference(
        classic::Attrs::None,
        classic::FQName::new(
            classic::Path::new(vec![
                classic::Name::from_str("morphir"),
                classic::Name::from_str("SDK"),
            ]),
            classic::Path::new(vec![classic::Name::from_str("string")]),
            classic::Name::from_str("string"),
        ),
        vec![],
    );
    let classic_tuple = classic::Type::Tuple(
        classic::Attrs::None,
        vec![classic_string, classic::Type::Unit(classic::Attrs::None)],
    );
    assert!(
        validator
            .validate_v3_data(&classic_tuple, &json!(["name", null]))
            .is_ok()
    );
    assert_eq!(
        validator
            .validate_v3_data(&classic_tuple, &json!(["name", "null"]))
            .unwrap_err()
            .path,
        "$[1]"
    );
}

#[test]
fn metadata_v3_dict_string_uses_classic_sdk_package_spelling() {
    let distribution: classic::Distribution =
        serde_json::from_str(include_str!("fixtures/ir/classic/greeting-example.json")).unwrap();
    let validator = DataValueValidator::v3(&distribution).unwrap();
    let sdk = |module: &str, local: &str| {
        classic::FQName::new(
            classic::Path::new(vec![
                classic::Name::from_str("morphir"),
                classic::Name::from_str("SDK"),
            ]),
            classic::Path::new(vec![classic::Name::from_str(module)]),
            classic::Name::from_str(local),
        )
    };
    let string = classic::Type::Reference(classic::Attrs::None, sdk("string", "string"), vec![]);
    let dict = classic::Type::Reference(
        classic::Attrs::None,
        sdk("dict", "dict"),
        vec![string.clone(), string],
    );
    assert!(
        validator
            .validate_v3_data(&dict, &json!({"f-sharp": "SubmitOrder"}))
            .is_ok()
    );
    assert_eq!(
        validator
            .validate_v3_data(&dict, &json!({"f-sharp": 42}))
            .unwrap_err()
            .path,
        r#"$["f-sharp"]"#
    );
}

#[test]
fn metadata_dict_resolves_string_key_aliases_after_generic_substitution() {
    let distribution = definitions();
    let validator = DataValueValidator::v4(&distribution).unwrap();
    let map = |key| reference("acme/metadata:naming#generic-map", vec![key]);
    let alias_key = reference("acme/metadata:naming#key-text", vec![]);
    assert!(
        validator
            .validate_v4_data(&map(alias_key), &json!({"custom-id": "value"}))
            .is_ok()
    );
    let nested_identity_key = reference(
        "acme/metadata:naming#identity",
        vec![reference("acme/metadata:naming#identity", vec![string()])],
    );
    assert!(
        validator
            .validate_v4_data(&map(nested_identity_key), &json!({"nested": "value"}))
            .is_ok()
    );
    let integer_key = reference("morphir/SDK:basics#int", vec![]);
    assert_eq!(
        validator
            .validate_v4_data(&map(integer_key), &json!({"1": "one"}))
            .unwrap_err()
            .kind,
        DataValueErrorKind::UnsupportedType
    );
    let cyclic_key = reference("acme/metadata:naming#loop-key", vec![]);
    assert_eq!(
        validator
            .validate_v4_data(&map(cyclic_key), &json!({"key": "value"}))
            .unwrap_err()
            .kind,
        DataValueErrorKind::UnsupportedType
    );
}

#[test]
fn metadata_sdk_errors_distinguish_unsupported_type_from_bad_data() {
    let distribution = definitions();
    let validator = DataValueValidator::v4(&distribution).unwrap();
    let unsupported_types = [
        reference("morphir/SDK:set#set", vec![string()]),
        reference("morphir/SDK:basics#bool", vec![string()]),
        reference("morphir/SDK:list#list", vec![]),
        reference("morphir/SDK:maybe#maybe", vec![string(), string()]),
        reference("morphir/SDK:dict#dict", vec![string()]),
    ];
    for ty in unsupported_types {
        assert_eq!(
            validator
                .validate_v4_data(&ty, &json!(null))
                .unwrap_err()
                .kind,
            DataValueErrorKind::UnsupportedType
        );
    }
    assert_eq!(
        validator
            .validate_v4_data(
                &reference("morphir/SDK:basics#bool", vec![]),
                &json!("true")
            )
            .unwrap_err()
            .kind,
        DataValueErrorKind::Mismatch
    );
    assert_eq!(
        validator
            .validate_v4_data(
                &reference("morphir/SDK:list#list", vec![string()]),
                &json!([42])
            )
            .unwrap_err()
            .kind,
        DataValueErrorKind::Mismatch
    );
}
