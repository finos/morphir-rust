use morphir_package::local_registry::*;
use morphir_package::resolution::PackagePath;
use serde_json::{Value, json};
#[path = "local_registry/mothers.rs"]
#[allow(dead_code)]
mod mothers;
fn parse(v: Value) -> Result<TrustPolicy, Diagnostic> {
    decode_trust_policy(&serde_json::to_vec(&v).unwrap())
}
fn wire(e: Diagnostic) -> Value {
    serde_json::to_value(e).unwrap()
}
#[test]
fn namespaces_match_components_and_longest_rule_without_pooling() {
    let policy = parse(mothers::policy()).unwrap();
    let identity = Digest::parse(&mothers::digest()).unwrap();
    for name in [
        "example.com/finance",
        "example.com/finance/eligibility",
        "example.com/finance-other",
        "other.example.com/finance",
        "example.com.evil/finance",
    ] {
        assert_eq!(
            repository_permits(&policy, &identity, &PackagePath::parse(name).unwrap()),
            matches!(
                name,
                "example.com/finance" | "example.com/finance/eligibility"
            )
        );
    }
    let rule = matching_publisher_rule(
        &policy,
        &PackagePath::parse("example.com/finance/eligibility").unwrap(),
    )
    .unwrap();
    assert_eq!(rule.namespace().as_str(), "example.com/finance");
    assert!(!publisher_threshold_met(
        Some(rule),
        &[PublisherKey::parse(&"0".repeat(64)).unwrap()]
    ));
    assert!(publisher_threshold_met(
        Some(rule),
        &[PublisherKey::parse(&"1".repeat(64)).unwrap()]
    ));
    assert!(
        matching_publisher_rule(
            &policy,
            &PackagePath::parse("other.example.com/finance").unwrap()
        )
        .is_none()
    );
}
#[test]
fn policy_positive_safe_decimal_tokens_and_achievable_thresholds() {
    for (n, phase) in [
        (json!(0), "shape"),
        (json!(-1), "shape"),
        (json!(2), "structure"),
        (json!(1.5), "shape"),
        (json!(9007199254740992u64), "shape"),
    ] {
        let mut v = mothers::policy();
        v["publisherRules"][0]["threshold"] = n;
        assert_eq!(wire(parse(v).unwrap_err())["phase"], phase);
    }
    for token in ["1e0", "1.0", "-0"] {
        let text = mothers::policy().to_string().replacen(
            "\"threshold\":1",
            &format!("\"threshold\":{token}"),
            1,
        );
        assert_eq!(
            wire(decode_trust_policy(text.as_bytes()).unwrap_err())["phase"],
            "shape"
        );
    }
}
#[test]
fn policy_duplicates_bounds_and_closed_fields() {
    for pointer in [
        "/repositories",
        "/publisherRules",
        "/publisherRules/0/publicKeys",
        "/repositories/0/namespaces",
    ] {
        let mut v = mothers::policy();
        let a = v.pointer_mut(pointer).unwrap().as_array_mut().unwrap();
        a.push(a[0].clone());
        assert_eq!(wire(parse(v).unwrap_err())["phase"], "structure");
    }
    for (pointer, count, resource) in [
        ("/publisherRules", 1025, "publisher-rules"),
        ("/publisherRules/0/publicKeys", 65, "publisher-keys"),
        ("/repositories/0/namespaces", 1025, "namespace-grants"),
    ] {
        let mut v = mothers::policy();
        *v.pointer_mut(pointer).unwrap() = json!(vec![json!(null); count]);
        assert_eq!(
            wire(parse(v).unwrap_err())["witnesses"][0]["resource"],
            resource
        );
    }
    let mut v = mothers::policy();
    v["repositories"] = json!([]);
    v["publisherRules"] = json!([]);
    assert!(parse(v.clone()).is_ok());
    v["implicitTrust"] = json!(true);
    assert_eq!(wire(parse(v.clone()).unwrap_err())["phase"], "shape");
    v["continuedUse"] = json!("future");
    assert_eq!(wire(parse(v).unwrap_err())["phase"], "support");
}
#[test]
fn thresholds_count_distinct_authorized_keys() {
    let mut v = mothers::policy();
    v["publisherRules"][0]["publicKeys"] = json!(["0".repeat(64), "1".repeat(64)]);
    v["publisherRules"][0]["threshold"] = json!(2);
    let p = parse(v).unwrap();
    let rule = matching_publisher_rule(&p, &PackagePath::parse("example.com/other").unwrap());
    let zero = PublisherKey::parse(&"0".repeat(64)).unwrap();
    let one = PublisherKey::parse(&"1".repeat(64)).unwrap();
    let two = PublisherKey::parse(&"2".repeat(64)).unwrap();
    assert!(!publisher_threshold_met(
        rule,
        &[zero.clone(), zero.clone(), two]
    ));
    assert!(publisher_threshold_met(rule, &[zero, one]));
    assert!(!publisher_threshold_met(None, &[]));
}
