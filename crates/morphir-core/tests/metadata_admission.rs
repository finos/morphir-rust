use indexmap::IndexMap;
use morphir_core::data_value::DataValueValidator;
use morphir_core::ir::{classic, v4};
use morphir_core::metadata::admission::{
    Admission, AdmissionError, Interpretation, Interpreter, NodeTargetKind, ObjectDeclaration,
    PredicateClosure, PredicateDeclaration, SubjectRole, UnvalidatedReason,
};
use morphir_core::metadata::{Carrier, Fact, GraphName, ObjectTerm};
use morphir_core::naming::{FQName, Name, PackageName, Path};
use morphir_core::node_address::NodeUri;
use serde_json::json;

fn uri(value: &str) -> NodeUri {
    NodeUri::parse(value).unwrap()
}

fn subject() -> NodeUri {
    uri("morphir://ir/pkg/acme/orders?format=4.1.0#/module/api/value/legacy-submit-order")
}

fn predicate() -> NodeUri {
    uri("morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/deprecated")
}

fn validator() -> DataValueValidator {
    let distribution = v4::Distribution::Specs(v4::SpecsContent {
        package_name: PackageName::new(Path::new("acme/metadata")),
        dependencies: IndexMap::new(),
        spec: v4::PackageSpecification {
            modules: IndexMap::new(),
        },
    });
    DataValueValidator::v4(&distribution).unwrap()
}

fn bool_type() -> v4::Type {
    v4::Type::reference(
        v4::TypeAttributes::default(),
        FQName::from_canonical_string("morphir/SDK:basics#bool").unwrap(),
        vec![],
    )
}

fn string_type() -> v4::Type {
    v4::Type::reference(
        v4::TypeAttributes::default(),
        FQName::from_canonical_string("morphir/SDK:string#string").unwrap(),
        vec![],
    )
}

fn target_names_predicate() -> NodeUri {
    uri("morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/value/target-names")
}

fn target_names_type_uri() -> NodeUri {
    uri("morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/type/target-names")
}

fn target_names_type() -> v4::Type {
    v4::Type::reference(
        v4::TypeAttributes::default(),
        FQName::from_canonical_string("acme/metadata:naming#target-names").unwrap(),
        vec![],
    )
}

fn target_names_validator() -> DataValueValidator {
    let dict = v4::Type::reference(
        v4::TypeAttributes::default(),
        FQName::from_canonical_string("morphir/SDK:dict#dict").unwrap(),
        vec![string_type(), string_type()],
    );
    let definition = v4::AccessControlled {
        access: v4::Access::Public,
        value: v4::Documented::new(
            None,
            v4::TypeDefinition::TypeAliasDefinition {
                type_params: vec![],
                type_expr: v4::Type::record(
                    v4::TypeAttributes::default(),
                    vec![
                        v4::Field::new(Name::from("frontend"), dict.clone()),
                        v4::Field::new(Name::from("backend"), dict),
                    ],
                ),
            },
        ),
    };
    let distribution = v4::Distribution::Library(v4::LibraryContent {
        package_name: PackageName::new(Path::new("acme/metadata")),
        dependencies: IndexMap::new(),
        def: v4::PackageDefinition {
            modules: IndexMap::from([(
                "naming".into(),
                v4::AccessControlled {
                    access: v4::Access::Public,
                    value: v4::ModuleDefinition {
                        types: IndexMap::from([("target-names".into(), definition)]),
                        values: IndexMap::new(),
                        doc: None,
                    },
                },
            )]),
        },
    });
    DataValueValidator::v4(&distribution).unwrap()
}

fn deprecated_fact(value: serde_json::Value) -> Fact {
    Fact::new(
        subject(),
        predicate(),
        ObjectTerm::value(value),
        GraphName::Default,
    )
}

#[test]
fn declared_value_predicate_accepts_boolean_on_allowed_subject() {
    let declaration = PredicateDeclaration::value(
        predicate(),
        ObjectDeclaration::data(bool_type()),
        [
            SubjectRole::ValueSpecification,
            SubjectRole::ValueDefinition,
        ],
        Interpretation::Descriptive,
    );
    let closure = PredicateClosure::new([declaration]).unwrap();
    let fact = deprecated_fact(json!(true));
    assert_eq!(
        closure.admit(
            &fact,
            SubjectRole::ValueSpecification,
            &Carrier::AttributesFacts(subject()),
            &validator(),
            &[],
            None,
        ),
        Ok(Admission::Validated),
    );
}

#[test]
fn declared_predicate_rejects_wrong_value_and_subject_role() {
    let declaration = PredicateDeclaration::value(
        predicate(),
        ObjectDeclaration::data(bool_type()),
        [SubjectRole::ValueSpecification],
        Interpretation::Descriptive,
    );
    let closure = PredicateClosure::new([declaration]).unwrap();
    assert!(matches!(
        closure.admit(
            &deprecated_fact(json!("true")),
            SubjectRole::ValueSpecification,
            &Carrier::AttributesFacts(subject()),
            &validator(),
            &[],
            None,
        ),
        Err(AdmissionError::DataValue(_)),
    ));
    assert_eq!(
        closure.admit(
            &deprecated_fact(json!(true)),
            SubjectRole::TypeDefinition,
            &Carrier::DocumentGraph,
            &validator(),
            &[],
            None,
        ),
        Err(AdmissionError::SubjectRoleMismatch),
    );
}

#[test]
fn missing_predicate_is_preserved_without_claiming_validation() {
    let closure = PredicateClosure::new([]).unwrap();
    assert_eq!(
        closure.admit(
            &deprecated_fact(json!(true)),
            SubjectRole::ValueSpecification,
            &Carrier::AttributesFacts(subject()),
            &validator(),
            &[],
            None,
        ),
        Ok(Admission::PreservedUnvalidated(
            UnvalidatedReason::PredicateDeclaration,
        )),
    );
}

#[test]
fn closure_rejects_duplicates_and_untyped_json_datatype_addresses() {
    let declaration = || {
        PredicateDeclaration::value(
            predicate(),
            ObjectDeclaration::data(bool_type()),
            [SubjectRole::ValueSpecification],
            Interpretation::Descriptive,
        )
    };
    assert_eq!(
        PredicateClosure::new([declaration(), declaration()]).unwrap_err(),
        AdmissionError::DuplicatePredicate,
    );
    let bad = PredicateDeclaration::value(
        target_names_predicate(),
        ObjectDeclaration::json(predicate(), target_names_type()),
        [SubjectRole::ValueSpecification],
        Interpretation::Descriptive,
    );
    assert_eq!(
        PredicateClosure::new([bad]).unwrap_err(),
        AdmissionError::InvalidDatatypeDeclaration,
    );
    let old_native = PredicateDeclaration::value(
        uri("morphir://ir/pkg/acme/metadata?format=3.1.0#/module/lifecycle/value/deprecated"),
        ObjectDeclaration::data(bool_type()),
        [SubjectRole::ValueSpecification],
        Interpretation::Descriptive,
    );
    assert_eq!(
        PredicateClosure::new([old_native]).unwrap_err(),
        AdmissionError::InvalidDeclarationRole,
    );
    let wrong_format = PredicateDeclaration::value(
        target_names_predicate(),
        ObjectDeclaration::json(
            uri("morphir://ir/pkg/acme/metadata?format=3.1.0#/module/naming/type/target-names"),
            target_names_type(),
        ),
        [SubjectRole::ValueSpecification],
        Interpretation::Descriptive,
    );
    assert_eq!(
        PredicateClosure::new([wrong_format]).unwrap_err(),
        AdmissionError::InvalidDatatypeDeclaration,
    );
}

#[test]
fn value_and_sidecar_declaration_roles_are_distinct() {
    let bad = PredicateDeclaration::value(
        uri("morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/type/deprecation"),
        ObjectDeclaration::data(bool_type()),
        [SubjectRole::ValueSpecification],
        Interpretation::Descriptive,
    );
    assert_eq!(
        PredicateClosure::new([bad]).unwrap_err(),
        AdmissionError::InvalidDeclarationRole,
    );
    let v3_entry = uri("morphir://ir/pkg/elm-compat?format=3.1.0#/module/api/type/api-error");
    let v3_data: classic::Distribution =
        serde_json::from_str(include_str!("fixtures/ir/classic/greeting-example.json")).unwrap();
    let v3_validator = DataValueValidator::v3(&v3_data).unwrap();
    let closure = PredicateClosure::new([PredicateDeclaration::sidecar_type(
        v3_entry.clone(),
        ObjectDeclaration::sidecar_json(
            v3_entry.clone(),
            FQName::from_canonical_string("elm-compat:api#api-error").unwrap(),
        ),
        [SubjectRole::ValueSpecification],
    )])
    .unwrap();
    let sidecar_fact = Fact::new(
        uri("morphir://ir/pkg/acme/orders?format=3.1.0#/module/api/value/legacy-submit-order"),
        v3_entry.clone(),
        ObjectTerm::typed_json(json!(["internalError"]), v3_entry.clone()),
        GraphName::Default,
    );
    let sidecar_carrier = Carrier::Sidecar {
        target: sidecar_fact.subject().clone(),
        entry_point: v3_entry,
    };
    assert_eq!(
        closure.admit(
            &sidecar_fact,
            SubjectRole::ValueSpecification,
            &sidecar_carrier,
            &v3_validator,
            &[],
            None
        ),
        Ok(Admission::Validated),
    );
    assert_eq!(
        closure.admit(
            &sidecar_fact,
            SubjectRole::ValueSpecification,
            &Carrier::DocumentGraph,
            &v3_validator,
            &[],
            None
        ),
        Err(AdmissionError::DeclarationCarrierMismatch),
    );
}

#[test]
fn v4_sidecar_type_specification_uses_the_shared_data_validator() {
    let entry = uri("morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/type/label");
    let closure = PredicateClosure::new([PredicateDeclaration::sidecar_type(
        entry.clone(),
        ObjectDeclaration::json(entry.clone(), string_type()),
        [SubjectRole::ValueDefinition],
    )])
    .unwrap();
    let fact = Fact::new(
        subject(),
        entry.clone(),
        ObjectTerm::typed_json(json!("Legacy order"), entry.clone()),
        GraphName::Default,
    );
    let carrier = Carrier::Sidecar {
        target: subject(),
        entry_point: entry,
    };
    assert_eq!(
        closure.admit(
            &fact,
            SubjectRole::ValueDefinition,
            &carrier,
            &validator(),
            &[],
            None
        ),
        Ok(Admission::Validated),
    );
    let wrong = Fact::new(
        fact.subject().clone(),
        fact.predicate().clone(),
        ObjectTerm::typed_json(json!(42), fact.predicate().clone()),
        GraphName::Default,
    );
    assert!(matches!(
        closure.admit(
            &wrong,
            SubjectRole::ValueDefinition,
            &carrier,
            &validator(),
            &[],
            None
        ),
        Err(AdmissionError::DataValue(_)),
    ));
}

#[test]
fn node_links_require_declared_target_kind() {
    let replacement =
        uri("morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/replacement");
    let target =
        uri("morphir://ir/pkg/acme/orders-next?format=4.0.0#/module/api/value/submit-order-v2");
    let closure = PredicateClosure::new([PredicateDeclaration::value(
        replacement.clone(),
        ObjectDeclaration::node(NodeTargetKind::Value),
        [SubjectRole::ValueSpecification],
        Interpretation::Descriptive,
    )])
    .unwrap();
    let fact = Fact::new(
        subject(),
        replacement,
        ObjectTerm::NodeRef(target.clone()),
        GraphName::Default,
    );
    let carrier = Carrier::AnnotationsFacts(subject());
    assert_eq!(
        closure.admit(
            &fact,
            SubjectRole::ValueSpecification,
            &carrier,
            &validator(),
            &[],
            Some(NodeTargetKind::Value)
        ),
        Ok(Admission::Validated),
    );
    assert_eq!(
        closure.admit(
            &fact,
            SubjectRole::ValueSpecification,
            &carrier,
            &validator(),
            &[],
            None
        ),
        Ok(Admission::PreservedUnvalidated(UnvalidatedReason::Target)),
    );
    assert_eq!(
        closure.admit(
            &fact,
            SubjectRole::ValueSpecification,
            &carrier,
            &validator(),
            &[],
            Some(NodeTargetKind::Type)
        ),
        Err(AdmissionError::NodeTargetKindMismatch),
    );
    let wrong_root = Fact::new(
        subject(),
        fact.predicate().clone(),
        ObjectTerm::NodeRef(target_names_type_uri()),
        GraphName::Default,
    );
    assert_eq!(
        closure.admit(
            &wrong_root,
            SubjectRole::ValueSpecification,
            &carrier,
            &validator(),
            &[],
            None
        ),
        Err(AdmissionError::NodeTargetKindMismatch),
    );
    let value_child = Fact::new(
        subject(),
        fact.predicate().clone(),
        ObjectTerm::NodeRef(uri(
            "morphir://ir/pkg/acme/orders-next?format=4.0.0#/module/api/value/submit-order-v2/body",
        )),
        GraphName::Default,
    );
    assert_eq!(
        closure.admit(
            &value_child,
            SubjectRole::ValueSpecification,
            &carrier,
            &validator(),
            &[],
            None
        ),
        Err(AdmissionError::NodeTargetKindMismatch),
    );
    let literal = Fact::new(
        fact.subject().clone(),
        fact.predicate().clone(),
        ObjectTerm::value(json!(target.to_string())),
        GraphName::Default,
    );
    assert_eq!(
        closure.admit(
            &literal,
            SubjectRole::ValueSpecification,
            &carrier,
            &validator(),
            &[],
            None
        ),
        Err(AdmissionError::ObjectKindMismatch),
    );
}

#[test]
fn carrier_subject_mismatch_cannot_claim_valid_semantics() {
    let closure = PredicateClosure::new([PredicateDeclaration::value(
        predicate(),
        ObjectDeclaration::data(bool_type()),
        [SubjectRole::ValueSpecification],
        Interpretation::Descriptive,
    )])
    .unwrap();
    let other = uri("morphir://ir/pkg/acme/orders?format=4.1.0#/module/api/value/other-order");
    assert_eq!(
        closure.admit(
            &deprecated_fact(json!(true)),
            SubjectRole::ValueSpecification,
            &Carrier::AttributesFacts(other),
            &validator(),
            &[],
            None,
        ),
        Err(AdmissionError::CarrierSubjectMismatch),
    );
}

#[test]
fn target_names_type_check_precedes_required_interpreter() {
    let declaration = PredicateDeclaration::value(
        target_names_predicate(),
        ObjectDeclaration::json(target_names_type_uri(), target_names_type()),
        [SubjectRole::ValueSpecification],
        Interpretation::Required(Interpreter::TargetNameLanguageIds),
    );
    let closure = PredicateClosure::new([declaration]).unwrap();
    let fact = |value| {
        Fact::new(
            subject(),
            target_names_predicate(),
            ObjectTerm::typed_json(value, target_names_type_uri()),
            GraphName::Default,
        )
    };
    let carrier = Carrier::AttributesFacts(subject());
    let valid = fact(
        json!({"frontend":{"gleam":"submit_order","f-sharp":"SubmitOrder"},"backend":{"sql":"SUBMIT_ORDER"}}),
    );
    assert_eq!(
        closure.admit(
            &valid,
            SubjectRole::ValueSpecification,
            &carrier,
            &target_names_validator(),
            &[],
            None
        ),
        Ok(Admission::PreservedUnvalidated(
            UnvalidatedReason::RequiredInterpreter(Interpreter::TargetNameLanguageIds)
        )),
    );
    assert_eq!(
        closure.admit(
            &valid,
            SubjectRole::ValueSpecification,
            &carrier,
            &target_names_validator(),
            &[Interpreter::TargetNameLanguageIds],
            None
        ),
        Ok(Admission::Validated),
    );
    let malformed = fact(json!({"frontend":{"gleam":42},"backend":{"sql":"SUBMIT_ORDER"}}));
    assert!(matches!(
        closure.admit(
            &malformed,
            SubjectRole::ValueSpecification,
            &carrier,
            &target_names_validator(),
            &[],
            None
        ),
        Err(AdmissionError::DataValue(_)),
    ));
    let invalid_language =
        fact(json!({"frontend":{"C#":"SubmitOrder"},"backend":{"sql":"SUBMIT_ORDER"}}));
    assert_eq!(
        closure.admit(
            &invalid_language,
            SubjectRole::ValueSpecification,
            &carrier,
            &target_names_validator(),
            &[],
            None
        ),
        Ok(Admission::PreservedUnvalidated(
            UnvalidatedReason::RequiredInterpreter(Interpreter::TargetNameLanguageIds)
        )),
    );
    assert_eq!(
        closure.admit(
            &invalid_language,
            SubjectRole::ValueSpecification,
            &carrier,
            &target_names_validator(),
            &[Interpreter::TargetNameLanguageIds],
            None
        ),
        Err(AdmissionError::InvalidLanguageId {
            path: "$.frontend[\"C#\"]".into()
        }),
    );
    let wrong_datatype = Fact::new(
        subject(),
        target_names_predicate(),
        ObjectTerm::typed_json(
            json!({"frontend":{},"backend":{}}),
            uri("morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/type/display-info"),
        ),
        GraphName::Default,
    );
    assert_eq!(
        closure.admit(
            &wrong_datatype,
            SubjectRole::ValueSpecification,
            &carrier,
            &target_names_validator(),
            &[Interpreter::TargetNameLanguageIds],
            None
        ),
        Err(AdmissionError::DatatypeMismatch),
    );
    assert_eq!(
        closure.json_datatype(&target_names_predicate()),
        Some(target_names_type_uri()),
    );
}
