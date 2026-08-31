use super::*;
use pretty_assertions::assert_eq;

#[test]
fn strict_errors_emit_no_artifacts_and_partial_generation_preserves_warning_locations() {
    let input = partial_package();
    let strict = generate(&input, &AvroOptions::default());
    assert!(!strict.success);
    assert!(strict.artifacts.is_empty());
    assert_eq!(strict.diagnostics.len(), 1);
    assert_eq!(strict.diagnostics[0].code.as_deref(), Some("AVRO001"));
    assert_eq!(
        strict.diagnostics[0]
            .location
            .as_ref()
            .map(|location| location.uri.as_str()),
        Some("morphir-fqname:acme/customer:customer#unsupported")
    );

    let partial = generate(
        &input,
        &AvroOptions {
            unsupported: Unsupported::WarnAndSkip,
            ..AvroOptions::default()
        },
    );
    assert!(partial.success);
    assert_eq!(partial.artifacts.len(), 1);
    assert_eq!(partial.diagnostics.len(), 1);
    assert_eq!(
        partial.diagnostics[0].severity,
        morphir_extension_sdk::DiagnosticSeverity::Warning
    );
    assert_eq!(partial.diagnostics[0].code.as_deref(), Some("AVRO001"));
    assert_eq!(
        partial.diagnostics[0]
            .location
            .as_ref()
            .map(|location| location.uri.as_str()),
        Some("morphir-fqname:acme/customer:customer#unsupported")
    );
}

#[test]
fn dependency_policy_embeds_each_closure_or_emits_each_link_once() {
    let input = linked_package();
    let self_contained = generate(&input, &AvroOptions::default());
    assert!(self_contained.success);
    assert_eq!(self_contained.artifacts.len(), 1);
    assert_eq!(
        self_contained.artifacts[0].path,
        "acme/customer/customer/Order.avsc"
    );
    assert!(
        self_contained.artifacts[0]
            .content
            .contains("\"name\": \"Customer\"")
    );
    Schema::parse_str(&self_contained.artifacts[0].content).unwrap();

    let linked = generate(
        &input,
        &AvroOptions {
            dependencies: morphir_avro_extension::Dependencies::Linked,
            ..AvroOptions::default()
        },
    );
    assert!(linked.success);
    assert_eq!(
        linked
            .artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect::<Vec<_>>(),
        [
            "acme/shared/types/Customer.avsc",
            "acme/customer/customer/Order.avsc"
        ]
    );
    let order = linked
        .artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with("Order.avsc"))
        .unwrap();
    assert_eq!(
        order.content.matches("acme.shared.types.Customer").count(),
        1
    );
    assert_eq!(
        linked
            .artifacts
            .iter()
            .filter(|artifact| artifact.path == "acme/shared/types/Customer.avsc")
            .count(),
        1
    );
    Schema::parse_list(
        linked
            .artifacts
            .iter()
            .map(|artifact| artifact.content.as_str()),
    )
    .expect("Apache Avro accepts the coherent linked schema set");
}

#[test]
fn linked_protocol_keeps_dependency_declarations_out_of_its_types_array() {
    let mut input = linked_package();
    input.kind = DistributionKind::Application;
    input.modules[0].values = vec![value_specification(
        "acme/customer:customer#get-order",
        "get-order",
        vec![],
        Some(reference("acme/customer:customer#order", vec![])),
        ValueKind::Constant,
        None,
    )];
    let result = generate(
        &input,
        &AvroOptions {
            projection: Projection::ProtocolPublic,
            dependencies: morphir_avro_extension::Dependencies::Linked,
            ..AvroOptions::default()
        },
    );
    assert!(result.success);
    assert_eq!(result.artifacts.len(), 2);
    let protocol = result
        .artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with(".avpr"))
        .unwrap();
    let value: Value = serde_json::from_str(&protocol.content).unwrap();
    let types = value["types"].as_array().unwrap();
    assert_eq!(types.len(), 1);
    assert_eq!(types[0]["name"], "Order");
    assert_eq!(types[0]["fields"][0]["type"], "acme.shared.types.Customer");
    assert_eq!(
        result
            .artifacts
            .iter()
            .filter(|artifact| artifact.path == "acme/shared/types/Customer.avsc")
            .count(),
        1
    );
    validate_protocol_registry_with_linked(
        &value,
        result
            .artifacts
            .iter()
            .filter(|artifact| artifact.path.ends_with(".avsc"))
            .map(|artifact| artifact.content.as_str()),
    );
}

#[test]
fn linked_root_alias_uses_a_full_name_and_does_not_duplicate_its_declaration() {
    let result = generate(
        &linked_alias_package(),
        &AvroOptions {
            dependencies: morphir_avro_extension::Dependencies::Linked,
            ..AvroOptions::default()
        },
    );
    assert!(result.success);
    assert_eq!(result.artifacts.len(), 2);
    let root = result
        .artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with("CustomerAlias.avsc"))
        .unwrap();
    let value: Value = serde_json::from_str(&root.content).unwrap();
    assert_eq!(value, "acme.shared.types.Customer");
    let declaration = result
        .artifacts
        .iter()
        .find(|artifact| artifact.path == "acme/shared/types/Customer.avsc")
        .unwrap();
    let reference_holder = format!(
        r#"{{"type":"record","name":"ReferenceHolder","fields":[{{"name":"value","type":{value}}}]}}"#,
        value = root.content.trim()
    );
    Schema::parse_list([declaration.content.as_str(), reference_holder.as_str()])
        .expect("Apache Avro resolves the linked root full name in context");
}

#[test]
fn protocol_defines_acyclic_dependencies_at_first_use() {
    let result = generate(
        &acyclic_protocol_package(),
        &options(Projection::ProtocolPublic),
    );
    assert!(result.success);
    let protocol = only_protocol_json(&result.artifacts);
    let types = protocol["types"].as_array().unwrap();
    assert_eq!(types.len(), 1);
    assert_eq!(types[0]["name"], "Customer");
    assert_eq!(types[0]["fields"][0]["type"]["name"], "Identifier");
    validate_protocol_registry(&protocol);
}

#[test]
fn protocol_nests_mutually_recursive_peer_once_and_uses_an_active_back_reference() {
    let result = generate(
        &mutually_recursive_protocol_package(),
        &options(Projection::ProtocolPublic),
    );
    assert!(result.success, "{:?}", result.diagnostics);
    let protocol = only_protocol_json(&result.artifacts);
    let types = protocol["types"].as_array().unwrap();
    assert_eq!(types.len(), 1);
    assert_eq!(types[0]["name"], "A");
    assert_eq!(types[0]["fields"][0]["type"]["name"], "B");
    assert_eq!(
        types[0]["fields"][0]["type"]["fields"][0]["type"],
        "acme.customer.customer.A"
    );
    validate_protocol_registry(&protocol);
}

#[test]
fn linked_schema_chain_is_a_coherent_nonduplicated_bundle() {
    let result = generate(
        &linked_chain_package(),
        &AvroOptions {
            dependencies: morphir_avro_extension::Dependencies::Linked,
            ..AvroOptions::default()
        },
    );
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(
        result
            .artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect::<Vec<_>>(),
        [
            "acme/shared/types/Identifier.avsc",
            "acme/shared/types/Customer.avsc",
            "acme/customer/customer/Order.avsc"
        ]
    );
    let customer = result
        .artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with("Customer.avsc"))
        .unwrap();
    let customer: Value = serde_json::from_str(&customer.content).unwrap();
    assert_eq!(
        customer["fields"][0]["type"],
        "acme.shared.types.Identifier"
    );
    assert!(customer["fields"][0]["type"].get("name").is_none());
    let schema_strings = result
        .artifacts
        .iter()
        .map(|artifact| artifact.content.as_str())
        .collect::<Vec<_>>();
    let parsed = Schema::parse_list(schema_strings).expect("Apache Avro accepts linked bundle");
    ResolvedSchema::new_with_schemata(parsed.iter().collect())
        .expect("Apache Avro resolves every linked bundle name");
}

#[test]
fn linked_cross_ownership_cycle_emits_one_cluster_definition_and_resolves_bundle() {
    let input = linked_cross_ownership_cycle_package();
    let result = generate(
        &input,
        &AvroOptions {
            dependencies: morphir_avro_extension::Dependencies::Linked,
            ..AvroOptions::default()
        },
    );
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(
        result
            .artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect::<Vec<_>>(),
        [
            "acme/customer/customer/ZOwnedShared.avsc",
            "acme/customer/customer/ARoot.avsc"
        ]
    );
    let leader: Value = serde_json::from_str(&result.artifacts[0].content).unwrap();
    assert_eq!(leader["name"], "ZOwnedShared");
    assert_eq!(leader["fields"][0]["type"]["name"], "Dependency");
    assert_eq!(
        leader["fields"][0]["type"]["fields"][0]["type"],
        "acme.customer.customer.ZOwnedShared"
    );

    let mut names = std::collections::BTreeSet::new();
    for artifact in &result.artifacts {
        let value: Value = serde_json::from_str(&artifact.content).unwrap();
        collect_named_definitions(&value, &mut names);
    }
    assert_eq!(
        names,
        std::collections::BTreeSet::from([
            "acme.customer.customer.ARoot".to_owned(),
            "acme.customer.customer.ZOwnedShared".to_owned(),
            "acme.shared.types.Dependency".to_owned(),
        ])
    );
    let (dependent, known) = Schema::parse_str_with_list(
        &result.artifacts[1].content,
        [result.artifacts[0].content.as_str()],
    )
    .expect("Apache Avro resolves the dependent artifact after the cluster leader");
    let mut registry_schemas = known.iter().collect::<Vec<_>>();
    registry_schemas.push(&dependent);
    ResolvedSchema::new_with_schemata(registry_schemas)
        .expect("the returned artifact sequence forms one resolved registry");

    let mut shuffled = input;
    shuffled.modules[0].types.reverse();
    shuffled.dependencies.reverse();
    for dependency in &mut shuffled.dependencies {
        dependency.modules.reverse();
        for module in &mut dependency.modules {
            module.types.reverse();
        }
    }
    let shuffled = generate(
        &shuffled,
        &AvroOptions {
            dependencies: morphir_avro_extension::Dependencies::Linked,
            ..AvroOptions::default()
        },
    );
    assert_eq!(
        artifact_pairs(&result.artifacts),
        artifact_pairs(&shuffled.artifacts)
    );
}

#[test]
fn linked_owned_cycle_keeps_nonleader_root_as_a_reference() {
    let result = generate(
        &mutually_recursive_schema_package(),
        &AvroOptions {
            dependencies: morphir_avro_extension::Dependencies::Linked,
            ..AvroOptions::default()
        },
    );
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(
        result
            .artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect::<Vec<_>>(),
        [
            "acme/customer/customer/A.avsc",
            "acme/customer/customer/B.avsc"
        ]
    );
    let leader: Value = serde_json::from_str(&result.artifacts[0].content).unwrap();
    assert_eq!(leader["name"], "A");
    assert_eq!(leader["fields"][0]["type"]["name"], "B");
    let nonleader: Value = serde_json::from_str(&result.artifacts[1].content).unwrap();
    assert_eq!(nonleader, "acme.customer.customer.B");
    Schema::parse_str_with_list(
        &result.artifacts[1].content,
        [result.artifacts[0].content.as_str()],
    )
    .expect("Apache Avro resolves the nonleader root through the leader cluster");
}

#[test]
fn linked_self_loop_remains_one_standalone_recursive_definition() {
    let result = generate(
        &linked_self_loop_package(),
        &AvroOptions {
            dependencies: morphir_avro_extension::Dependencies::Linked,
            ..AvroOptions::default()
        },
    );
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(
        result
            .artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect::<Vec<_>>(),
        [
            "acme/shared/types/Node.avsc",
            "acme/customer/customer/Root.avsc"
        ]
    );
    let node: Value = serde_json::from_str(&result.artifacts[0].content).unwrap();
    assert_eq!(node["name"], "Node");
    assert_eq!(node["fields"][0]["type"], "acme.shared.types.Node");
    let parsed = Schema::parse_list(
        result
            .artifacts
            .iter()
            .map(|artifact| artifact.content.as_str()),
    )
    .expect("Apache Avro parses the linked self-loop bundle");
    ResolvedSchema::new_with_schemata(parsed.iter().collect())
        .expect("self-loop definition precedes its owned root user");
}
