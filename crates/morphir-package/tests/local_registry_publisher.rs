use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_zebra::SigningKey;
use morphir_package::local_registry::*;
use morphir_package::resolution::ReleaseId;
use serde_json::{Value, json};
#[path = "local_registry/mothers.rs"]
#[allow(dead_code)]
mod mothers;
const TYPE: &str = "application/vnd.morphir.library-release.v0.1.0-draft.3+json";
fn subject() -> ObjectSubject {
    ObjectSubject {
        registry: LocalId::parse("example").unwrap(),
        path: RegistryPath::parse("statements/release.json").unwrap(),
    }
}
fn release() -> ReleaseId {
    decode_library_lock(&serde_json::to_vec(&mothers::lock()).unwrap())
        .unwrap()
        .graph()
        .root()
        .clone()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn key(i: u8) -> SigningKey {
    SigningKey::from([i; 32])
}
fn raw_key(i: u8) -> String {
    let bytes: [u8; 32] = key(i).verification_key().into();
    hex(&bytes)
}
fn policy(keys: Vec<String>, threshold: u64) -> TrustPolicy {
    decode_trust_policy(&serde_json::to_vec(&json!({"formatVersion":"0.1.0-draft.3","kind":"LibraryTrustPolicy","repositories":[],"publisherRules":[{"namespace":"example.com","publicKeys":keys,"threshold":threshold}],"continuedUse":"previous-authorization"})).unwrap()).unwrap()
}
fn pae(payload: &[u8]) -> Vec<u8> {
    let mut out = format!("DSSEv1 {} {TYPE} {} ", TYPE.len(), payload.len()).into_bytes();
    out.extend(payload);
    out
}
fn signed(payload: &[u8], keys: &[u8]) -> Value {
    json!({"payloadType":TYPE,"payload":STANDARD.encode(payload),"signatures":keys.iter().map(|i|{let bytes:[u8;64]=key(*i).sign(&pae(payload)).into();json!({"sig":STANDARD.encode(bytes)})}).collect::<Vec<_>>()})
}
fn prepare(v: Value) -> Result<PreparedPublisherEnvelope, Diagnostic> {
    prepare_publisher_envelope(&serde_json::to_vec(&v).unwrap(), &subject())
}
fn verify(v: Value, p: &TrustPolicy) -> Result<PublisherSignatureEvidence, Diagnostic> {
    verify_publisher_signatures(&prepare(v).unwrap(), &release(), p)
}
fn wire(e: Diagnostic) -> Value {
    serde_json::to_value(e).unwrap()
}
#[test]
fn publisher_counts_all_distinct_authorized_keys_and_ignores_hints() {
    let payload = "{ malformed payload λ }".as_bytes();
    let p = policy(vec![raw_key(1), raw_key(2)], 1);
    let mut e = signed(payload, &[1, 2, 1, 3]);
    for (i, s) in e["signatures"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        s["keyid"] = json!(format!("misleading λ {i}"));
        s["extension"] = json!(false)
    }
    let evidence = verify(e, &p).unwrap();
    let mut expected = vec![raw_key(1), raw_key(2)];
    expected.sort();
    assert_eq!(
        evidence
            .verified_keys()
            .iter()
            .map(PublisherKey::as_str)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(evidence.payload_bytes(), payload);
    let error = wire(
        verify(
            signed(payload, &[1, 1]),
            &policy(vec![raw_key(1), raw_key(2)], 2),
        )
        .unwrap_err(),
    );
    assert_eq!(error["witnesses"][0]["verified"], "1");
}
#[test]
fn exact_pae_binds_payload_type_lengths_and_bytes() {
    let payload = "λ".as_bytes();
    let p = policy(vec![raw_key(1)], 1);
    let mut e = signed(payload, &[1]);
    assert!(verify(e.clone(), &p).is_ok());
    e["payload"] = json!(STANDARD.encode(b"other"));
    assert!(verify(e, &p).is_err());
    for msg in [
        payload.to_vec(),
        format!("DSSEv1 0  {} λ", payload.len()).into_bytes(),
        format!("DSSEv1 {} {TYPE} 0{} λ", TYPE.len(), payload.len()).into_bytes(),
    ] {
        let mut e = signed(payload, &[]);
        let sig: [u8; 64] = key(1).sign(&msg).into();
        e["signatures"] = json!([{"sig":STANDARD.encode(sig)}]);
        assert!(verify(e, &p).is_err());
    }
}
#[test]
fn base64_forms_and_rejection_grammar() {
    let p = policy(vec![raw_key(1)], 1);
    for url in [false, true] {
        for padded in [false, true] {
            let mut e = signed(&[251, 255, 255, 254], &[1]);
            for pointer in ["/payload", "/signatures/0/sig"] {
                let mut text = e.pointer(pointer).unwrap().as_str().unwrap().to_owned();
                if url {
                    text = text.replace('+', "-").replace('/', "_")
                }
                if !padded {
                    text = text.trim_end_matches('=').into()
                }
                *e.pointer_mut(pointer).unwrap() = json!(text)
            }
            assert!(verify(e, &p).is_ok());
        }
    }
    for bad in [
        "A", "AA=", "AA===", "=AAA", "AAA==", "AA==x", " A A==", "AA==\n", "+_8=", "AB==", "AAB=",
        "AB", "AAB",
    ] {
        for pointer in ["/payload", "/signatures/0/sig"] {
            let mut e = signed(b"payload", &[1]);
            *e.pointer_mut(pointer).unwrap() = json!(bad);
            assert_eq!(
                wire(prepare(e).unwrap_err())["code"],
                "invalid-input",
                "{bad}"
            );
        }
    }
}
#[test]
fn envelope_shape_open_extensions_and_precedence() {
    for v in [
        json!(null),
        json!([]),
        json!({}),
        json!({"payloadType":4}),
        json!({"payloadType":TYPE,"payload":null,"signatures":[]}),
        json!({"payloadType":TYPE,"payload":"","signatures":null}),
        json!({"payloadType":TYPE,"payload":"","signatures":[null]}),
        json!({"payloadType":TYPE,"payload":"","signatures":[{}]}),
        json!({"payloadType":TYPE,"payload":"","signatures":[{"sig":"","keyid":null}]}),
    ] {
        assert_eq!(wire(prepare(v).unwrap_err())["code"], "invalid-input")
    }
    let unsupported = wire(
        prepare(json!({"payloadType":"future","payload":"AB==","signatures":[{}]})).unwrap_err(),
    );
    assert_eq!(unsupported["code"], "unsupported-payload-type");
    assert_eq!(unsupported["phase"], "repository");
    for text in [
        "{\"payloadType\":\"future\",\"x\":\"\\ud800\"}",
        "{\"payloadType\":\"future\",\"x\":{\"sig\":\"\",\"\\u0073ig\":\"\"}}",
        "\u{feff}{}",
    ] {
        assert_eq!(
            wire(prepare_publisher_envelope(text.as_bytes(), &subject()).unwrap_err())["code"],
            "invalid-input"
        )
    }
    let mut e = signed(b"payload", &[1]);
    e["extension"] = json!(["λ", null, true, 1.5]);
    let text = e.to_string().replace("1.5", "1.5e9999");
    let prepared = prepare_publisher_envelope(text.as_bytes(), &subject()).unwrap();
    assert!(
        verify_publisher_signatures(&prepared, &release(), &policy(vec![raw_key(1)], 1)).is_ok()
    );
}
#[test]
fn signature_limits_and_well_decoded_invalid_signatures() {
    let p = policy(vec![raw_key(1)], 1);
    for signatures in [
        json!([]),
        json!([{"sig":""}]),
        json!([{"sig":"AA=="}]),
        json!([{"sig":STANDARD.encode([0;65])}]),
        json!([{"sig":STANDARD.encode([255;64])}]),
    ] {
        let mut e = signed(b"payload", &[]);
        e["signatures"] = signatures;
        let error = wire(verify(e, &p).unwrap_err());
        assert_eq!(error["code"], "signature-invalid");
        assert_eq!(error["witnesses"][0]["verified"], "0")
    }
    let mut e = signed(b"payload", &[1]);
    let sig = e["signatures"][0].clone();
    e["signatures"] = json!(vec![sig; 64]);
    assert!(prepare(e.clone()).is_ok());
    e["signatures"] = json!(vec![json!(null); 65]);
    assert_eq!(
        wire(prepare(e).unwrap_err())["witnesses"][0]["resource"],
        "signatures"
    );
    let text = signed(b"payload", &[1]).to_string();
    let mut padded = text.clone();
    padded.extend(std::iter::repeat_n(' ', 1_048_576 - text.len()));
    assert!(prepare_publisher_envelope(padded.as_bytes(), &subject()).is_ok());
    padded.push(' ');
    assert_eq!(
        wire(prepare_publisher_envelope(padded.as_bytes(), &subject()).unwrap_err())["witnesses"]
            [0]["resource"],
        "envelope-bytes"
    );
    for depth in [64, 65] {
        let raw = format!(
            "{},\"x\":{}0{}}}",
            &text[..text.len() - 1],
            "[".repeat(depth - 1),
            "]".repeat(depth - 1)
        );
        assert_eq!(
            prepare_publisher_envelope(raw.as_bytes(), &subject()).is_ok(),
            depth == 64
        )
    }
}
#[test]
fn evidence_owns_bytes_and_does_not_authenticate_payload_identity() {
    let p = policy(vec![raw_key(1)], 1);
    let requested = release();
    let mut b = mothers::statement();
    b["release"]["packagePath"] = json!("example.com/finance/b");
    let canonical = canonical_diagnostic(&b);
    for payload in [
        b"{ malformed".to_vec(),
        canonical.as_bytes().to_vec(),
        format!("{canonical}\n").into_bytes(),
    ] {
        let original = serde_json::to_vec(&signed(&payload, &[1])).unwrap();
        let mut input = original.clone();
        let mut sub = subject();
        let prepared = prepare_publisher_envelope(&input, &sub).unwrap();
        input.fill(0);
        sub.registry = LocalId::parse("changed").unwrap();
        let evidence = verify_publisher_signatures(&prepared, &requested, &p).unwrap();
        assert_eq!(evidence.requested_release(), &requested);
        assert_eq!(evidence.subject().registry.as_str(), "example");
        assert_eq!(evidence.payload_bytes(), payload);
        assert_eq!(evidence.envelope_bytes(), original);
        assert!(
            verify_publisher_signatures(&prepared, &requested, &policy(vec![raw_key(3)], 1))
                .is_err()
        );
    }
    let mut empty = mothers::policy();
    empty["publisherRules"] = json!([]);
    let empty = decode_trust_policy(&serde_json::to_vec(&empty).unwrap()).unwrap();
    assert_eq!(
        wire(verify(signed(b"x", &[1]), &empty).unwrap_err())["code"],
        "unauthorized-publisher"
    );
}
#[test]
fn strict_points_reject_identity_keys_noncanonical_encodings_and_overflow() {
    let identity = format!("01{}", "00".repeat(31));
    let sig = STANDARD.encode([&[1u8][..], &[0u8; 63]].concat());
    for raw in [identity, format!("ee{}7f", "ff".repeat(30))] {
        let mut e = signed(b"payload", &[]);
        e["signatures"] = json!([{"sig":sig}]);
        assert_eq!(
            wire(verify(e, &policy(vec![raw], 1)).unwrap_err())["witnesses"][0]["verified"],
            "0"
        )
    }
    let mut e = signed(b"payload", &[1]);
    let mut sig = STANDARD
        .decode(e["signatures"][0]["sig"].as_str().unwrap())
        .unwrap();
    sig[32..].fill(255);
    e["signatures"][0]["sig"] = json!(STANDARD.encode(sig));
    assert!(verify(e, &policy(vec![raw_key(1)], 1)).is_err());
}
#[test]
fn canonical_identity_r_and_mixed_order_vectors_preserve_noble_strict_semantics() {
    // Frozen using the audited Noble 2.4.0 baseline. Scalars a=1 and r=0/1 are
    // public test values; these are verification-criteria probes, not publisher keys.
    let rows: Value =
        serde_json::from_str(include_str!("local_registry/crypto_vectors.json")).unwrap();
    let unhex = |s: &str| {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>()
    };
    for row in rows.as_array().unwrap() {
        let payload = unhex(row["payload"].as_str().unwrap());
        let signature = unhex(row["signature"].as_str().unwrap());
        assert_eq!(pae(&payload), unhex(row["message"].as_str().unwrap()));
        let e = json!({"payloadType":TYPE,"payload":STANDARD.encode(payload),"signatures":[{"sig":STANDARD.encode(signature)}]});
        assert!(
            verify(e, &policy(vec![row["key"].as_str().unwrap().into()], 1)).is_ok(),
            "{}",
            row["name"]
        );
    }
}
#[test]
fn requested_release_can_be_constructed_from_validated_values() {
    let path = morphir_package::resolution::PackagePath::parse("example.com/finance/a").unwrap();
    let version = morphir_package::resolution::StableVersion::parse("1.0.0").unwrap();
    let requested = ReleaseId::new(path.clone(), version.clone());
    assert_eq!(requested.package_path(), &path);
    assert_eq!(requested.version(), &version);
}
