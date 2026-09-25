use morphir_core::metadata::{
    Coercion, ContextError, ContextResources, expand_object, inline_document_contexts,
    resolve_context,
};
use morphir_core::node_address::NodeUri;
use serde_json::json;
use sha2::Digest;

const NAMING: &str = "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/value/";
const LIFE: &str = "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/";
const OPS: &str = "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/operations/value/";

fn resources() -> ContextResources {
    let mut resources = ContextResources::new("contexts");
    resources.insert_local(
        "contexts/naming.jsonld",
        format!(r#"{{"@context":{{"@vocab":"{NAMING}","aliases":"{NAMING}aliases"}}}}"#)
            .into_bytes(),
    );
    resources.insert_local(
        "contexts/lifecycle.jsonld",
        format!(r#"{{"@context":{{"deprecated":"{LIFE}deprecated"}}}}"#).into_bytes(),
    );
    resources.insert_local(
        "contexts/ops.jsonld",
        format!(r#"{{"@context":{{"ops":{{"@id":"{OPS}","@prefix":true}}}}}}"#).into_bytes(),
    );
    resources.insert_local(
        "contexts/ops-conflict.jsonld",
        format!(r#"{{"@context":{{"ops":{{"@id":"{LIFE}","@prefix":true}}}}}}"#).into_bytes(),
    );
    resources.insert_local(
        "contexts/cycle-a.jsonld",
        br#"{"@context":"./cycle-b.jsonld"}"#.to_vec(),
    );
    resources.insert_local(
        "contexts/cycle-b.jsonld",
        br#"{"@context":"./cycle-a.jsonld"}"#.to_vec(),
    );
    resources
}

#[test]
fn verified_archive_contexts_inline_without_rewriting_fact_data() {
    let mut resources = ContextResources::new(".");
    resources.insert_local(
        "contexts/lifecycle.jsonld",
        format!(r#"{{"@context":{{"deprecated":"{LIFE}deprecated"}}}}"#).into_bytes(),
    );
    let document = json!({
        "$meta":{"@context":"./contexts/lifecycle.jsonld","@graph":[
            {"@id":"morphir://ir/pkg/acme/orders?format=4.1.0#/module/api","deprecated":true}
        ]},
        "node":{"attributes":{
            "@context":{"alias":format!("{NAMING}aliases")},
            "facts":{"alias":{"@value":{"attributes":{"@context":"literal data"}},"@type":"@json"}}
        }}
    });
    let inline = inline_document_contexts(&document, &resources, Some("ir.json")).unwrap();
    assert_eq!(
        inline["$meta"]["@context"]["deprecated"],
        format!("{LIFE}deprecated")
    );
    assert_eq!(
        inline["node"]["attributes"]["@context"]["deprecated"],
        format!("{LIFE}deprecated")
    );
    assert_eq!(
        inline["node"]["attributes"]["@context"]["alias"],
        format!("{NAMING}aliases")
    );
    assert_eq!(inline["$meta"]["@graph"], document["$meta"]["@graph"]);
    assert_eq!(
        inline["node"]["attributes"]["facts"],
        document["node"]["attributes"]["facts"]
    );
    assert_eq!(document["$meta"]["@context"], "./contexts/lifecycle.jsonld");
    assert!(matches!(
        inline_document_contexts(&document, &ContextResources::new("."), Some("ir.json")),
        Err(ContextError::ResourceUnavailable(_))
    ));
}

#[test]
fn graph_record_scope_is_inlined_over_document_scope() {
    let mut resources = ContextResources::new(".");
    resources.insert_local(
        "contexts/lifecycle.jsonld",
        format!(r#"{{"@context":{{"deprecated":"{LIFE}deprecated"}}}}"#).into_bytes(),
    );
    let document = json!({"$meta":{
        "@context":"./contexts/lifecycle.jsonld",
        "@graph":[{
            "@id":"morphir://ir/pkg/acme/orders?format=4.1.0#/module/api",
            "@context":{"@vocab":NAMING},
            "deprecated":true,
            "operational-name":"submitOrder"
        }]
    }});
    let inline = inline_document_contexts(&document, &resources, Some("ir.json")).unwrap();
    let scope = &inline["$meta"]["@graph"][0]["@context"];
    assert_eq!(scope["deprecated"], format!("{LIFE}deprecated"));
    assert_eq!(scope["@vocab"], NAMING);
    assert_eq!(
        inline["$meta"]["@graph"][0]["operational-name"],
        "submitOrder"
    );
}

#[test]
fn aliases_prefixes_and_vocab_expand_with_precedence() {
    let context = resolve_context(
        None,
        &json!({
            "@vocab": NAMING,
            "ops": {"@id": OPS, "@prefix": true},
            "aliases": format!("{LIFE}deprecated"),
            "replacedBy": {"@id": "ops:replacement", "@type": "@id"},
            "targetNames": {"@id": format!("{NAMING}target-names"), "@type": "@json"}
        }),
        &resources(),
        None,
    )
    .unwrap();
    assert_eq!(
        context.expand_key("aliases").unwrap().uri().to_string(),
        format!("{LIFE}deprecated")
    );
    assert_eq!(
        context
            .expand_key("ops:replacement")
            .unwrap()
            .uri()
            .to_string(),
        format!("{OPS}replacement")
    );
    assert_eq!(
        context
            .expand_key("transactional-name")
            .unwrap()
            .uri()
            .to_string(),
        format!("{NAMING}transactional-name")
    );
    assert_eq!(
        context.expand_key("replacedBy").unwrap().coercion(),
        Coercion::NodeId
    );
    assert_eq!(
        context.expand_key("targetNames").unwrap().coercion(),
        Coercion::Json
    );
    assert!(context.expand_key("targetNamesBadCamelCase").is_err());
}

#[test]
fn node_link_and_json_coercion_keep_distinct_object_kinds() {
    let node = json!("morphir://ir/pkg/acme/orders?format=4.0.0#/module/api/value/submit-order");
    let datatype = NodeUri::parse(&format!("{NAMING}target-names")).unwrap();
    assert!(matches!(
        expand_object(Coercion::NodeId, node.clone(), None).unwrap(),
        morphir_core::metadata::ObjectTerm::NodeRef(_)
    ));
    assert!(matches!(
        expand_object(Coercion::None, node.clone(), None).unwrap(),
        morphir_core::metadata::ObjectTerm::Value(_)
    ));
    let typed = expand_object(
        Coercion::Json,
        json!({"node": node}),
        Some(datatype.clone()),
    )
    .unwrap();
    match typed {
        morphir_core::metadata::ObjectTerm::Value(value) => {
            assert_eq!(value.datatype(), Some(&datatype))
        }
        _ => panic!("@json must remain data"),
    }
}

#[test]
fn inherited_scope_is_isolated_and_protected_bindings_cannot_change() {
    let parent = resolve_context(
        None,
        &json!({
            "@vocab": NAMING,
            "name": {"@id": format!("{NAMING}name"), "@protected": true},
            "alias": format!("{NAMING}aliases")
        }),
        &resources(),
        None,
    )
    .unwrap();
    let attributes = resolve_context(
        Some(&parent),
        &json!({"@vocab": OPS, "alias": format!("{OPS}aliases")}),
        &resources(),
        None,
    )
    .unwrap();
    let annotations = resolve_context(
        Some(&parent),
        &json!({"alias": format!("{LIFE}deprecated")}),
        &resources(),
        None,
    )
    .unwrap();
    assert_eq!(
        attributes.expand_key("new-name").unwrap().uri().to_string(),
        format!("{OPS}new-name")
    );
    assert_eq!(
        annotations
            .expand_key("new-name")
            .unwrap()
            .uri()
            .to_string(),
        format!("{NAMING}new-name")
    );
    assert_eq!(
        parent.expand_key("alias").unwrap().uri().to_string(),
        format!("{NAMING}aliases")
    );
    assert_eq!(
        resolve_context(
            Some(&parent),
            &json!({"name": format!("{OPS}name")}),
            &resources(),
            None
        )
        .unwrap_err(),
        ContextError::ProtectedTermRedefinition("name".into())
    );
}

#[test]
fn imports_collide_before_inline_override_but_override_one_import() {
    let resources = resources();
    let accepted = resolve_context(None, &json!(["contexts/naming.jsonld", {"@vocab": LIFE, "aliases": {"@id": format!("{NAMING}aliases"), "@type": "@json"}}]), &resources, None).unwrap();
    assert_eq!(
        accepted.expand_key("aliases").unwrap().coercion(),
        Coercion::Json
    );
    assert_eq!(
        accepted.expand_key("deprecated").unwrap().uri().to_string(),
        format!("{LIFE}deprecated")
    );
    assert_eq!(resolve_context(None, &json!(["contexts/ops.jsonld", "contexts/ops-conflict.jsonld", {"ops": {"@id": OPS, "@prefix": true}}]), &resources, None).unwrap_err(), ContextError::TermCollision("ops".into()));
    assert_eq!(
        resolve_context(
            None,
            &json!(["contexts/naming.jsonld", "contexts/naming.jsonld"]),
            &resources,
            None
        )
        .unwrap_err(),
        ContextError::DuplicateImport("contexts/naming.jsonld".into())
    );
}

#[test]
fn recursive_imports_detect_cycles_and_hashed_relative_children() {
    let mut resources = resources();
    assert_eq!(
        resolve_context(None, &json!("contexts/cycle-a.jsonld"), &resources, None).unwrap_err(),
        ContextError::ImportCycle("contexts/cycle-a.jsonld".into())
    );
    let bytes = br#"{"@context":"./child.jsonld"}"#.to_vec();
    let digest = sha2::Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let reference = format!("morphir://context/sha256/{digest}");
    resources.insert_verified(&reference, bytes, true);
    assert_eq!(
        resolve_context(None, &json!(reference), &resources, None).unwrap_err(),
        ContextError::RelativeImportWithoutBase
    );
    let bytes = br#"{"@context":"child.jsonld"}"#.to_vec();
    let digest = sha2::Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let reference = format!("morphir://context/sha256/{digest}");
    resources.insert_verified(&reference, bytes, true);
    assert_eq!(
        resolve_context(None, &json!(reference), &resources, None).unwrap_err(),
        ContextError::RelativeImportWithoutBase
    );
}

#[test]
fn unsupported_keywords_and_invalid_array_forms_fail_as_whole_context() {
    let resources = resources();
    assert_eq!(
        resolve_context(
            None,
            &json!({"@language":"en", "aliases": format!("{NAMING}aliases")}),
            &resources,
            None
        )
        .unwrap_err(),
        ContextError::UnsupportedKeyword("@language".into())
    );
    assert_eq!(resolve_context(None, &json!(["contexts/naming.jsonld", {"aliases": format!("{NAMING}aliases")}, "contexts/lifecycle.jsonld"]), &resources, None).unwrap_err(), ContextError::InvalidForm);
    assert_eq!(
        resolve_context(
            None,
            &json!({"aliases":{"@id":format!("{NAMING}aliases"),"@container":"@list"}}),
            &resources,
            None
        )
        .unwrap_err(),
        ContextError::UnsupportedKeyword("@container".into())
    );
    assert_eq!(
        resolve_context(
            None,
            &json!([{"aliases": format!("{NAMING}aliases") }]),
            &resources,
            None
        )
        .unwrap_err(),
        ContextError::InvalidForm
    );
}

#[test]
fn explicit_resource_identity_and_local_path_errors_are_reported() {
    let mut resources = resources();
    let bytes = br#"{"@context":{"deprecated":"morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/deprecated"}}"#.to_vec();
    let wrong = format!("morphir://context/sha256/{}", "0".repeat(64));
    resources.insert_verified(&wrong, bytes.clone(), true);
    assert_eq!(
        resolve_context(None, &json!(wrong), &resources, None).unwrap_err(),
        ContextError::DigestMismatch(wrong.clone())
    );
    let correct = format!(
        "morphir://context/sha256/{}",
        sha2::Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    assert!(matches!(
        resolve_context(None, &json!(correct), &resources, None),
        Err(ContextError::ResourceUnavailable(_))
    ));
    resources.insert_verified(&correct, bytes.clone(), false);
    assert_eq!(
        resolve_context(None, &json!(correct), &resources, None).unwrap_err(),
        ContextError::ResourceUntrusted(correct.clone())
    );
    resources.insert_verified(&correct, bytes, true);
    assert_eq!(
        resolve_context(None, &json!(correct), &resources, None)
            .unwrap()
            .expand_key("deprecated")
            .unwrap()
            .uri()
            .to_string(),
        format!("{LIFE}deprecated")
    );
    assert_eq!(
        resolve_context(
            None,
            &json!("../outside.jsonld"),
            &resources,
            Some("contexts/naming.jsonld")
        )
        .unwrap_err(),
        ContextError::PathEscape
    );
    assert_eq!(
        resolve_context(
            None,
            &json!("https://example.org/context.jsonld"),
            &resources,
            None
        )
        .unwrap_err(),
        ContextError::RemoteForbidden("https://example.org/context.jsonld".into())
    );
}

#[test]
fn recursive_local_imports_and_vocab_conflicts_are_bounded() {
    let mut resources = resources();
    resources.insert_local(
        "contexts/recursive-root.jsonld",
        br#"{"@context":"./nested/leaf.jsonld"}"#.to_vec(),
    );
    resources.insert_local("contexts/nested/leaf.jsonld", br#"{"@context":["../lifecycle.jsonld",{"aliasDeprecated":"morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/deprecated"}]}"#.to_vec());
    let context = resolve_context(
        None,
        &json!("contexts/recursive-root.jsonld"),
        &resources,
        None,
    )
    .unwrap();
    assert_eq!(
        context
            .expand_key("aliasDeprecated")
            .unwrap()
            .uri()
            .to_string(),
        format!("{LIFE}deprecated")
    );
    assert_eq!(
        resolve_context(None, &json!("./lifecycle.jsonld"), &resources, None)
            .unwrap()
            .expand_key("deprecated")
            .unwrap()
            .uri()
            .to_string(),
        format!("{LIFE}deprecated")
    );
    resources.insert_local(
        "contexts/other-vocab.jsonld",
        format!(r#"{{"@context":{{"@vocab":"{LIFE}"}}}}"#).into_bytes(),
    );
    assert_eq!(
        resolve_context(
            None,
            &json!(["contexts/naming.jsonld", "contexts/other-vocab.jsonld"]),
            &resources,
            None
        )
        .unwrap_err(),
        ContextError::VocabCollision
    );
}

#[test]
fn imported_context_bytes_reject_duplicate_json_members() {
    let mut resources = resources();
    resources.insert_local(
        "contexts/duplicate.jsonld",
        format!(r#"{{"@context":{{"alias":"{NAMING}aliases","alias":"{LIFE}deprecated"}}}}"#)
            .into_bytes(),
    );
    assert_eq!(
        resolve_context(None, &json!("contexts/duplicate.jsonld"), &resources, None).unwrap_err(),
        ContextError::InvalidResource("contexts/duplicate.jsonld".into())
    );
    resources.insert_local(
        "contexts/duplicate.jsonld",
        format!(
            r#"{{"@context":{{"alias":{{"@id":"{NAMING}aliases","@id":"{LIFE}deprecated"}}}}}}"#
        )
        .into_bytes(),
    );
    assert_eq!(
        resolve_context(None, &json!("contexts/duplicate.jsonld"), &resources, None).unwrap_err(),
        ContextError::InvalidResource("contexts/duplicate.jsonld".into())
    );
}

#[test]
fn prefix_marker_requires_a_prefix_stem() {
    assert_eq!(
        resolve_context(
            None,
            &json!({"aliases": {"@id": format!("{NAMING}aliases"), "@prefix": false}}),
            &resources(),
            None
        )
        .unwrap_err(),
        ContextError::InvalidForm
    );
}

#[test]
fn context_targets_must_be_declarations_without_child_steps() {
    let resources = resources();
    let module = "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming";
    let child = format!("{NAMING}aliases/body");
    for target in [module, &child] {
        assert_eq!(
            resolve_context(None, &json!({"alias": target}), &resources, None).unwrap_err(),
            ContextError::InvalidTarget(target.into())
        );
    }
    let bad_stem =
        "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/type/order/type-exp/";
    assert_eq!(
        resolve_context(None, &json!({"@vocab": bad_stem}), &resources, None).unwrap_err(),
        ContextError::InvalidTarget(bad_stem.into())
    );
    let node = NodeUri::parse(&child).unwrap();
    assert!(matches!(
        expand_object(Coercion::NodeId, json!(node.to_string()), None).unwrap(),
        morphir_core::metadata::ObjectTerm::NodeRef(_)
    ));
}

#[test]
fn an_exact_colon_alias_precedes_compact_iri_expansion() {
    let context = resolve_context(
        None,
        &json!({
            "ops": {"@id": OPS, "@prefix": true},
            "ops:aliases": format!("{NAMING}aliases")
        }),
        &resources(),
        None,
    )
    .unwrap();
    assert_eq!(
        context.expand_key("ops:aliases").unwrap().uri().to_string(),
        format!("{NAMING}aliases")
    );
    assert_eq!(
        context
            .expand_key("ops:replacement")
            .unwrap()
            .uri()
            .to_string(),
        format!("{OPS}replacement")
    );
    for key in [":aliases", "ops:", "ops::aliases"] {
        assert_eq!(
            resolve_context(
                None,
                &json!({key: format!("{NAMING}aliases")}),
                &resources(),
                None
            )
            .unwrap_err(),
            ContextError::InvalidForm
        );
    }
}

#[test]
fn imported_numeric_unsupported_keyword_keeps_its_diagnostic() {
    let mut resources = resources();
    for (name, numeric) in [
        ("integer", "1"),
        ("float", "1.1"),
        ("large-integer", "18446744073709551616"),
    ] {
        let path = format!("contexts/{name}.jsonld");
        resources.insert_local(
            &path,
            format!(r#"{{"@context":{{"@version":{numeric}}}}}"#).into_bytes(),
        );
        assert_eq!(
            resolve_context(None, &json!(path), &resources, None).unwrap_err(),
            ContextError::UnsupportedKeyword("@version".into())
        );
    }
}

#[test]
fn doubled_digest_reference_prefix_is_invalid() {
    let mut resources = resources();
    let bytes = format!(r#"{{"@context":{{"deprecated":"{LIFE}deprecated"}}}}"#).into_bytes();
    let digest = sha2::Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let reference = format!("morphir://context/sha256/morphir://context/sha256/{digest}");
    resources.insert_verified(&reference, bytes, true);
    assert_eq!(
        resolve_context(None, &json!(reference), &resources, None).unwrap_err(),
        ContextError::InvalidForm
    );
}

#[test]
fn local_roots_keep_absolute_and_relative_path_identity() {
    for (root, inserted, reference) in [
        (
            "/workspace/contexts",
            "/workspace/contexts/leaf.jsonld",
            "leaf.jsonld",
        ),
        (
            "./contexts",
            "./contexts/leaf.jsonld",
            "contexts/leaf.jsonld",
        ),
        (".", "./leaf.jsonld", "leaf.jsonld"),
    ] {
        let mut resources = ContextResources::new(root);
        resources.insert_local(
            inserted,
            format!(r#"{{"@context":{{"alias":"{NAMING}aliases"}}}}"#).into_bytes(),
        );
        assert_eq!(
            resolve_context(None, &json!(reference), &resources, None)
                .unwrap()
                .expand_key("alias")
                .unwrap()
                .uri()
                .to_string(),
            format!("{NAMING}aliases")
        );
    }
}

#[test]
fn dot_root_resolves_relative_child_from_top_level_file() {
    let mut resources = ContextResources::new(".");
    resources.insert_local(
        "parent.jsonld",
        br#"{"@context":"./child.jsonld"}"#.to_vec(),
    );
    resources.insert_local(
        "child.jsonld",
        format!(r#"{{"@context":{{"alias":"{NAMING}aliases"}}}}"#).into_bytes(),
    );
    let context = resolve_context(None, &json!("parent.jsonld"), &resources, None).unwrap();
    assert_eq!(
        context.expand_key("alias").unwrap().uri().to_string(),
        format!("{NAMING}aliases")
    );
}

#[test]
fn absolute_authored_path_is_rejected_before_base_join() {
    let mut resources = ContextResources::new("contexts");
    resources.insert_local("contexts/leaf.jsonld", br#"{"@context":{}}"#.to_vec());
    for base in [None, Some("contexts/parent.jsonld")] {
        assert_eq!(
            resolve_context(None, &json!("/leaf.jsonld"), &resources, base).unwrap_err(),
            ContextError::PathEscape
        );
    }
}

#[test]
fn acyclic_import_chain_over_depth_budget_fails_without_recursing_further() {
    let mut resources = ContextResources::new("contexts");
    for index in 0..=128 {
        let bytes = if index == 128 {
            br#"{"@context":{}}"#.to_vec()
        } else {
            format!(r#"{{"@context":"./{}.jsonld"}}"#, index + 1).into_bytes()
        };
        resources.insert_local(format!("contexts/{index}.jsonld"), bytes);
    }
    assert_eq!(
        resolve_context(None, &json!("0.jsonld"), &resources, None).unwrap_err(),
        ContextError::ImportDepthExceeded(128)
    );
    assert!(resolve_context(None, &json!("1.jsonld"), &resources, None).is_ok());
}

#[test]
fn materialized_context_keeps_scoped_coercion_and_protection() {
    let mut resources = ContextResources::new(".");
    resources.insert_local(
        "terms.jsonld",
        format!(
            r#"{{"@context":{{"ops":{{"@id":"{NAMING}","@prefix":true,"@protected":true}},"alias":{{"@id":"{NAMING}aliases","@type":"@json"}}}}}}"#
        )
        .into_bytes(),
    );
    let resolved = resolve_context(
        None,
        &json!(["terms.jsonld", {"name": format!("{NAMING}name")}]),
        &resources,
        None,
    )
    .unwrap();
    let materialized = resolved.to_inline_value();
    assert_eq!(materialized["ops"]["@protected"], true);
    assert_eq!(materialized["ops"]["@prefix"], true);
    assert_eq!(materialized["alias"]["@type"], "@json");
    let reread = resolve_context(None, &materialized, &ContextResources::new("."), None).unwrap();
    assert_eq!(reread, resolved);
}
