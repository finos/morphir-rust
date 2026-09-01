use morphir_openapi_extension::{Schema, SchemaOptions, SchemaProjection, Unsupported, project};
use morphir_projection::{normalize, testing::classic};

fn projection(ir: serde_json::Value) -> SchemaProjection {
    let package = normalize(&ir).expect("the fixture normalizes");
    project(&package, &SchemaOptions::default()).expect("the fixture projects")
}

fn root<'a>(projection: &'a SchemaProjection, source_name: &str) -> &'a Schema {
    &projection
        .roots
        .iter()
        .find(|root| root.source_name == source_name)
        .unwrap_or_else(|| panic!("no root for {source_name}"))
        .schema
}

#[test]
fn projects_a_record_alias_as_an_object_with_required_fields() {
    let projection = projection(classic::classic_schema_library());

    let Schema::Object { fields, required } = root(&projection, "acme/customer:customer#customer")
    else {
        panic!("a record alias projects as an object");
    };
    assert!(fields.iter().any(|field| field.name == "customerId"));
    assert!(required.contains(&"customerId".to_owned()));
}

#[test]
fn projects_maybe_as_a_union_with_null() {
    let projection = projection(classic::classic_schema_library());

    let optional = projection
        .definitions
        .values()
        .flat_map(|named| match &named.schema {
            Schema::Object { fields, .. } => fields.clone(),
            _ => Vec::new(),
        })
        .find(|field| matches!(field.schema, Schema::Union(_)))
        .expect("the fixture has an optional field");

    let Schema::Union(members) = optional.schema else {
        unreachable!("filtered above");
    };
    assert!(members.iter().any(|member| matches!(member, Schema::Null)));
}

#[test]
fn projects_a_nullary_custom_type_as_an_enumeration() {
    let projection = projection(classic::classic_schema_library());

    let enumeration = projection
        .definitions
        .values()
        .find(|named| matches!(named.schema, Schema::Enumeration(_)))
        .expect("the fixture has a nullary custom type");

    let Schema::Enumeration(values) = &enumeration.schema else {
        unreachable!("filtered above");
    };
    assert!(!values.is_empty());
    assert_eq!(values.clone(), {
        let mut sorted = values.clone();
        sorted.sort();
        sorted
    });
}

#[test]
fn a_name_collision_is_an_error_rather_than_a_rename() {
    let package =
        normalize(&classic::classic_colliding_names_library()).expect("the fixture normalizes");

    let error =
        project(&package, &SchemaOptions::default()).expect_err("a collision fails projection");

    assert_eq!(error.code(), "JSC004");
}

#[test]
fn strict_mode_fails_on_a_function_used_as_data() {
    let package =
        normalize(&classic::classic_function_field_library()).expect("the fixture normalizes");

    let error =
        project(&package, &SchemaOptions::default()).expect_err("a function field has no schema");

    assert_eq!(error.code(), "JSC003");
}

#[test]
fn warn_and_skip_omits_the_form_and_keeps_the_rest() {
    let package =
        normalize(&classic::classic_function_field_library()).expect("the fixture normalizes");
    let options = SchemaOptions {
        unsupported: Unsupported::WarnAndSkip,
    };

    let projection = project(&package, &options).expect("skipping keeps projection successful");

    assert!(
        projection
            .diagnostics
            .iter()
            .any(|(diagnostic, warning)| { *warning && diagnostic.code() == "JSC003" })
    );
    assert!(!projection.roots.is_empty());
    assert!(
        !projection
            .roots
            .iter()
            .any(|root| root.source_name == "acme/customer:customer#handler")
    );
}
