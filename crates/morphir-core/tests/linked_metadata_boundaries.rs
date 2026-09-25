use morphir_core::ir::v4::{
    Annotation, Annotations, ModuleSpecification, Type, TypeAttributes, TypeDefinitionFile,
    TypeSpecification, Value, ValueAttributes, ValueSpecification,
};
use serde_json::json;

#[test]
fn standalone_annotation_and_specification_decoders_reject_unresolved_aliases() {
    assert!(serde_json::from_value::<Annotations>(json!(["not-a-qualified-name"])).is_err());
    assert!(
        serde_json::from_value::<ModuleSpecification>(json!({
            "annotations": ["not-a-qualified-name"], "types": {}, "values": {}
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<TypeSpecification>(json!({
            "OpaqueTypeSpecification": {"annotations": ["not-a-qualified-name"]}
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<ValueSpecification>(json!({
            "annotations": ["not-a-qualified-name"], "output": "morphir/SDK:string#string"
        }))
        .is_err()
    );
    let annotations: Annotations = serde_json::from_value(json!({
        "@context": {"publicApi": "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/api/value/public-api"},
        "entries": ["publicApi"]
    }))
    .unwrap();
    assert!(matches!(
        annotations.entries[0],
        Annotation::LinkedCompact { .. }
    ));
}

#[test]
fn standalone_node_decoders_reject_unvalidated_contexts() {
    let malformed = json!({"broken": {"@id": 123}});
    assert!(
        serde_json::from_value::<Type>(json!({
            "Unit": {"attributes": {"@context": malformed}}
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<Value>(json!({
            "Unit": {"attributes": {"@context": malformed}}
        }))
        .is_err()
    );
    assert!(serde_json::from_value::<TypeAttributes>(json!({"@context": malformed})).is_err());
    assert!(serde_json::from_value::<ValueAttributes>(json!({"@context": malformed})).is_err());
    assert!(
        serde_json::from_value::<TypeDefinitionFile>(json!({
            "formatVersion": 4,
            "name": "item",
            "def": {"Public": {"TypeAliasDefinition": {
                "typeParams": [],
                "typeExp": {"Unit": {"attributes": {"@context": malformed}}}
            }}}
        }))
        .is_err()
    );
}
