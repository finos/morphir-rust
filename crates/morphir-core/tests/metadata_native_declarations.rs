use morphir_core::data_value::DataValueValidator;
use morphir_core::ir::{json::read_ir_file, v4};
use morphir_core::metadata::admission::{Admission, NodeTargetKind, PredicateClosure, SubjectRole};
use morphir_core::metadata::{
    Carrier, ContextResources, Fact, GraphName, ObjectTerm, ProviderDeclarationError,
};
use morphir_core::node_address::NodeUri;
use serde_json::json;

const PREDICATE: &str =
    "morphir://ir/pkg/acme/metadata?format=4.1.0#/module/lifecycle/value/deprecated";
const SUBJECT: &str = "morphir://ir/pkg/acme/orders?format=4.1.0#/module/api/value/submit-order";
const SCHEMA_VOCAB: &str = "morphir://ir/pkg/morphir/metadata?format=4.1.0#/module/schema/value/";

fn provider_document() -> serde_json::Value {
    json!({
        "formatVersion": "4.1.0",
        "distribution": {"Library": {
            "packageName": "acme/metadata",
            "dependencies": {},
            "def": {"modules": {"lifecycle": {"Public": {
                "types": {},
                "values": {"deprecated": {"Public": {"ExpressionBody": {
                    "inputTypes": {},
                    "outputType": "morphir/SDK:basics#bool",
                    "body": {"Literal": {"BoolLiteral": true}}
                }}}}
            }}}}
        }},
        "$meta": {
            "@context": {"@vocab": SCHEMA_VOCAB},
            "@graph": [{
                "@id": PREDICATE,
                "subject-role": ["ValueSpecification", "ValueDefinition"],
                "object-form": "data",
                "interpreter": "descriptive"
            }]
        }
    })
}

fn read_provider(authored: &serde_json::Value) -> v4::IRFile {
    read_ir_file(&authored.to_string()).unwrap().0
}

#[test]
fn library_native_facts_define_a_typed_predicate_without_a_closure_file() {
    let provider = read_provider(&provider_document());
    let closure =
        PredicateClosure::from_v4_provider(&provider, &ContextResources::new("contexts")).unwrap();
    let fact = Fact::new(
        NodeUri::parse(SUBJECT).unwrap(),
        NodeUri::parse(PREDICATE).unwrap(),
        ObjectTerm::value(json!(true)),
        GraphName::Default,
    );
    let validator = DataValueValidator::v4(&provider.distribution).unwrap();
    assert_eq!(
        closure.admit(
            &fact,
            SubjectRole::ValueSpecification,
            &Carrier::DocumentGraph,
            &validator,
            &[],
            None,
        ),
        Ok(Admission::Validated)
    );
}

#[test]
fn native_declaration_cannot_target_a_private_or_missing_library_value() {
    let mut private = provider_document();
    let values =
        &mut private["distribution"]["Library"]["def"]["modules"]["lifecycle"]["Public"]["values"];
    values["deprecated"] = json!({"Private": values["deprecated"]["Public"].clone()});
    let private = read_provider(&private);
    assert!(matches!(
        PredicateClosure::from_v4_provider(&private, &ContextResources::new("contexts")),
        Err(ProviderDeclarationError::MissingPublicValue)
    ));

    let mut missing = provider_document();
    missing["$meta"]["@graph"][0]["@id"] =
        json!("morphir://ir/pkg/acme/metadata?format=4.1.0#/module/lifecycle/value/unknown");
    let missing = read_provider(&missing);
    assert!(
        PredicateClosure::from_v4_provider(&missing, &ContextResources::new("contexts")).is_err()
    );
}

#[test]
fn native_json_predicate_uses_its_public_output_type_declaration() {
    let mut authored = provider_document();
    let module = &mut authored["distribution"]["Library"]["def"]["modules"]["lifecycle"]["Public"];
    module["types"]["target-names"] = json!({"Public": {"TypeAliasDefinition": {
        "typeParams": [],
        "typeExp": {"Record": {"fields": {
            "frontend": "morphir/SDK:string#string"
        }}}
    }}});
    module["values"]["target-names"] = module["values"]["deprecated"].clone();
    module["values"]["target-names"]["Public"]["ExpressionBody"]["outputType"] =
        json!("acme/metadata:lifecycle#target-names");
    authored["$meta"]["@graph"][0]["@id"] =
        json!("morphir://ir/pkg/acme/metadata?format=4.1.0#/module/lifecycle/value/target-names");
    authored["$meta"]["@graph"][0]["object-form"] = json!("json");
    let provider = read_provider(&authored);
    let closure =
        PredicateClosure::from_v4_provider(&provider, &ContextResources::new("contexts")).unwrap();
    let fact = Fact::new(
        NodeUri::parse(SUBJECT).unwrap(),
        NodeUri::parse(
            "morphir://ir/pkg/acme/metadata?format=4.1.0#/module/lifecycle/value/target-names",
        )
        .unwrap(),
        ObjectTerm::typed_json(
            json!({"frontend":"gleam"}),
            NodeUri::parse(
                "morphir://ir/pkg/acme/metadata?format=4.1.0#/module/lifecycle/type/target-names",
            )
            .unwrap(),
        ),
        GraphName::Default,
    );
    let validator = DataValueValidator::v4(&provider.distribution).unwrap();
    assert_eq!(
        closure.admit(
            &fact,
            SubjectRole::ValueSpecification,
            &Carrier::DocumentGraph,
            &validator,
            &[],
            None,
        ),
        Ok(Admission::Validated)
    );
}

#[test]
fn native_node_predicate_requires_the_declared_target_kind() {
    let mut authored = provider_document();
    let values =
        &mut authored["distribution"]["Library"]["def"]["modules"]["lifecycle"]["Public"]["values"];
    values["replacement"] = values["deprecated"].clone();
    values["replacement"]["Public"]["ExpressionBody"]["outputType"] =
        json!("morphir/SDK:string#string");
    authored["$meta"]["@graph"][0]["@id"] =
        json!("morphir://ir/pkg/acme/metadata?format=4.1.0#/module/lifecycle/value/replacement");
    authored["$meta"]["@graph"][0]["object-form"] = json!("node");
    authored["$meta"]["@graph"][0]["node-target-kind"] = json!("Value");
    let provider = read_provider(&authored);
    let closure =
        PredicateClosure::from_v4_provider(&provider, &ContextResources::new("contexts")).unwrap();
    let fact = Fact::new(
        NodeUri::parse(SUBJECT).unwrap(),
        NodeUri::parse(
            "morphir://ir/pkg/acme/metadata?format=4.1.0#/module/lifecycle/value/replacement",
        )
        .unwrap(),
        ObjectTerm::NodeRef(
            NodeUri::parse(
                "morphir://ir/pkg/acme/orders-next?format=4.1.0#/module/api/value/submit-order",
            )
            .unwrap(),
        ),
        GraphName::Default,
    );
    let validator = DataValueValidator::v4(&provider.distribution).unwrap();
    assert_eq!(
        closure.admit(
            &fact,
            SubjectRole::ValueSpecification,
            &Carrier::DocumentGraph,
            &validator,
            &[],
            Some(NodeTargetKind::Value),
        ),
        Ok(Admission::Validated)
    );
}

#[test]
fn specs_can_scope_the_same_declaration_facts_in_annotations() {
    let mut authored = provider_document();
    authored["distribution"] = json!({"Specs": {
        "packageName": "acme/metadata",
        "dependencies": {},
        "spec": {"modules": {"lifecycle": {
            "types": {},
            "values": {"deprecated": {
                "annotations": {
                    "@context": {"@vocab": SCHEMA_VOCAB},
                    "facts": {
                        "subject-role": ["ValueSpecification", "ValueDefinition"],
                        "object-form": "data",
                        "interpreter": "descriptive"
                    }
                },
                "output": "morphir/SDK:basics#bool"
            }}
        }}}
    }});
    authored.as_object_mut().unwrap().remove("$meta");
    let provider = read_provider(&authored);
    let closure =
        PredicateClosure::from_v4_provider(&provider, &ContextResources::new("contexts")).unwrap();
    let fact = Fact::new(
        NodeUri::parse(SUBJECT).unwrap(),
        NodeUri::parse(PREDICATE).unwrap(),
        ObjectTerm::value(json!(true)),
        GraphName::Default,
    );
    let validator = DataValueValidator::v4(&provider.distribution).unwrap();
    assert_eq!(
        closure.admit(
            &fact,
            SubjectRole::ValueSpecification,
            &Carrier::DocumentGraph,
            &validator,
            &[],
            None,
        ),
        Ok(Admission::Validated)
    );
}

#[test]
fn node_object_form_cannot_override_a_boolean_output_signature() {
    let mut authored = provider_document();
    authored["$meta"]["@graph"][0]["object-form"] = json!("node");
    authored["$meta"]["@graph"][0]["node-target-kind"] = json!("Value");
    let provider = read_provider(&authored);
    assert!(
        PredicateClosure::from_v4_provider(&provider, &ContextResources::new("contexts")).is_err()
    );
}
