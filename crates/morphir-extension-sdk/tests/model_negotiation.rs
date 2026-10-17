use morphir_extension_sdk::model::{
    MODEL_ENVELOPE_VERSION, MODEL_MEP_VERSION, ModelCodec, ModelOperation, ModelTransportLimits,
    ModelTransportSupport, ModelTransportTuple, NegotiatedModelTransport,
};

fn an_offer() -> ModelTransportSupport {
    ModelTransportSupport {
        tuples: vec![ModelTransportTuple {
            operation: ModelOperation::GenerateModel,
            profile: "morphir-model".into(),
            codec: ModelCodec::IonBinary,
            envelope_version: MODEL_ENVELOPE_VERSION.into(),
            ir_version: "4.0.0".into(),
        }],
        limits: ModelTransportLimits {
            max_encoded_bytes: 1024,
            max_decoded_bytes: 768,
            max_depth: 32,
            max_nodes: 128,
        },
        guarantees: vec!["urn:example:checked".into(), "urn:example:typed".into()],
    }
}

#[test]
fn selection_intersects_whole_tuples_limits_and_guarantees() {
    let host = an_offer();
    let mut guest = an_offer();
    guest.limits.max_depth = 16;
    guest.limits.max_encoded_bytes = 2048;
    guest.guarantees.remove(0);
    let selected = NegotiatedModelTransport::select(&host, &guest).unwrap();
    assert_eq!(selected.support().tuples, host.tuples);
    assert_eq!(selected.support().limits.max_depth, 16);
    assert_eq!(selected.support().limits.max_encoded_bytes, 1024);
    assert_eq!(selected.support().guarantees, guest.guarantees);
    assert_eq!(
        NegotiatedModelTransport::from_selection(
            MODEL_MEP_VERSION,
            Some(&host),
            Some(selected.support())
        )
        .unwrap()
        .unwrap()
        .support(),
        selected.support()
    );
}

#[test]
fn crossed_tuples_do_not_form_a_cartesian_product() {
    let host = an_offer();
    for mutate in [
        |t: &mut ModelTransportTuple| t.operation = ModelOperation::CompileModel,
        |t: &mut ModelTransportTuple| t.profile = "morphir-ir".into(),
        |t: &mut ModelTransportTuple| t.codec = ModelCodec::MorphirJson,
        |t: &mut ModelTransportTuple| t.ir_version = "4.1.0".into(),
    ] {
        let mut guest = an_offer();
        mutate(&mut guest.tuples[0]);
        let selected = NegotiatedModelTransport::select(&host, &guest).unwrap();
        assert!(selected.support().tuples.is_empty());
        assert!(
            NegotiatedModelTransport::from_selection(
                MODEL_MEP_VERSION,
                Some(&host),
                Some(selected.support())
            )
            .unwrap()
            .is_none()
        );
        assert!(
            NegotiatedModelTransport::from_selection(MODEL_MEP_VERSION, Some(&host), Some(&guest))
                .is_err()
        );
    }
}

#[test]
fn rich_support_requires_exact_protocol_and_a_live_host_offer() {
    let offer = an_offer();
    assert!(
        NegotiatedModelTransport::from_selection("0.1", Some(&offer), Some(&offer))
            .unwrap()
            .is_none()
    );
    for protocol in ["0.2.0", "0.2.0-draft.2", "0.2", "invalid"] {
        assert!(
            NegotiatedModelTransport::from_selection(protocol, Some(&offer), Some(&offer)).is_err()
        );
    }
    assert!(
        NegotiatedModelTransport::from_selection(MODEL_MEP_VERSION, None, Some(&offer)).is_err()
    );
    assert!(
        NegotiatedModelTransport::from_selection(MODEL_MEP_VERSION, Some(&offer), None)
            .unwrap()
            .is_none()
    );
    assert!(
        NegotiatedModelTransport::from_selection("0.1", None, None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn malformed_support_is_not_admitted() {
    let host = an_offer();
    let mut cases = Vec::new();
    let mut invalid = an_offer();
    invalid.tuples.push(invalid.tuples[0].clone());
    cases.push(invalid);
    let mut invalid = an_offer();
    invalid.guarantees.push(invalid.guarantees[0].clone());
    cases.push(invalid);
    let mut invalid = an_offer();
    invalid.guarantees.push(" ".into());
    cases.push(invalid);
    let mut invalid = an_offer();
    invalid.limits.max_depth = 0;
    cases.push(invalid);
    let mut invalid = an_offer();
    invalid.limits.max_nodes = 2147483648;
    cases.push(invalid);
    for version in [
        "",
        "4",
        "4.0.0-draft.1",
        "4.0.0+build",
        "2.0.0",
        "4294967296.0.0",
    ] {
        let mut invalid = an_offer();
        invalid.tuples[0].ir_version = version.into();
        cases.push(invalid);
    }
    for value in ["", " ", "0.1.0", "0.1.0-draft.2"] {
        let mut invalid = an_offer();
        invalid.tuples[0].envelope_version = value.into();
        cases.push(invalid);
    }
    let mut invalid = an_offer();
    invalid.tuples[0].profile.clear();
    cases.push(invalid);
    for invalid in cases {
        assert!(
            NegotiatedModelTransport::select(&host, &invalid).is_err(),
            "accepted {invalid:?}"
        );
        assert!(NegotiatedModelTransport::select(&invalid, &host).is_err());
    }
}

#[test]
fn selected_support_cannot_raise_any_bound_or_guarantee() {
    let host = an_offer();
    for change in [
        |s: &mut ModelTransportSupport| s.limits.max_encoded_bytes += 1,
        |s: &mut ModelTransportSupport| s.limits.max_decoded_bytes += 1,
        |s: &mut ModelTransportSupport| s.limits.max_depth += 1,
        |s: &mut ModelTransportSupport| s.limits.max_nodes += 1,
        |s: &mut ModelTransportSupport| s.guarantees.push("urn:example:unknown".into()),
    ] {
        let mut selected = host.clone();
        change(&mut selected);
        assert!(
            NegotiatedModelTransport::from_selection(
                MODEL_MEP_VERSION,
                Some(&host),
                Some(&selected)
            )
            .is_err()
        );
    }
}
