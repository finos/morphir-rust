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
