use super::*;
use pretty_assertions::assert_eq;

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
