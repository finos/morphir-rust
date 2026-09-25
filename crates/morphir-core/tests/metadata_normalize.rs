use morphir_core::metadata::{
    ContextError, ContextResources, GraphName, ObjectTerm, expand_properties, resolve_context,
};
use morphir_core::node_address::NodeUri;
use serde_json::{Value, json};

fn uri(local: &str) -> NodeUri {
    NodeUri::parse(&format!(
        "morphir://ir/pkg/acme/orders?format=4.1.0#/module/api/value/{local}"
    ))
    .unwrap()
}

fn context() -> morphir_core::metadata::EffectiveContext {
    resolve_context(
        None,
        &json!({
            "label": uri("label").to_string(),
            "successor": {"@id": uri("successor").to_string(), "@type": "@id"},
            "targetNames": {"@id": uri("target-names").to_string(), "@type": "@json"},
            "targetNamesExpanded": uri("target-names").to_string()
        }),
        &ContextResources::new("contexts"),
        None,
    )
    .unwrap()
}

fn expand(value: Value) -> Result<Vec<morphir_core::metadata::Fact>, ContextError> {
    let properties = value.as_object().unwrap();
    expand_properties(
        &uri("submit-order"),
        properties.iter().map(|(key, value)| (key.as_str(), value)),
        &context(),
        |predicate| (predicate == &uri("target-names")).then(|| uri("target-names-type")),
    )
}

#[test]
fn repeated_values_expand_but_absence_and_bare_null_do_not() {
    let facts = expand(json!({"label": ["first", null, "second"]})).unwrap();
    assert_eq!(facts.len(), 2);
    assert_eq!(facts[0].object(), &ObjectTerm::value(json!("first")));
    assert_eq!(facts[1].object(), &ObjectTerm::value(json!("second")));
    assert!(expand(json!({"label": []})).unwrap().is_empty());
    assert!(expand(json!({"label": null})).unwrap().is_empty());
    assert_eq!(facts[0].graph(), &GraphName::Default);
}

#[test]
fn json_coercion_keeps_one_structured_or_null_object() {
    let facts = expand(json!({"targetNames": {"frontend":{"elm":"placeOrder"}}})).unwrap();
    assert_eq!(facts.len(), 1);
    assert_eq!(
        facts[0].object(),
        &ObjectTerm::typed_json(
            json!({"frontend":{"elm":"placeOrder"}}),
            uri("target-names-type")
        )
    );
    let list = expand(json!({"targetNames": ["a", "b"]})).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0].object(),
        &ObjectTerm::typed_json(json!(["a", "b"]), uri("target-names-type"))
    );
    assert_eq!(expand(json!({"targetNames": null})).unwrap().len(), 1);
}

#[test]
fn node_coercion_and_expanded_id_are_links_but_uri_text_is_data() {
    let next = uri("submit-order-v2");
    let facts = expand(json!({
        "successor": next.to_string(),
        "label": next.to_string()
    }))
    .unwrap();
    assert_eq!(
        facts
            .iter()
            .find(|fact| fact.predicate() == &uri("label"))
            .unwrap()
            .object(),
        &ObjectTerm::value(json!(next.to_string()))
    );
    assert_eq!(
        facts
            .iter()
            .find(|fact| fact.predicate() == &uri("successor"))
            .unwrap()
            .object(),
        &ObjectTerm::NodeRef(next.clone())
    );
    assert_eq!(
        expand(json!({"label": {"@id": next.to_string()}})).unwrap()[0].object(),
        &ObjectTerm::NodeRef(next)
    );
}

#[test]
fn unsupported_plain_object_and_nested_array_are_rejected() {
    assert!(matches!(
        expand(json!({"label": {"backend": "java"}})),
        Err(ContextError::InvalidFactObject)
    ));
    assert!(matches!(
        expand(json!({"label": [["a"]]})),
        Err(ContextError::InvalidFactObject)
    ));
    assert!(matches!(
        expand(json!({"label": {"@value": ["a"]}})),
        Err(ContextError::InvalidFactObject)
    ));
}

#[test]
fn expanded_value_forms_preserve_explicit_null_and_json_type() {
    let null = expand(json!({"label": {"@value": null}})).unwrap();
    assert_eq!(null.len(), 1);
    assert_eq!(null[0].object(), &ObjectTerm::value(Value::Null));

    let structured = expand(json!({
        "targetNamesExpanded": {"@value": {"backend": {"java": "submitOrder"}}, "@type": "@json"}
    }))
    .unwrap();
    assert_eq!(structured.len(), 1);
    assert_eq!(
        structured[0].object(),
        &ObjectTerm::typed_json(
            json!({"backend": {"java": "submitOrder"}}),
            uri("target-names-type")
        )
    );
}
