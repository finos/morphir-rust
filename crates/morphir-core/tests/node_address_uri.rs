use morphir_core::ir::v4::{Type, TypeAttributes};
use morphir_core::naming::{Name, Path};
use morphir_core::node_address::{
    ArtifactRevision, ArtifactSelector, IrFormatVersion, NodeFingerprintBuilder, NodeOwner,
    NodeRoot, NodeStep, NodeUri, NodeUriError, Sha256Digest,
};

#[test]
fn parent_draft_uri_corpus_is_executable_here() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/node-addresses-uri-draft.json")).unwrap();
    for case in corpus["uriCases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let input = case["uri"].as_str().unwrap();
        match case["expectedCanonicalUri"].as_str() {
            Some(canonical) => assert_eq!(
                NodeUri::parse(input).unwrap().to_string(),
                canonical,
                "{id}"
            ),
            None => assert!(NodeUri::parse(input).is_err(), "{id} accepted {input}"),
        }
    }
}

#[test]
fn every_child_role_has_one_parseable_spelling() {
    let name = Name::from("customerId");
    let steps = vec![
        NodeStep::TypeExpression,
        NodeStep::Body,
        NodeStep::ValueInputType(name.clone()),
        NodeStep::ValueInputAnnotation(name.clone()),
        NodeStep::ValueOutputType,
        NodeStep::DerivedBaseType,
        NodeStep::PartialTypeExpression,
        NodeStep::CustomConstructor(name.clone()),
        NodeStep::ConstructorArgument(2),
        NodeStep::RecordField(name.clone()),
        NodeStep::ExtensibleRecordField(name.clone()),
        NodeStep::TypeFunctionParameter,
        NodeStep::TypeFunctionResult,
        NodeStep::ReferenceArgument(2),
        NodeStep::ApplyFunction,
        NodeStep::ApplyArgument,
        NodeStep::FieldSubject,
        NodeStep::DestructurePattern,
        NodeStep::DestructureValue,
        NodeStep::DestructureBody,
        NodeStep::IfCondition,
        NodeStep::IfThen,
        NodeStep::IfElse,
        NodeStep::LambdaPattern,
        NodeStep::LambdaBody,
        NodeStep::LetDefinition(name.clone()),
        NodeStep::LetBody,
        NodeStep::ListElement(2),
        NodeStep::PatternMatchSubject,
        NodeStep::TupleElement(2),
        NodeStep::PatternMatchCasePattern(2),
        NodeStep::PatternMatchCaseBody(2),
        NodeStep::UpdateSubject,
        NodeStep::UpdateField(name.clone()),
        NodeStep::AsPatternChild,
        NodeStep::PatternTupleElement(2),
        NodeStep::PatternConstructorArgument(2),
        NodeStep::HeadTailHead,
        NodeStep::HeadTailTail,
        NodeStep::ExternalFallback,
        NodeStep::IncompletePartialBody,
        NodeStep::HoleExpectedType,
        NodeStep::InferredType,
        NodeStep::AnnotationEntry(2),
    ];
    for step in steps {
        let address = NodeUri::new(
            ArtifactSelector::Workspace("orders".to_owned()),
            IrFormatVersion::new(4, 0, 0),
            NodeRoot::Type {
                owner: NodeOwner::OwnPackage,
                module: Path::new("domain"),
                name: Name::from("order"),
            },
            vec![step],
            ArtifactRevision::Pinned(
                Sha256Digest::parse(&format!("sha256:{}", "a".repeat(64))).unwrap(),
            ),
            None,
        )
        .unwrap();
        let uri = address.to_string();
        assert_eq!(NodeUri::parse(&uri).unwrap(), address, "{uri}");
    }
}

#[test]
fn annotation_argument_address_round_trips_under_its_entry() {
    let uri = "morphir://ir/pkg/acme/orders?format=4.1.0&rev=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa#/module/api/value/submit-order/annotation/entry/2/argument/1";
    let parsed = NodeUri::parse(uri).unwrap();
    assert_eq!(parsed.to_string(), uri);
    assert_eq!(
        parsed.steps(),
        &[
            NodeStep::AnnotationEntry(2),
            NodeStep::AnnotationArgument(1)
        ]
    );
}

#[test]
fn named_v3_and_v4_nodes_round_trip() {
    for uri in [
        "morphir://ir/pkg/acme/orders?format=4.0.0#/module/domain/type/order/type-exp/record/field/customer-id",
        "morphir://ir/pkg/acme/orders?format=3.0.0#/module/domain/value/calculate-total/body/apply/argument",
        "morphir://ir/pkg/acme/orders?format=4.0.0#/module/sales%2Forders/type/order",
        "morphir://ir/workspace/team-orders-2?format=4.0.0#/module/domain/value/calculate-total",
        "morphir://ir/pkg/acme/orders?format=4.0.0#/dependency/morphir%2FSDK/module/basics/type/int",
        "morphir://ir/pkg/acme/orders?format=4.0.0#/entry-point/launch-%C3%A9",
    ] {
        let address = NodeUri::parse(uri).unwrap();
        assert_eq!(address.to_string(), uri);
    }
}

#[test]
fn positional_steps_require_a_guard_without_revision() {
    let path = "#/module/domain/type/pair/type-exp/tuple/element/1";
    let base = "morphir://ir/pkg/acme/orders?format=4.0.0";
    assert_eq!(
        NodeUri::parse(&format!("{base}{path}")),
        Err(NodeUriError::MissingGuard)
    );
    let guarded = format!("{base}&guard=sha256:{}{path}", "b".repeat(64));
    assert_eq!(NodeUri::parse(&guarded).unwrap().to_string(), guarded);
    let pinned = format!("{base}&rev=sha256:{}{path}", "a".repeat(64));
    assert_eq!(NodeUri::parse(&pinned).unwrap().to_string(), pinned);
}

#[test]
fn invalid_spellings_are_refused() {
    for uri in [
        "morphir://ir/pkg/acme/orders?format=4#/module/domain/type/order",
        "morphir://ir/pkg/acme/orders?format=4.0.0&format=3.0.0#/module/domain/type/order",
        "morphir://ir/pkg/acme/orders?format=4.0.0#/module/sales%2forders/type/order",
        "morphir://ir/pkg/acme/orders?format=4.0.0#/module/domain/type/caf%C3%A9",
        "morphir://ir/pkg/acme/orders?format=4.0.0#/module/domain/type/pair/type-exp/tuple/element/01",
        "morphir://ir/workspace/Orders?format=4.0.0#/module/domain/value/calculate-total",
        "morphir://ir/workspace/caf%C3%A9?format=4.0.0#/module/domain/value/calculate-total",
        "morphir://ir/workspace/team%2Forders?format=4.0.0#/module/domain/value/calculate-total",
        "morphir://ir/workspace/ord%65rs?format=4.0.0#/module/domain/value/calculate-total",
        "morphir://ir/pkg/acme/orders?format=4.0.0&latest=true#/module/domain/type/order",
    ] {
        assert!(NodeUri::parse(uri).is_err(), "accepted {uri}");
    }
}

#[test]
fn fingerprint_tracks_only_selected_ordered_lineage() {
    let selected = Type::unit(TypeAttributes::default());
    assert_eq!(serde_json::to_string(&selected).unwrap(), r#"{"Unit":{}}"#);
    let mut a = NodeFingerprintBuilder::new();
    a.push(&NodeStep::TupleElement(1), &selected).unwrap();
    let digest = a.finish().to_string();
    assert_eq!(
        digest,
        "sha256:05709fcf331bd4125b9a2121fac18712390e8572bb545cd091be14bd925ffe92"
    );

    let mut same = NodeFingerprintBuilder::new();
    same.push(&NodeStep::TupleElement(1), &selected).unwrap();
    assert_eq!(same.finish().to_string(), digest);

    let mut shifted = NodeFingerprintBuilder::new();
    shifted.push(&NodeStep::TupleElement(2), &selected).unwrap();
    assert_ne!(shifted.finish().to_string(), digest);

    let mut changed = NodeFingerprintBuilder::new();
    changed
        .push(
            &NodeStep::TupleElement(1),
            &Type::variable(
                TypeAttributes::default(),
                morphir_core::naming::Name::from("a"),
            ),
        )
        .unwrap();
    assert_ne!(changed.finish().to_string(), digest);
}

#[test]
fn document_literal_payload_is_not_treated_as_ir_attributes() {
    use morphir_core::ir::v4::{Literal, Value, ValueAttributes};
    let fingerprint = |payload| {
        let child = Value::Literal(ValueAttributes::default(), Literal::Document(payload));
        let mut builder = NodeFingerprintBuilder::new();
        builder.push(&NodeStep::ListElement(0), &child).unwrap();
        builder.finish()
    };
    let first = fingerprint(serde_json::json!({"attributes":{"source":"first"}}));
    let second = fingerprint(serde_json::json!({"attributes":{"source":"second"}}));
    assert_ne!(first, second);
}

#[test]
fn linked_facts_do_not_change_value_or_pattern_shorthand_guards() {
    use morphir_core::ir::v4::{Literal, MetadataScope, Pattern, Value, ValueAttributes};

    let facts = MetadataScope::parse(
        None,
        Some(&serde_json::json!({
            "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/deprecated": true
        })),
    )
    .unwrap();
    fn fingerprint<T: serde::Serialize>(step: &NodeStep, child: &T) -> Sha256Digest {
        let mut builder = NodeFingerprintBuilder::for_v4_1();
        builder.push(step, child).unwrap();
        builder.finish()
    }
    let plain = ValueAttributes::default();
    let with_facts = ValueAttributes {
        metadata: facts,
        ..ValueAttributes::default()
    };

    assert_eq!(
        fingerprint(
            &NodeStep::ListElement(0),
            &Value::variable(plain.clone(), Name::from("x"))
        ),
        fingerprint(
            &NodeStep::ListElement(0),
            &Value::variable(with_facts.clone(), Name::from("x"))
        ),
    );
    assert_eq!(
        fingerprint(
            &NodeStep::ListElement(0),
            &Value::list(
                plain.clone(),
                vec![Value::variable(plain.clone(), Name::from("x"))]
            )
        ),
        fingerprint(
            &NodeStep::ListElement(0),
            &Value::list(
                plain.clone(),
                vec![Value::variable(with_facts.clone(), Name::from("x"))]
            )
        ),
    );
    assert_eq!(
        fingerprint(
            &NodeStep::ListElement(0),
            &Value::Literal(plain.clone(), Literal::string("x"))
        ),
        fingerprint(
            &NodeStep::ListElement(0),
            &Value::Literal(with_facts.clone(), Literal::string("x"))
        ),
    );
    assert_eq!(
        fingerprint(
            &NodeStep::ListElement(0),
            &Value::Tuple(plain.clone(), vec![])
        ),
        fingerprint(
            &NodeStep::ListElement(0),
            &Value::Tuple(with_facts.clone(), vec![])
        ),
    );
    assert_eq!(
        fingerprint(
            &NodeStep::PatternTupleElement(0),
            &Pattern::tuple(plain.clone(), vec![Pattern::unit(plain.clone())])
        ),
        fingerprint(
            &NodeStep::PatternTupleElement(0),
            &Pattern::tuple(with_facts.clone(), vec![Pattern::unit(with_facts.clone())])
        ),
    );
    assert_eq!(
        fingerprint(
            &NodeStep::PatternTupleElement(0),
            &Pattern::literal(plain, Literal::string("x"))
        ),
        fingerprint(
            &NodeStep::PatternTupleElement(0),
            &Pattern::literal(with_facts, Literal::string("x"))
        ),
    );
}

#[test]
fn linked_facts_do_not_change_type_variable_guards() {
    use morphir_core::ir::v4::MetadataScope;
    let with_facts = TypeAttributes {
        metadata: MetadataScope::parse(
            None,
            Some(&serde_json::json!({
                "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/deprecated": true
            })),
        )
        .unwrap(),
        ..TypeAttributes::default()
    };
    let fingerprint = |attrs| {
        let mut builder = NodeFingerprintBuilder::for_v4_1();
        builder
            .push(
                &NodeStep::TupleElement(0),
                &Type::variable(attrs, Name::from("x")),
            )
            .unwrap();
        builder.finish()
    };
    assert_eq!(
        fingerprint(TypeAttributes::default()),
        fingerprint(with_facts)
    );
}

#[test]
fn linked_facts_in_pattern_match_cases_do_not_change_guard() {
    use morphir_core::ir::v4::{MetadataScope, Pattern, PatternCase, Value, ValueAttributes};
    let with_facts = ValueAttributes {
        metadata: MetadataScope::parse(
            None,
            Some(&serde_json::json!({
                "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/deprecated": true
            })),
        )
        .unwrap(),
        ..ValueAttributes::default()
    };
    let value = |attributes: ValueAttributes| {
        Value::PatternMatch(
            ValueAttributes::default(),
            Box::new(Value::variable(ValueAttributes::default(), Name::from("x"))),
            vec![PatternCase(
                Pattern::tuple(attributes.clone(), vec![]),
                Value::variable(attributes, Name::from("x")),
            )],
        )
    };
    let fingerprint = |child| {
        let mut builder = NodeFingerprintBuilder::for_v4_1();
        builder.push(&NodeStep::ListElement(0), &child).unwrap();
        builder.finish()
    };
    assert_eq!(
        fingerprint(value(ValueAttributes::default())),
        fingerprint(value(with_facts))
    );
}

#[test]
fn constraints_with_nested_attributes_remain_semantic() {
    let fingerprint = |fact| {
        let mut attrs = TypeAttributes::default();
        attrs.constraints.insert(
            "custom".to_owned(),
            serde_json::json!({"attributes":{"facts":fact}}),
        );
        let mut builder = NodeFingerprintBuilder::for_v4_1();
        builder
            .push(&NodeStep::TupleElement(0), &Type::unit(attrs))
            .unwrap();
        builder.finish()
    };
    assert_ne!(fingerprint(1), fingerprint(2));
}

#[test]
fn a_record_field_named_attributes_has_its_metadata_filtered() {
    use morphir_core::ir::v4::{MetadataScope, RecordFieldEntry, Value, ValueAttributes};
    let fingerprint = |metadata| {
        let child = Value::Record(
            ValueAttributes::default(),
            vec![RecordFieldEntry::new(
                Name::from("attributes"),
                Value::variable(
                    ValueAttributes {
                        metadata,
                        ..ValueAttributes::default()
                    },
                    Name::from("x"),
                ),
            )],
        );
        let mut builder = NodeFingerprintBuilder::for_v4_1();
        builder.push(&NodeStep::ListElement(0), &child).unwrap();
        builder.finish()
    };
    let facts = MetadataScope::parse(
        None,
        Some(&serde_json::json!({
            "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/deprecated": true
        })),
    )
    .unwrap();
    assert_eq!(fingerprint(MetadataScope::default()), fingerprint(facts));
}

#[test]
fn boxed_value_has_the_same_fingerprint_as_its_unboxed_value() {
    use morphir_core::ir::v4::{MetadataScope, Value, ValueAttributes};
    let child = Value::variable(ValueAttributes {
        metadata: MetadataScope::parse(None, Some(&serde_json::json!({
            "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/deprecated": true
        }))).unwrap(),
        ..ValueAttributes::default()
    }, Name::from("x"));
    let mut direct = NodeFingerprintBuilder::for_v4_1();
    direct.push(&NodeStep::ListElement(0), &child).unwrap();
    let mut boxed = NodeFingerprintBuilder::for_v4_1();
    boxed
        .push(&NodeStep::ListElement(0), &Box::new(child))
        .unwrap();
    assert_eq!(direct.finish(), boxed.finish());
}

#[test]
fn legacy_let_definition_guard_keeps_expanded_nested_type_variable() {
    use morphir_core::ir::v4::{Value, ValueAttributes, ValueDefinition};
    let child = Value::LetDefinition(
        ValueAttributes::default(),
        Name::from("bound"),
        Box::new(ValueDefinition::new(
            vec![],
            Type::variable(TypeAttributes::default(), Name::from("a")),
            Value::unit(ValueAttributes::default()),
        )),
        Box::new(Value::variable(
            ValueAttributes::default(),
            Name::from("bound"),
        )),
    );
    let mut builder = NodeFingerprintBuilder::new();
    builder.push(&NodeStep::ListElement(0), &child).unwrap();
    assert_eq!(
        builder.finish().to_string(),
        "sha256:fa836ae00372d3b91e5249c4193db79e4f790631c465c1f8f27bf8eb3f47899d"
    );
}

#[test]
fn source_only_legacy_guard_keeps_its_expanded_value_shape() {
    use morphir_core::ir::v4::{SourceLocation, Value, ValueAttributes};
    let plain = Value::variable(ValueAttributes::default(), Name::from("x"));
    let sourced = Value::variable(
        ValueAttributes {
            source: Some(SourceLocation::new(1, 1, 1, 2)),
            ..ValueAttributes::default()
        },
        Name::from("x"),
    );
    let fingerprint = |child: &Value, linked| {
        let mut builder = if linked {
            NodeFingerprintBuilder::for_v4_1()
        } else {
            NodeFingerprintBuilder::new()
        };
        builder.push(&NodeStep::ListElement(0), child).unwrap();
        builder.finish()
    };
    assert_ne!(fingerprint(&plain, false), fingerprint(&sourced, false));
    assert_eq!(
        fingerprint(&sourced, false).to_string(),
        "sha256:dba0b5032720c0071bdbe85f20a09ac1b9cff47b3e0c7503eb4d7e0f878c1efa"
    );
    assert_eq!(fingerprint(&plain, true), fingerprint(&sourced, true));
}

#[test]
fn fingerprint_ignores_ambient_v4_type_serialization_mode() {
    use morphir_core::ir::v4::{TypeEncoding, with_type_encoding};
    let child = Type::variable(TypeAttributes::default(), Name::from("item"));
    let fingerprint = || {
        let mut builder = NodeFingerprintBuilder::new();
        builder.push(&NodeStep::TupleElement(0), &child).unwrap();
        builder.finish()
    };
    assert_eq!(
        fingerprint(),
        with_type_encoding(TypeEncoding::Compact, fingerprint)
    );
}
