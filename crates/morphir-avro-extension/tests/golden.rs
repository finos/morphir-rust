mod support;

use std::{env, ffi::OsStr, fs, path::PathBuf};

use apache_avro::{
    Schema,
    schema::{Name, NamesRef, ResolvedSchema},
};
use morphir_avro_extension::{
    Aliases, AvroOptions, Constructor, DistributionKind, EntryPointKind, EntryPointMetadata,
    Projection, ProjectionDependency, ProjectionModule, ProjectionPackage, TypeDeclaration,
    TypeExpr, TypeMapping, Unsupported, ValueKind, generate,
};
use pretty_assertions::assert_eq;
use serde_json::Value;
use support::projection::{alias, customer_record, field, package, reference, value_specification};

const STRING: &str = "morphir/SDK:string#string";
const SET: &str = "morphir/SDK:set#set";
const RESULT: &str = "morphir/SDK:result#result";
const LOCAL_DATE: &str = "morphir/SDK:local-date#local-date";
const LOCAL_TIME: &str = "morphir/SDK:local-time#local-time";
const INSTANT: &str = "morphir/SDK:instant#instant";
const UUID: &str = "morphir/SDK:uuid#uuid";
const CHAR: &str = "morphir/SDK:char#char";
const MAYBE: &str = "morphir/SDK:maybe#maybe";
const DICT: &str = "morphir/SDK:dict#dict";
const DECIMAL: &str = "morphir/SDK:decimal#decimal";
const CUSTOMER: &str = "Acme:Customer:Customer";

#[derive(Clone)]
struct GoldenCase {
    golden: &'static str,
    expected_path: &'static str,
    package: ProjectionPackage,
    options: AvroOptions,
}

#[test]
fn json_modes_match_reviewed_goldens_and_parse_as_avro() {
    for case in json_cases() {
        let result = generate(&case.package, &case.options);
        assert!(result.success, "{}: {:?}", case.golden, result.diagnostics);
        assert_eq!(result.artifacts.len(), 1, "{}", case.golden);
        let artifact = &result.artifacts[0];
        assert_eq!(artifact.path, case.expected_path, "{}", case.golden);
        assert!(!artifact.binary);
        assert_eq!(artifact.content, golden(&case, &artifact.content));
        assert!(artifact.content.ends_with('\n'));
        assert!(!artifact.content.ends_with("\n\n"));
        validate_json_artifact(&artifact.path, &artifact.content);
        if case.golden == "edge-generic-result.avsc" {
            let value: Value = serde_json::from_str(&artifact.content).unwrap();
            assert_eq!(
                value["morphir.fqname"],
                "acme/customer:customer#lookup-result"
            );
            assert_eq!(value["type"]["morphir.fqname"], "morphir/SDK:result#result");
        }
        if case.golden == "edge-logical-constants.avpr" {
            assert!(
                artifact
                    .content
                    .contains("\"morphir.collection-kind\": \"set\"")
            );
        }
    }
}

#[test]
fn idl_modes_match_reviewed_goldens() {
    for case in idl_cases() {
        let result = generate(&case.package, &case.options);
        assert!(result.success, "{}: {:?}", case.golden, result.diagnostics);
        let artifact = result
            .artifacts
            .iter()
            .find(|artifact| artifact.path == case.expected_path)
            .unwrap_or_else(|| {
                panic!(
                    "{}: missing {}; got {:?}",
                    case.golden,
                    case.expected_path,
                    result
                        .artifacts
                        .iter()
                        .map(|artifact| artifact.path.as_str())
                        .collect::<Vec<_>>()
                )
            });
        assert!(artifact.path.ends_with(".avdl"));
        assert!(!artifact.binary);
        assert_eq!(artifact.content, golden(&case, &artifact.content));
        assert_eq!(artifact.content.matches("protocol ").count(), 1);
        assert!(artifact.content.ends_with('\n'));
        assert!(!artifact.content.ends_with("\n\n"));

        match case.golden {
            "customer-schemas.avdl" => {
                assert!(artifact.content.contains("protocol CustomerSchemas"));
                assert!(!artifact.content.contains("()"));
            }
            "customer-entry-points.avdl" => {
                assert!(artifact.content.starts_with("// Avro Tools 1.12.2 requires `idl --useJavaCC` for message annotations with named responses.\n"));
                assert!(artifact.content.contains("findCustomer("));
                assert!(!artifact.content.contains("schemaVersion("));
            }
            "customer-public.avdl" => {
                assert!(artifact.content.starts_with("// Avro Tools 1.12.2 requires `idl --useJavaCC` for message annotations with named responses.\n"));
                assert!(artifact.content.contains("findCustomer("));
                assert!(artifact.content.contains("schemaVersion("));
                assert!(
                    artifact
                        .content
                        .contains("@morphir.value-kind(\"constant\")")
                );
            }
            "edge-linked.avdl" => {
                assert!(artifact.content.contains("import idl \""));
                assert_eq!(result.artifacts.len(), 2);
                assert!(result.artifacts.iter().all(|artifact| {
                    artifact.path.ends_with(".avdl")
                        && artifact.content.matches("protocol ").count() == 1
                }));
                let dependency = result
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.path == "acme/shared/types/CustomerSchemas.avdl")
                    .expect("linked declaration protocol");
                assert_eq!(
                    dependency.content,
                    golden_content("edge-linked-dependency.avdl", &dependency.content)
                );
            }
            "edge-linked-chain.avdl" => {
                assert_eq!(result.artifacts.len(), 3);
                assert_eq!(artifact.content.matches("import idl ").count(), 2);
                assert_eq!(artifact.content.matches("record ChainOrder").count(), 1);
                for (path, golden_name) in [
                    (
                        "acme/chain/identity/IdentifierSchemas.avdl",
                        "edge-linked-chain-identifier.avdl",
                    ),
                    (
                        "acme/chain/model/CustomerSchemas.avdl",
                        "edge-linked-chain-customer.avdl",
                    ),
                ] {
                    let dependency = result
                        .artifacts
                        .iter()
                        .find(|candidate| candidate.path == path)
                        .unwrap_or_else(|| panic!("missing linked chain artifact {path}"));
                    assert_eq!(
                        dependency.content,
                        golden_content(golden_name, &dependency.content)
                    );
                    assert_eq!(dependency.content.matches("protocol ").count(), 1);
                }
                let customer = result
                    .artifacts
                    .iter()
                    .find(|candidate| candidate.path == "acme/chain/model/CustomerSchemas.avdl")
                    .unwrap();
                assert_eq!(customer.content.matches("import idl ").count(), 1);
                assert_eq!(customer.content.matches("record Customer").count(), 1);
                assert!(!customer.content.contains("record Identifier"));
            }
            "edge-custom-types.avdl" => {
                assert!(artifact.content.starts_with("// Avro Tools 1.12.2 requires `idl --useJavaCC` for message annotations with named responses.\n"));
                assert!(
                    artifact
                        .content
                        .contains("@morphir.collection-kind(\"set\")")
                );
                assert!(artifact.content.contains("decimal(38, 10)"));
                assert!(artifact.content.contains("date"));
                assert!(
                    artifact
                        .content
                        .contains("@logicalType(\"time-micros\") long")
                );
                assert!(
                    artifact
                        .content
                        .contains("@logicalType(\"timestamp-micros\") long")
                );
                assert!(artifact.content.contains("uuid identifier"));
                assert!(artifact.content.contains("union { null, string } nickname"));
                assert!(artifact.content.contains("map<string> labels"));
                assert!(
                    artifact
                        .content
                        .contains("@morphir.type(\"Char\") string initial")
                );
                assert!(artifact.content.contains("string `record`;"));
                assert!(artifact.content.contains("string `error`();"));
                assert!(
                    artifact
                        .content
                        .contains("enum Status { Active, Inactive }")
                );
                assert!(artifact.content.contains("record Shape {"));
                assert!(artifact.content.contains(
                    "union { acme.customer.customer.Shape.Circle, acme.customer.customer.Shape.Point } value;"
                ));
                assert!(artifact.content.contains("record Circle {"));
                assert!(artifact.content.contains("long radius;"));
                assert!(artifact.content.contains(
                    "@logicalType(\"date\") @morphir.fqname(\"acme/customer:customer#legacy-date\") long mappedDate;"
                ));
                assert!(artifact.content.contains(
                    "@logicalType(\"uuid\") @morphir.fqname(\"acme/customer:customer#binary-id\") bytes mappedIdentifier;"
                ));
                assert!(artifact.content.contains(
                    "@morphir.fqname(\"acme/customer:customer#money\") decimal(20, 4) mappedAmount;"
                ));
            }
            "edge-escaping.avdl" => {
                assert!(artifact.content.starts_with("// Avro Tools 1.12.2 requires `idl --useJavaCC` for message annotations with named responses.\n"));
                assert!(artifact.content.contains("Protocol * / docs\\path"));
                assert!(artifact.content.contains("control\\u0001line"));
                assert!(
                    artifact.content.contains(
                        "@morphir.doc(\"Protocol */ docs\\\\path\\ncontrol\\u0001line\")"
                    )
                );
                assert!(
                    artifact
                        .content
                        .contains("@morphir.entry-point-doc(\"Entry \\\\ docs\\ncontrol\\u0002\")")
                );
            }
            "edge-primitive-protocol.avdl" => {
                assert!(!artifact.content.starts_with("// Avro Tools 1.12.2"));
                assert!(artifact.content.contains("string primitiveResponse();"));
            }
            _ => unreachable!("known IDL golden"),
        }
    }
}

#[test]
fn idl_schema_mode_wraps_an_inline_alias_as_a_named_root() {
    let result = generate(
        &package(vec![alias(
            "acme/customer:customer#customer-label",
            "customer-label",
            reference(STRING, vec![]),
        )]),
        &AvroOptions {
            representation: morphir_avro_extension::Representation::Idl,
            ..AvroOptions::default()
        },
    );
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.artifacts.len(), 1);
    let artifact = &result.artifacts[0];
    assert_eq!(
        artifact.path,
        "acme/customer/customer/CustomerLabelSchemas.avdl"
    );
    assert!(artifact.content.contains("record CustomerLabel {"));
    assert!(artifact.content.contains("string value;"));
    assert_eq!(artifact.content.matches("protocol ").count(), 1);
}

#[test]
fn idl_output_is_byte_identical_across_input_order() {
    let first = customer_package();
    let mut second = first.clone();
    second.modules.reverse();
    second.modules[0].types.reverse();
    second.modules[0].values.reverse();
    let options = AvroOptions {
        representation: morphir_avro_extension::Representation::Idl,
        projection: Projection::ProtocolPublic,
        ..AvroOptions::default()
    };

    let first = generate(&first, &options);
    let second = generate(&second, &options);
    assert_eq!(
        artifact_pairs(&first.artifacts),
        artifact_pairs(&second.artifacts)
    );
    assert_eq!(
        diagnostic_keys(&first.diagnostics),
        diagnostic_keys(&second.diagnostics)
    );
}

#[test]
fn idl_reports_mutual_and_cross_ownership_cycles_without_artifacts() {
    for input in [
        mutually_recursive_schema_package(),
        linked_cross_ownership_cycle_package(),
    ] {
        let result = generate(
            &input,
            &AvroOptions {
                representation: morphir_avro_extension::Representation::Idl,
                dependencies: morphir_avro_extension::Dependencies::Linked,
                ..AvroOptions::default()
            },
        );
        assert!(!result.success);
        assert!(result.artifacts.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code.as_deref(), Some("AVRO005"));
    }
}

#[test]
fn json_output_is_byte_identical_across_input_order() {
    let first = customer_package();
    let mut second = first.clone();
    second.modules.reverse();
    second.modules[0].types.reverse();
    second.modules[0].values.reverse();

    let options = options(Projection::ProtocolPublic);
    let first = generate(&first, &options);
    let second = generate(&second, &options);
    assert_eq!(
        artifact_pairs(&first.artifacts),
        artifact_pairs(&second.artifacts)
    );
    assert_eq!(
        diagnostic_keys(&first.diagnostics),
        diagnostic_keys(&second.diagnostics)
    );
}

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
fn golden_update_mode_requires_exact_one_and_refuses_ci() {
    assert_eq!(golden_update_mode(None, None), Ok(false));
    assert_eq!(golden_update_mode(Some(OsStr::new("0")), None), Ok(false));
    assert_eq!(
        golden_update_mode(Some(OsStr::new("false")), None),
        Ok(false)
    );
    assert_eq!(golden_update_mode(Some(OsStr::new("1")), None), Ok(true));
    assert_eq!(
        golden_update_mode(Some(OsStr::new("1")), Some(OsStr::new("true"))),
        Err("refusing to update goldens in CI")
    );
}

#[test]
fn protocol_registry_validation_resolves_request_response_and_error_names() {
    validate_protocol_registry(&serde_json::json!({
        "types": [{
            "type": "record",
            "name": "Customer",
            "namespace": "acme.customer",
            "fields": []
        }],
        "messages": {
            "exchange": {
                "request": [{"name": "customer", "type": "acme.customer.Customer"}],
                "response": "acme.customer.Customer",
                "errors": ["acme.customer.Customer"]
            }
        }
    }));
}

#[test]
fn protocol_registry_validation_handles_direct_array_map_and_wrapped_types() {
    validate_protocol_registry(&serde_json::json!({
        "types": [{
            "type": "record",
            "name": "Customer",
            "namespace": "acme.customer",
            "fields": []
        }],
        "messages": {
            "exchange": {
                "request": [{
                    "name": "customers",
                    "type": {
                        "type": "array",
                        "items": "acme.customer.Customer",
                        "morphir.collection-kind": "set"
                    }
                }],
                "response": {
                    "type": "map",
                    "values": "acme.customer.Customer"
                },
                "errors": [{
                    "type": {
                        "type": "array",
                        "items": "acme.customer.Customer"
                    },
                    "morphir.wrapper-kind": "annotated"
                }]
            }
        }
    }));
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

fn json_cases() -> Vec<GoldenCase> {
    vec![
        GoldenCase {
            golden: "customer-schemas.avsc",
            expected_path: "acme/customer/customer/Customer.avsc",
            package: package(vec![documented_customer_record()]),
            options: AvroOptions::default(),
        },
        GoldenCase {
            golden: "customer-entry-points.avpr",
            expected_path: "acme/customer/Customer.avpr",
            package: customer_package(),
            options: options(Projection::ProtocolEntryPoints),
        },
        GoldenCase {
            golden: "customer-public.avpr",
            expected_path: "acme/customer/Customer.avpr",
            package: customer_package(),
            options: options(Projection::ProtocolPublic),
        },
        GoldenCase {
            golden: "edge-alias-wrapper.avsc",
            expected_path: "acme/customer/customer/CustomerLabels.avsc",
            package: alias_wrapper_package(),
            options: AvroOptions {
                aliases: Aliases::WrapperRecord,
                ..AvroOptions::default()
            },
        },
        GoldenCase {
            golden: "edge-generic-result.avsc",
            expected_path: "acme/customer/customer/LookupResult.avsc",
            package: generic_result_package(),
            options: AvroOptions::default(),
        },
        GoldenCase {
            golden: "edge-logical-constants.avpr",
            expected_path: "acme/customer/Customer.avpr",
            package: logical_constants_package(),
            options: options(Projection::ProtocolPublic),
        },
        GoldenCase {
            golden: "edge-partial.avsc",
            expected_path: "acme/customer/customer/Supported.avsc",
            package: partial_package(),
            options: AvroOptions {
                unsupported: Unsupported::WarnAndSkip,
                ..AvroOptions::default()
            },
        },
    ]
}

fn idl_cases() -> Vec<GoldenCase> {
    let idl = |projection| AvroOptions {
        representation: morphir_avro_extension::Representation::Idl,
        projection,
        ..AvroOptions::default()
    };
    vec![
        GoldenCase {
            golden: "customer-schemas.avdl",
            expected_path: "acme/customer/customer/CustomerSchemas.avdl",
            package: package(vec![documented_customer_record()]),
            options: idl(Projection::Schemas),
        },
        GoldenCase {
            golden: "customer-entry-points.avdl",
            expected_path: "acme/customer/Customer.avdl",
            package: customer_package(),
            options: idl(Projection::ProtocolEntryPoints),
        },
        GoldenCase {
            golden: "customer-public.avdl",
            expected_path: "acme/customer/Customer.avdl",
            package: customer_package(),
            options: idl(Projection::ProtocolPublic),
        },
        GoldenCase {
            golden: "edge-linked.avdl",
            expected_path: "acme/customer/customer/OrderSchemas.avdl",
            package: linked_package(),
            options: AvroOptions {
                representation: morphir_avro_extension::Representation::Idl,
                dependencies: morphir_avro_extension::Dependencies::Linked,
                ..AvroOptions::default()
            },
        },
        GoldenCase {
            golden: "edge-linked-chain.avdl",
            expected_path: "acme/customer/customer/ChainOrderSchemas.avdl",
            package: idl_linked_chain_package(),
            options: AvroOptions {
                representation: morphir_avro_extension::Representation::Idl,
                dependencies: morphir_avro_extension::Dependencies::Linked,
                ..AvroOptions::default()
            },
        },
        GoldenCase {
            golden: "edge-custom-types.avdl",
            expected_path: "acme/customer/Customer.avdl",
            package: idl_custom_types_package(),
            options: idl_custom_types_options(),
        },
        GoldenCase {
            golden: "edge-escaping.avdl",
            expected_path: "acme/customer/Customer.avdl",
            package: idl_escaping_package(),
            options: idl(Projection::ProtocolPublic),
        },
        GoldenCase {
            golden: "edge-primitive-protocol.avdl",
            expected_path: "acme/customer/Customer.avdl",
            package: idl_primitive_protocol_package(),
            options: idl(Projection::ProtocolPublic),
        },
    ]
}

fn customer_package() -> ProjectionPackage {
    let mut input = package(vec![documented_customer_record()]);
    input.kind = DistributionKind::Application;
    input.modules[0].doc = Some("Customer operations.".to_owned());
    input.modules[0].values = vec![
        value_specification(
            "acme/customer:customer#find-customer",
            "find-customer",
            vec![field("id", reference(STRING, vec![]))],
            Some(reference(CUSTOMER, vec![])),
            ValueKind::Function,
            Some(EntryPointMetadata {
                identifier: "customer-query".to_owned(),
                kind: EntryPointKind::Command,
                doc: Some("Query a customer by ID.".to_owned()),
            }),
        ),
        value_specification(
            "acme/customer:customer#schema-version",
            "schema-version",
            vec![],
            Some(reference(STRING, vec![])),
            ValueKind::Constant,
            None,
        ),
    ];
    input
}

fn documented_customer_record() -> TypeDeclaration {
    let mut declaration = customer_record();
    let TypeDeclaration::Alias { doc, .. } = &mut declaration else {
        unreachable!("customer fixture is an alias")
    };
    *doc = Some("A customer record.".to_owned());
    declaration
}

fn alias_wrapper_package() -> ProjectionPackage {
    package(vec![alias(
        "acme/customer:customer#customer-labels",
        "customer-labels",
        reference("morphir/SDK:list#list", vec![reference(STRING, vec![])]),
    )])
}

fn generic_result_package() -> ProjectionPackage {
    package(vec![alias(
        "acme/customer:customer#lookup-result",
        "lookup-result",
        reference(
            RESULT,
            vec![reference(STRING, vec![]), reference(STRING, vec![])],
        ),
    )])
}

fn logical_constants_package() -> ProjectionPackage {
    let mut input = package(vec![alias(
        "acme/customer:customer#logical-values",
        "logical-values",
        TypeExpr::Record(vec![
            field("as-of", reference(LOCAL_DATE, vec![])),
            field("amount", reference(DECIMAL, vec![])),
            field("tags", reference(SET, vec![reference(STRING, vec![])])),
        ]),
    )]);
    input.kind = DistributionKind::Application;
    input.modules[0].values = vec![value_specification(
        "acme/customer:customer#logical-defaults",
        "logical-defaults",
        vec![],
        Some(reference("acme/customer:customer#logical-values", vec![])),
        ValueKind::Constant,
        None,
    )];
    input
}

fn idl_custom_types_package() -> ProjectionPackage {
    let custom_values = alias(
        "acme/customer:customer#custom-values",
        "custom-values",
        TypeExpr::Record(vec![
            field("amount", reference(DECIMAL, vec![])),
            field("as-of", reference(LOCAL_DATE, vec![])),
            field("identifier", reference(UUID, vec![])),
            field(
                "mapped-date",
                reference("acme/customer:customer#legacy-date", vec![]),
            ),
            field(
                "mapped-identifier",
                reference("acme/customer:customer#binary-id", vec![]),
            ),
            field(
                "mapped-amount",
                reference("acme/customer:customer#money", vec![]),
            ),
            field("initial", reference(CHAR, vec![])),
            field(
                "labels",
                reference(
                    DICT,
                    vec![reference(STRING, vec![]), reference(STRING, vec![])],
                ),
            ),
            field(
                "nickname",
                reference(MAYBE, vec![reference(STRING, vec![])]),
            ),
            field("observed-at", reference(INSTANT, vec![])),
            field("opens-at", reference(LOCAL_TIME, vec![])),
            field("record", reference(STRING, vec![])),
            field("tags", reference(SET, vec![reference(STRING, vec![])])),
        ]),
    );
    let status = TypeDeclaration::Custom {
        source_name: "acme/customer:customer#status".to_owned(),
        name: "status".to_owned(),
        type_params: Vec::new(),
        constructors: vec![
            Constructor {
                source_name: "acme/customer:customer#active".to_owned(),
                name: "active".to_owned(),
                arguments: Vec::new(),
            },
            Constructor {
                source_name: "acme/customer:customer#inactive".to_owned(),
                name: "inactive".to_owned(),
                arguments: Vec::new(),
            },
        ],
        doc: Some("Customer status.".to_owned()),
    };
    let shape = TypeDeclaration::Custom {
        source_name: "acme/customer:customer#shape".to_owned(),
        name: "shape".to_owned(),
        type_params: Vec::new(),
        constructors: vec![
            Constructor {
                source_name: "acme/customer:customer#point".to_owned(),
                name: "point".to_owned(),
                arguments: Vec::new(),
            },
            Constructor {
                source_name: "acme/customer:customer#circle".to_owned(),
                name: "circle".to_owned(),
                arguments: vec![field("radius", reference("morphir/SDK:basics#int", vec![]))],
            },
        ],
        doc: Some("A shape with payload constructors.".to_owned()),
    };
    let mapped = [
        ("acme/customer:customer#legacy-date", "legacy-date"),
        ("acme/customer:customer#binary-id", "binary-id"),
        ("acme/customer:customer#money", "money"),
    ]
    .map(|(source_name, name)| TypeDeclaration::Opaque {
        source_name: source_name.to_owned(),
        name: name.to_owned(),
        type_params: Vec::new(),
        doc: None,
    });
    let mut input = package(
        [custom_values, status, shape]
            .into_iter()
            .chain(mapped)
            .collect(),
    );
    input.kind = DistributionKind::Application;
    input.modules[0].values = vec![
        value_specification(
            "acme/customer:customer#custom-defaults",
            "custom-defaults",
            vec![],
            Some(reference("acme/customer:customer#custom-values", vec![])),
            ValueKind::Constant,
            None,
        ),
        value_specification(
            "acme/customer:customer#error",
            "error",
            vec![],
            Some(reference(STRING, vec![])),
            ValueKind::Function,
            None,
        ),
    ];
    input
}

fn idl_custom_types_options() -> AvroOptions {
    let mut options = AvroOptions {
        representation: morphir_avro_extension::Representation::Idl,
        projection: Projection::ProtocolPublic,
        ..AvroOptions::default()
    };
    for (source, physical_type, logical_type, precision, scale) in [
        (
            "acme/customer:customer#legacy-date",
            "long",
            "date",
            None,
            None,
        ),
        (
            "acme/customer:customer#binary-id",
            "bytes",
            "uuid",
            None,
            None,
        ),
        (
            "acme/customer:customer#money",
            "bytes",
            "decimal",
            Some(20),
            Some(4),
        ),
    ] {
        options.type_mappings.insert(
            source.to_owned(),
            TypeMapping {
                physical_type: physical_type.to_owned(),
                logical_type: Some(logical_type.to_owned()),
                precision,
                scale,
            },
        );
    }
    options
}

fn idl_escaping_package() -> ProjectionPackage {
    let mut input = customer_package();
    input.modules[0].doc = Some("Protocol */ docs\\path\ncontrol\u{0001}line".to_owned());
    let TypeDeclaration::Alias { doc, .. } = &mut input.modules[0].types[0] else {
        unreachable!("customer is an alias")
    };
    *doc = Some("Record */ docs\\path\ncontrol\u{0003}line".to_owned());
    input.modules[0].values[0].doc = Some("Message */ docs\\path\ncontrol\u{0004}line".to_owned());
    input.modules[0].values[0]
        .entry_point
        .as_mut()
        .expect("entry point")
        .doc = Some("Entry \\ docs\ncontrol\u{0002}".to_owned());
    input
}

fn idl_primitive_protocol_package() -> ProjectionPackage {
    let mut input = package(Vec::new());
    input.kind = DistributionKind::Application;
    input.modules[0].values = vec![value_specification(
        "acme/customer:customer#primitive-response",
        "primitive-response",
        Vec::new(),
        Some(reference(STRING, vec![])),
        ValueKind::Function,
        None,
    )];
    input
}

fn idl_linked_chain_package() -> ProjectionPackage {
    let mut input = package(vec![alias(
        "acme/customer:customer#chain-order",
        "chain-order",
        TypeExpr::Record(vec![field(
            "customer",
            reference("acme/chain:model#customer", vec![]),
        )]),
    )]);
    input.dependencies = vec![ProjectionDependency {
        package_name: "acme/chain".to_owned(),
        modules: vec![
            ProjectionModule {
                path: vec!["identity".to_owned()],
                types: vec![alias(
                    "acme/chain:identity#identifier",
                    "identifier",
                    TypeExpr::Record(vec![field("value", reference(STRING, vec![]))]),
                )],
                values: Vec::new(),
                doc: None,
            },
            ProjectionModule {
                path: vec!["model".to_owned()],
                types: vec![alias(
                    "acme/chain:model#customer",
                    "customer",
                    TypeExpr::Record(vec![field(
                        "identifier",
                        reference("acme/chain:identity#identifier", vec![]),
                    )]),
                )],
                values: Vec::new(),
                doc: None,
            },
        ],
    }];
    input
}

fn partial_package() -> ProjectionPackage {
    package(vec![
        alias(
            "acme/customer:customer#supported",
            "supported",
            reference(STRING, vec![]),
        ),
        alias(
            "acme/customer:customer#unsupported",
            "unsupported",
            TypeExpr::Function {
                input: Box::new(reference(STRING, vec![])),
                output: Box::new(reference(STRING, vec![])),
            },
        ),
    ])
}

fn linked_package() -> ProjectionPackage {
    let mut input = package(vec![alias(
        "acme/customer:customer#order",
        "order",
        TypeExpr::Record(vec![field(
            "customer",
            reference("acme/shared:types#customer", vec![]),
        )]),
    )]);
    input.dependencies = vec![ProjectionDependency {
        package_name: "acme/shared".to_owned(),
        modules: vec![ProjectionModule {
            path: vec!["types".to_owned()],
            types: vec![alias(
                "acme/shared:types#customer",
                "customer",
                TypeExpr::Record(vec![field("id", reference(STRING, vec![]))]),
            )],
            values: Vec::new(),
            doc: None,
        }],
    }];
    input
}

fn linked_alias_package() -> ProjectionPackage {
    let mut input = package(vec![alias(
        "acme/customer:customer#customer-alias",
        "customer-alias",
        reference("acme/shared:types#customer", vec![]),
    )]);
    input.dependencies = linked_package().dependencies;
    input
}

fn linked_chain_package() -> ProjectionPackage {
    let mut input = package(vec![alias(
        "acme/customer:customer#order",
        "order",
        TypeExpr::Record(vec![field(
            "customer",
            reference("acme/shared:types#customer", vec![]),
        )]),
    )]);
    input.dependencies = vec![ProjectionDependency {
        package_name: "acme/shared".to_owned(),
        modules: vec![ProjectionModule {
            path: vec!["types".to_owned()],
            types: vec![
                alias(
                    "acme/shared:types#customer",
                    "customer",
                    TypeExpr::Record(vec![field(
                        "identifier",
                        reference("acme/shared:types#identifier", vec![]),
                    )]),
                ),
                alias(
                    "acme/shared:types#identifier",
                    "identifier",
                    TypeExpr::Record(vec![field("value", reference(STRING, vec![]))]),
                ),
            ],
            values: Vec::new(),
            doc: None,
        }],
    }];
    input
}

fn linked_cross_ownership_cycle_package() -> ProjectionPackage {
    let dependency = "acme/shared:types#dependency";
    let owned_shared = "acme/customer:customer#z-owned-shared";
    let mut input = package(vec![
        alias(
            "acme/customer:customer#a-root",
            "a-root",
            TypeExpr::Record(vec![field("dependency", reference(dependency, vec![]))]),
        ),
        alias(
            owned_shared,
            "z-owned-shared",
            TypeExpr::Record(vec![field("cycle", reference(dependency, vec![]))]),
        ),
    ]);
    input.dependencies = vec![ProjectionDependency {
        package_name: "acme/shared".to_owned(),
        modules: vec![ProjectionModule {
            path: vec!["types".to_owned()],
            types: vec![alias(
                dependency,
                "dependency",
                TypeExpr::Record(vec![field("owned", reference(owned_shared, vec![]))]),
            )],
            values: Vec::new(),
            doc: None,
        }],
    }];
    input
}

fn acyclic_protocol_package() -> ProjectionPackage {
    let identifier = alias(
        "acme/customer:customer#identifier",
        "identifier",
        TypeExpr::Record(vec![field("value", reference(STRING, vec![]))]),
    );
    let customer = alias(
        "acme/customer:customer#customer",
        "customer",
        TypeExpr::Record(vec![field(
            "identifier",
            reference("acme/customer:customer#identifier", vec![]),
        )]),
    );
    protocol_package(
        vec![identifier, customer],
        vec![field(
            "customer",
            reference("acme/customer:customer#customer", vec![]),
        )],
        reference("acme/customer:customer#identifier", vec![]),
    )
}

fn mutually_recursive_protocol_package() -> ProjectionPackage {
    let a = alias(
        "acme/customer:customer#a",
        "a",
        TypeExpr::Record(vec![field(
            "b",
            reference("acme/customer:customer#b", vec![]),
        )]),
    );
    let b = alias(
        "acme/customer:customer#b",
        "b",
        TypeExpr::Record(vec![field(
            "a",
            reference("acme/customer:customer#a", vec![]),
        )]),
    );
    protocol_package(
        vec![a, b],
        vec![field(
            "input",
            reference("acme/customer:customer#a", vec![]),
        )],
        reference("acme/customer:customer#b", vec![]),
    )
}

fn mutually_recursive_schema_package() -> ProjectionPackage {
    package(vec![
        alias(
            "acme/customer:customer#a",
            "a",
            TypeExpr::Record(vec![field(
                "b",
                reference("acme/customer:customer#b", vec![]),
            )]),
        ),
        alias(
            "acme/customer:customer#b",
            "b",
            TypeExpr::Record(vec![field(
                "a",
                reference("acme/customer:customer#a", vec![]),
            )]),
        ),
    ])
}

fn linked_self_loop_package() -> ProjectionPackage {
    let node = "acme/shared:types#node";
    let mut input = package(vec![alias(
        "acme/customer:customer#root",
        "root",
        TypeExpr::Record(vec![field("node", reference(node, vec![]))]),
    )]);
    input.dependencies = vec![ProjectionDependency {
        package_name: "acme/shared".to_owned(),
        modules: vec![ProjectionModule {
            path: vec!["types".to_owned()],
            types: vec![alias(
                node,
                "node",
                TypeExpr::Record(vec![field("next", reference(node, vec![]))]),
            )],
            values: Vec::new(),
            doc: None,
        }],
    }];
    input
}

fn protocol_package(
    types: Vec<TypeDeclaration>,
    inputs: Vec<morphir_avro_extension::NamedType>,
    output: TypeExpr,
) -> ProjectionPackage {
    let mut input = package(types);
    input.kind = DistributionKind::Application;
    input.modules[0].values = vec![value_specification(
        "acme/customer:customer#exchange",
        "exchange",
        inputs,
        Some(output),
        ValueKind::Function,
        None,
    )];
    input
}

fn options(projection: Projection) -> AvroOptions {
    AvroOptions {
        projection,
        ..AvroOptions::default()
    }
}

fn golden(case: &GoldenCase, actual: &str) -> String {
    golden_content(case.golden, actual)
}

fn golden_content(name: &str, actual: &str) -> String {
    let path = golden_path(name);
    let update = golden_update_mode(
        env::var_os("UPDATE_GOLDEN").as_deref(),
        env::var_os("CI").as_deref(),
    )
    .unwrap_or_else(|message| panic!("{message}"));
    if update {
        fs::create_dir_all(path.parent().expect("golden path has a parent"))
            .expect("create golden directory");
        fs::write(&path, actual).expect("write golden");
    }
    fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "read {}: {error}; review with UPDATE_GOLDEN=1",
            path.display()
        )
    })
}

fn golden_update_mode(
    update_golden: Option<&OsStr>,
    ci: Option<&OsStr>,
) -> Result<bool, &'static str> {
    let update = update_golden == Some(OsStr::new("1"));
    if update && ci.is_some() {
        return Err("refusing to update goldens in CI");
    }
    Ok(update)
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

fn validate_json_artifact(path: &str, content: &str) {
    let value: Value = serde_json::from_str(content).expect("valid JSON");
    if path.ends_with(".avsc") {
        Schema::parse_str(content).expect("Apache Avro accepts schema");
        return;
    }

    let object = value.as_object().expect("protocol JSON object");
    assert!(object.get("protocol").is_some());
    assert!(object.get("namespace").is_some());
    validate_protocol_registry(&value);
    let messages = object["messages"]
        .as_object()
        .expect("protocol messages object");
    for message in messages.values() {
        assert!(message["request"].is_array());
        assert!(message.get("response").is_some());
        assert_eq!(message["errors"], Value::Array(Vec::new()));
    }
}

fn only_protocol_json(artifacts: &[morphir_extension_sdk::Artifact]) -> Value {
    let protocol = artifacts
        .iter()
        .find(|artifact| artifact.path.ends_with(".avpr"))
        .expect("protocol artifact");
    serde_json::from_str(&protocol.content).expect("valid protocol JSON")
}

fn validate_protocol_registry(protocol: &Value) {
    validate_protocol_registry_with_linked(protocol, std::iter::empty::<&str>());
}

fn validate_protocol_registry_with_linked<'a>(
    protocol: &Value,
    linked_schemas: impl IntoIterator<Item = &'a str>,
) {
    let types = protocol["types"].as_array().expect("protocol types array");
    let mut type_strings = linked_schemas
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    type_strings.extend(types.iter().map(Value::to_string));
    let parsed = Schema::parse_list(&type_strings).expect("Apache Avro accepts protocol type set");
    let registry = ResolvedSchema::new_with_schemata(parsed.iter().collect())
        .expect("Apache Avro resolves protocol type registry");
    let messages = protocol["messages"]
        .as_object()
        .expect("protocol messages object");
    for message in messages.values() {
        for field in message["request"].as_array().expect("request fields") {
            resolve_protocol_type(&field["type"], registry.get_names());
        }
        resolve_protocol_type(&message["response"], registry.get_names());
        for error in message["errors"].as_array().expect("error types") {
            resolve_protocol_type(error, registry.get_names());
        }
    }
}

fn resolve_protocol_type(tpe: &Value, registry: &NamesRef<'_>) {
    match tpe {
        Value::String(name) if !is_primitive(name) => {
            let name = Name::new(name).expect("valid Avro named reference");
            assert!(
                registry.contains_key(&name),
                "protocol reference {} is absent from the parsed type registry",
                name.fullname(None)
            );
        }
        Value::Array(branches) => {
            for branch in branches {
                resolve_protocol_type(branch, registry);
            }
        }
        Value::Object(object) => match object.get("type") {
            Some(Value::String(kind)) if kind == "array" => {
                resolve_protocol_type(&object["items"], registry);
            }
            Some(Value::String(kind)) if kind == "map" => {
                resolve_protocol_type(&object["values"], registry);
            }
            Some(physical) => resolve_protocol_type(physical, registry),
            None => {
                if let Some(items) = object.get("items") {
                    resolve_protocol_type(items, registry);
                }
                if let Some(values) = object.get("values") {
                    resolve_protocol_type(values, registry);
                }
            }
        },
        Value::String(_) | Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn is_primitive(name: &str) -> bool {
    matches!(
        name,
        "null" | "boolean" | "int" | "long" | "float" | "double" | "bytes" | "string"
    )
}

fn collect_named_definitions(value: &Value, names: &mut std::collections::BTreeSet<String>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_named_definitions(value, names);
            }
        }
        Value::Object(object) => {
            if matches!(
                object.get("type").and_then(Value::as_str),
                Some("record" | "enum" | "fixed")
            ) {
                let name = object["name"].as_str().expect("named declaration name");
                let namespace = object
                    .get("namespace")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let full_name = if namespace.is_empty() {
                    name.to_owned()
                } else {
                    format!("{namespace}.{name}")
                };
                assert!(names.insert(full_name), "duplicate named declaration");
            }
            for nested in object.values() {
                collect_named_definitions(nested, names);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn artifact_pairs(artifacts: &[morphir_extension_sdk::Artifact]) -> Vec<(&str, &str)> {
    artifacts
        .iter()
        .map(|artifact| (artifact.path.as_str(), artifact.content.as_str()))
        .collect()
}

fn diagnostic_keys(
    diagnostics: &[morphir_extension_sdk::Diagnostic],
) -> Vec<(
    morphir_extension_sdk::DiagnosticSeverity,
    Option<&str>,
    &str,
)> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.severity,
                diagnostic.code.as_deref(),
                diagnostic.message.as_str(),
            )
        })
        .collect()
}
