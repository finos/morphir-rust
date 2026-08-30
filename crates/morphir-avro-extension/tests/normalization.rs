mod support;

use morphir_avro_extension::{
    DistributionKind, EntryPointKind, IncompletenessKind, TypeDeclaration, TypeExpr, ValueKind,
    normalize,
};
use pretty_assertions::assert_eq;
use serde_json::Value;
use support::mothers;

#[test]
fn v3_and_v4_libraries_normalize_to_the_same_public_surface() {
    let v3 = normalize(&mothers::classic_customer_library()).unwrap();
    let v4 = normalize(&mothers::v4_customer_library()).unwrap();

    assert_eq!(v3, v4);
    assert_eq!(v3.kind, DistributionKind::Library);
    assert_eq!(v3.package_name, "acme/customer");
}

#[test]
fn v3_and_v4_dependencies_normalize_equivalently_in_package_order() {
    let v3 = normalize(&mothers::classic_customer_library()).unwrap();
    let v4 = normalize(&mothers::v4_customer_library()).unwrap();

    assert_eq!(v3.dependencies, v4.dependencies);
    assert_eq!(
        v3.dependencies
            .iter()
            .map(|dependency| dependency.package_name.as_str())
            .collect::<Vec<_>>(),
        ["shared/a", "shared/z"]
    );
    assert_eq!(v3.dependencies[0].modules[0].types[0].name(), "a-id");
    assert_eq!(v3.dependencies[1].modules[0].values[0].name, "lookup");
}

#[test]
fn supported_baseline_version_spellings_select_the_exact_decoder() {
    for (mut ir, version) in [
        (mothers::classic_customer_library(), serde_json::json!(3)),
        (
            mothers::classic_customer_library(),
            serde_json::json!("3.0.0"),
        ),
        (mothers::v4_customer_library(), serde_json::json!(4)),
        (mothers::v4_customer_library(), serde_json::json!("4.0.0")),
    ] {
        ir["formatVersion"] = version;
        normalize(&ir).unwrap();
    }
}

#[test]
fn format_version_errors_are_classified_before_distribution_decoding() {
    for (version, code) in [
        (serde_json::json!("4.x"), "invalid_format_version_syntax"),
        (serde_json::json!("04.0.0"), "invalid_format_version_syntax"),
        (
            serde_json::json!("3.4294967296.0"),
            "format_version_out_of_range",
        ),
        (
            serde_json::json!("3.1.0"),
            "unsupported_format_version_revision",
        ),
        (
            serde_json::json!("4.1.0"),
            "unsupported_format_version_revision",
        ),
        (serde_json::json!(5), "unsupported_format_version_major"),
        (serde_json::json!(true), "invalid_format_version_type"),
    ] {
        let mut ir = serde_json::json!({
            "formatVersion": version,
            "distribution": "not a distribution"
        });
        let error = normalize(&ir).unwrap_err();
        assert_eq!(error.code(), code, "version was {}", ir["formatVersion"]);
        ir["distribution"] = serde_json::Value::Null;
        assert_eq!(normalize(&ir).unwrap_err().code(), code);
    }
}

#[test]
fn v4_application_marks_only_declared_entry_points() {
    let package = normalize(&mothers::v4_customer_application()).unwrap();
    let values = &package.modules[0].values;
    let find_customer = values
        .iter()
        .find(|value| value.name == "find-customer")
        .unwrap();
    let default_customer = values
        .iter()
        .find(|value| value.name == "default-customer")
        .unwrap();

    assert_eq!(package.kind, DistributionKind::Application);
    let entry_point = find_customer.entry_point.as_ref().unwrap();
    assert_eq!(entry_point.identifier, "customer-query");
    assert_eq!(entry_point.kind, EntryPointKind::Command);
    assert_eq!(entry_point.doc.as_deref(), Some("Application command."));
    assert_eq!(default_customer.entry_point, None);
}

#[test]
fn incomplete_public_entry_points_remain_available_to_unsupported_policy() {
    let package = normalize(&mothers::v4_customer_application()).unwrap();
    let unfinished = package.modules[0]
        .values
        .iter()
        .find(|value| value.name == "unfinished")
        .unwrap();

    assert_eq!(
        unfinished.entry_point.as_ref().map(|entry| entry.kind),
        Some(EntryPointKind::Handler)
    );
    assert!(unfinished.output.is_none());
    assert_eq!(unfinished.doc.as_deref(), Some("An incomplete handler."));
}

#[test]
fn entry_point_targets_must_be_canonical_public_values() {
    for (target, expected_reason) in [
        ("not-an-fqname", "invalid"),
        ("acme/customer:domain#findCustomer", "invalid"),
        ("acme/customer:domain#missing", "dangling"),
        ("acme/customer:domain#helper", "private"),
        ("acme/customer:private-module#hidden", "private"),
    ] {
        let ir = mothers::v4_customer_application_with_entry_points(serde_json::json!({
            "candidate": { "target": target, "kind": "command" }
        }));
        let error = normalize(&ir).unwrap_err();
        assert_eq!(error.code(), "invalid_entry_point_target");
        assert!(error.to_string().contains(expected_reason), "{error}");
    }
}

#[test]
fn duplicate_entry_point_target_errors_do_not_depend_on_identifier_order() {
    let first = mothers::v4_customer_application_with_entry_points(serde_json::json!({
        "z-command": {
            "target": "acme/customer:domain#find-customer",
            "kind": "command"
        },
        "a-command": {
            "target": "acme/customer:domain#find-customer",
            "kind": "handler"
        }
    }));
    let second = mothers::v4_customer_application_with_entry_points(serde_json::json!({
        "a-command": {
            "target": "acme/customer:domain#find-customer",
            "kind": "handler"
        },
        "z-command": {
            "target": "acme/customer:domain#find-customer",
            "kind": "command"
        }
    }));

    let first = normalize(&first).unwrap_err();
    let second = normalize(&second).unwrap_err();
    assert_eq!(first.code(), "duplicate_entry_point_target");
    assert_eq!(first.to_string(), second.to_string());
}

#[test]
fn normalization_discards_value_bodies() {
    let json = serde_json::to_value(normalize(&mothers::v4_customer_library()).unwrap()).unwrap();

    assert_eq!(find_key(&json, "body"), None);
    assert_eq!(find_key(&json, "partialBody"), None);
}

#[test]
fn v4_specs_preserve_docs_and_flatten_unnamed_curried_inputs() {
    let package = normalize(&mothers::v4_customer_specs()).unwrap();
    let module = &package.modules[0];
    let curried = module
        .values
        .iter()
        .find(|value| value.name == "curried")
        .unwrap();

    assert_eq!(package.kind, DistributionKind::Specs);
    assert_eq!(module.doc.as_deref(), Some("Customer\nspecifications."));
    assert_eq!(curried.doc.as_deref(), Some("A curried signature."));
    assert!(matches!(module.types[1], TypeDeclaration::Opaque { .. }));
    assert_eq!(
        curried
            .inputs
            .iter()
            .map(|input| input.name.as_str())
            .collect::<Vec<_>>(),
        ["arg1", "arg2"]
    );
    assert_eq!(curried.value_kind, ValueKind::Function);
}

#[test]
fn synthesized_curried_input_names_skip_explicit_collisions() {
    let package = normalize(&mothers::v4_customer_specs()).unwrap();
    let value = package.modules[0]
        .values
        .iter()
        .find(|value| value.name == "curried-with-explicit")
        .unwrap();

    assert_eq!(
        value
            .inputs
            .iter()
            .map(|input| input.name.as_str())
            .collect::<Vec<_>>(),
        ["arg2", "arg1"]
    );
}

#[test]
fn v3_and_v4_normalize_all_shared_type_expression_forms_equivalently() {
    let v3 = normalize(&mothers::classic_customer_library()).unwrap();
    let v4 = normalize(&mothers::v4_customer_library()).unwrap();
    let v3 = v3.modules[0]
        .types
        .iter()
        .find(|declaration| declaration.name() == "complex")
        .unwrap();
    let v4 = v4.modules[0]
        .types
        .iter()
        .find(|declaration| declaration.name() == "complex")
        .unwrap();

    assert_eq!(v3, v4);
    let TypeDeclaration::Alias { value, .. } = v3 else {
        panic!("complex must be an alias")
    };
    assert!(matches!(value, TypeExpr::Tuple(_)));
}

#[test]
fn explicit_value_input_names_are_preserved() {
    let find_customer = normalize(&mothers::v4_customer_library()).unwrap();
    let find_customer = find_customer.modules[0]
        .values
        .iter()
        .find(|value| value.name == "find-customer")
        .unwrap();
    assert_eq!(find_customer.inputs[0].name, "id");
}

#[test]
fn private_modules_declarations_and_values_are_filtered() {
    let package = normalize(&mothers::v4_customer_library()).unwrap();

    assert_eq!(package.modules.len(), 1);
    let module = &package.modules[0];
    assert_eq!(module.path, ["domain"]);
    assert!(module.types.iter().all(|tpe| tpe.name() != "private-type"));
    assert!(module.values.iter().all(|value| value.name != "helper"));
}

#[test]
fn declarations_and_fields_are_sorted_by_canonical_source_name() {
    let package = normalize(&mothers::v4_customer_library()).unwrap();
    let module = &package.modules[0];

    assert_eq!(
        module
            .types
            .iter()
            .map(TypeDeclaration::name)
            .collect::<Vec<_>>(),
        ["complex", "customer", "secret", "status"]
    );
    assert_eq!(
        module
            .values
            .iter()
            .map(|value| value.name.as_str())
            .collect::<Vec<_>>(),
        ["default-customer", "find-customer"]
    );
}

#[test]
fn zero_argument_values_are_constants() {
    let package = normalize(&mothers::v4_customer_library()).unwrap();
    let value = package.modules[0]
        .values
        .iter()
        .find(|value| value.name == "default-customer")
        .unwrap();

    assert!(value.inputs.is_empty());
    assert_eq!(value.value_kind, ValueKind::Constant);
}

#[test]
fn incomplete_type_definitions_remain_explicit() {
    let package = normalize(&mothers::v4_incomplete_library()).unwrap();
    let declaration = &package.modules[0].types[0];

    match declaration {
        TypeDeclaration::Incomplete {
            source_name,
            type_params,
            incompleteness,
            partial_type,
            doc,
            ..
        } => {
            assert_eq!(source_name, "acme/customer:domain#draft-customer");
            assert_eq!(type_params.as_slice(), ["a"]);
            assert!(partial_type.is_some());
            assert_eq!(*incompleteness, IncompletenessKind::Hole);
            assert_eq!(doc.as_deref(), Some("Work in progress."));
        }
        other => panic!("expected incomplete declaration, got {other:?}"),
    }
}

fn find_key<'a>(value: &'a Value, needle: &str) -> Option<&'a Value> {
    match value {
        Value::Object(fields) => fields
            .get(needle)
            .or_else(|| fields.values().find_map(|value| find_key(value, needle))),
        Value::Array(values) => values.iter().find_map(|value| find_key(value, needle)),
        _ => None,
    }
}
