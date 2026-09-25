use morphir_core::ir::v4::{
    DocumentMeta, IRFile, expand_document_graph, expand_v4_single_file_graph,
};
use morphir_core::metadata::{AssertionSource, Carrier, ContextResources, DocumentId, ObjectTerm};
use morphir_core::node_address::NodeUri;
use serde_json::json;

fn uri(local: &str) -> String {
    format!("morphir://ir/pkg/acme/orders?format=4.1.0#/module/api/value/{local}")
}

#[test]
fn document_graph_expands_aliases_and_preserves_explicit_source_override() {
    let subject = uri("legacy-submit-order");
    let predicate = uri("deprecated");
    let meta = DocumentMeta::parse(&json!({
        "@context": {"deprecated": predicate},
        "@graph": [{"@id": subject, "deprecated": true}],
        "assertionSources": [{
            "selector": {
                "carrier": "documentGraph", "subject": subject, "predicate": predicate,
                "object": {"@value": true}
            },
            "sources": [{"kind": "author", "ref": "review/42"}]
        }]
    }))
    .unwrap();
    let owner = DocumentId::new("pkg/api/values.json").unwrap();
    let graph =
        expand_document_graph(&meta, &owner, &ContextResources::new("."), |_| None).unwrap();
    assert_eq!(graph.facts().len(), 1);
    assert_eq!(graph.assertions().len(), 1);
    assert_eq!(
        graph.outgoing(&NodeUri::parse(&subject).unwrap())[0].object(),
        &ObjectTerm::value(json!(true))
    );
    assert_eq!(
        graph.assertions()[0].sources(),
        vec![AssertionSource::Author {
            reference: "review/42".into()
        }]
    );
}

#[test]
fn document_graph_keeps_distinct_subjects_and_coalesces_repeated_alias() {
    let subject = uri("legacy-submit-order");
    let other = uri("submit-order-v2");
    let predicate = uri("deprecated");
    let meta = DocumentMeta::parse(&json!({
        "@context": {"deprecated": predicate, "retired": predicate},
        "@graph": [
            {"@id": subject, "deprecated": true, "retired": true},
            {"@id": other, "deprecated": true}
        ]
    }))
    .unwrap();
    let graph = expand_document_graph(
        &meta,
        &DocumentId::new("metadata.json").unwrap(),
        &ContextResources::new("."),
        |_| None,
    )
    .unwrap();
    assert_eq!(graph.facts().len(), 2);
    assert_eq!(graph.assertions().len(), 2);
    assert_eq!(graph.outgoing(&NodeUri::parse(&subject).unwrap()).len(), 1);
    assert_eq!(graph.outgoing(&NodeUri::parse(&other).unwrap()).len(), 1);
}

#[test]
fn explicit_expanded_objects_match_source_selectors() {
    let subject = uri("legacy-submit-order");
    let next = uri("submit-order-v2");
    let literal = uri("deprecated");
    let link = uri("replacement");
    let structured = uri("target-names");
    let metadata = DocumentMeta::parse(&json!({
        "@context": {"literal": literal, "link": link, "structured": structured},
        "@graph": [{
            "@id": subject,
            "literal": {"@value": true},
            "link": {"@id": next},
            "structured": {"@value": {"backend": {"java": "submitOrder"}}, "@type": "@json"}
        }],
        "assertionSources": [
            {"selector": {"carrier": "documentGraph", "subject": subject,
                "predicate": literal, "object": {"@value": true}},
             "sources": [{"kind": "author", "ref": "literal"}]},
            {"selector": {"carrier": "documentGraph", "subject": subject,
                "predicate": link, "object": {"@id": next}},
             "sources": [{"kind": "author", "ref": "link"}]},
            {"selector": {"carrier": "documentGraph", "subject": subject,
                "predicate": structured,
                "object": {"@value": {"backend": {"java": "submitOrder"}}, "@type": "@json"}},
             "sources": [{"kind": "author", "ref": "structured"}]}
        ]
    }))
    .unwrap();
    let graph = expand_document_graph(
        &metadata,
        &DocumentId::new("metadata.json").unwrap(),
        &ContextResources::new("."),
        |predicate| {
            (predicate.to_string() == structured)
                .then(|| NodeUri::parse(&uri("target-names-type")).unwrap())
        },
    )
    .unwrap();
    assert_eq!(graph.facts().len(), 3);
    assert_eq!(graph.assertions().len(), 3);
    assert!(
        graph
            .assertions()
            .iter()
            .all(|assertion| assertion.sources().len() == 1)
    );
}

#[test]
fn explicit_expanded_object_with_stale_selector_is_rejected() {
    let subject = uri("legacy-submit-order");
    let predicate = uri("deprecated");
    let error = DocumentMeta::parse(&json!({
        "@context": {"deprecated": predicate},
        "@graph": [{"@id": subject, "deprecated": {"@value": true}}],
        "assertionSources": [{
            "selector": {"carrier": "documentGraph", "subject": subject,
                "predicate": predicate, "object": {"@value": false}},
            "sources": [{"kind": "author", "ref": "stale"}]
        }]
    }))
    .unwrap_err();
    assert!(
        error.contains("no matching document-graph assertion"),
        "{error}"
    );
}

#[test]
fn single_file_projection_keeps_type_value_and_annotation_carriers() {
    let mut document: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ir/v4/complete-example.json")).unwrap();
    let predicate = uri("deprecated");
    document["formatVersion"] = json!("4.1.0");
    document["$meta"] = json!({
        "@context": {"deprecated": predicate},
        "@graph": [{"@id": uri("legacy-submit-order"), "deprecated": true}]
    });
    document["distribution"]["Library"]["def"]["modules"]["u-s/f-r-2052-a/data-tables"]["value"]
        ["types"]["data-tables"]["TypeAliasDefinition"]["typeExp"]["Record"]["attributes"] =
        json!({"facts": {"deprecated": true}});
    document["distribution"]["Library"]["def"]["modules"]["u-s/f-r-2052-a/data-tables"]["value"]
        ["values"]["calculate-total"]["ExpressionBody"]["body"]["Literal"] =
        json!({"attributes": {"facts": {"deprecated": true}}, "literal": {"FloatLiteral": 0.0}});
    document["distribution"]["Library"]["dependencies"]["morphir/SDK"]["modules"]["basics"]["values"]
        ["add"]["annotations"] = json!({"facts": {"deprecated": true}});
    let file: IRFile = serde_json::from_value(document).unwrap();
    let graph = expand_v4_single_file_graph(
        &file,
        &DocumentId::new("library.json").unwrap(),
        &ContextResources::new("."),
        |_| None,
    )
    .unwrap();
    assert_eq!(graph.facts().len(), 4);
    assert_eq!(graph.assertions().len(), 4);
    let carriers = graph
        .assertions()
        .iter()
        .map(|assertion| std::mem::discriminant(assertion.key().carrier()))
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(carriers.len(), 3);
}

#[test]
fn inferred_type_node_has_an_address_and_its_own_fact() {
    let mut document: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ir/v4/complete-example.json")).unwrap();
    let predicate = uri("deprecated");
    document["formatVersion"] = json!("4.1.0");
    document["$meta"] = json!({"@context": {"deprecated": predicate}});
    document["distribution"]["Library"]["def"]["modules"]["u-s/f-r-2052-a/data-tables"]["value"]
        ["values"]["calculate-total"]["ExpressionBody"]["body"]["Literal"] = json!({
        "attributes": {
            "inferredType": {"Unit": {"attributes": {"facts": {"deprecated": true}}}}
        },
        "literal": {"FloatLiteral": 0.0}
    });
    let file: IRFile = serde_json::from_value(document).unwrap();
    let graph = expand_v4_single_file_graph(
        &file,
        &DocumentId::new("library.json").unwrap(),
        &ContextResources::new("."),
        |_| None,
    )
    .unwrap();
    assert_eq!(graph.facts().len(), 1);
    assert!(
        graph.facts()[0]
            .subject()
            .to_string()
            .ends_with("/inferred-type")
    );
}

#[test]
fn annotation_argument_value_has_an_address_and_its_own_fact() {
    let mut document: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ir/v4/complete-example.json")).unwrap();
    let predicate = uri("deprecated");
    document["formatVersion"] = json!("4.1.0");
    document["$meta"] = json!({"@context": {"deprecated": predicate}});
    document["distribution"]["Library"]["dependencies"]["morphir/SDK"]["modules"]["basics"]["values"]
        ["add"]["annotations"] = json!([{
        "name": "morphir/SDK:basics#add",
        "arguments": [
            {"Unit": {"attributes": {"facts": {"deprecated": true}}}}
        ]
    }]);
    let file: IRFile = serde_json::from_value(document).unwrap();
    let graph = expand_v4_single_file_graph(
        &file,
        &DocumentId::new("library.json").unwrap(),
        &ContextResources::new("."),
        |_| None,
    )
    .unwrap();
    assert_eq!(graph.facts().len(), 1);
    assert!(
        graph.facts()[0]
            .subject()
            .to_string()
            .contains("/annotation/entry/0/argument/0")
    );
}

#[test]
fn node_carrier_source_rows_match_only_their_own_authored_assertions() {
    let mut document: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/ir/v4/complete-example.json")).unwrap();
    let predicate = uri("deprecated");
    document["formatVersion"] = json!("4.1.0");
    document["$meta"] = json!({"@context": {"deprecated": predicate}});
    document["distribution"]["Library"]["def"]["modules"]["u-s/f-r-2052-a/data-tables"]["value"]
        ["types"]["data-tables"]["TypeAliasDefinition"]["typeExp"]["Record"]["attributes"] =
        json!({"facts": {"deprecated": true}});
    document["distribution"]["Library"]["dependencies"]["morphir/SDK"]["modules"]["basics"]["values"]
        ["add"]["annotations"] = json!({"facts": {"deprecated": true}});
    let file: IRFile = serde_json::from_value(document.clone()).unwrap();
    let owner = DocumentId::new("library.json").unwrap();
    let graph =
        expand_v4_single_file_graph(&file, &owner, &ContextResources::new("."), |_| None).unwrap();
    assert_eq!(graph.assertions().len(), 2);
    let rows = graph
        .assertions()
        .iter()
        .map(|assertion| {
            let (carrier, source) = match assertion.key().carrier() {
                Carrier::AttributesFacts(_) => ("attributesFacts", "type"),
                Carrier::AnnotationsFacts(_) => ("annotationsFacts", "annotation"),
                _ => unreachable!(),
            };
            json!({
                "selector": {
                    "carrier": carrier,
                    "subject": assertion.key().fact().subject().to_string(),
                    "predicate": predicate,
                    "object": {"@value": true}
                },
                "sources": [{"kind": "author", "ref": source}]
            })
        })
        .collect::<Vec<_>>();
    document["$meta"]["assertionSources"] = json!(rows);
    let file: IRFile = serde_json::from_value(document.clone()).unwrap();
    let graph =
        expand_v4_single_file_graph(&file, &owner, &ContextResources::new("."), |_| None).unwrap();
    assert_eq!(graph.assertions().len(), 2);
    assert!(graph.assertions().iter().all(|assertion| matches!(
        assertion.sources().as_slice(),
        [AssertionSource::Author { .. }]
    )));

    document["$meta"]["assertionSources"][0]["selector"]["object"] = json!({"@value": false});
    let error = serde_json::from_value::<IRFile>(document)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unmatched") || error.contains("matching"),
        "{error}"
    );
}
