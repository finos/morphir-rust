use super::super::*;
use pretty_assertions::assert_eq;

#[test]
fn linked_projection_reports_a_missing_declaration_as_avro006() {
    let missing = alias(
        "acme/customer:customer#customer",
        "customer",
        TypeExpr::Record(vec![field(
            "missing",
            reference("acme/missing:types#identifier", vec![]),
        )]),
    );
    let error = project(
        &package(vec![missing]),
        &AvroOptions {
            dependencies: Dependencies::Linked,
            ..AvroOptions::default()
        },
    )
    .unwrap_err()
    .into_diagnostics()
    .unwrap();
    assert_eq!(error[0].code(), "AVRO006");
    assert_eq!(error[0].source(), Some("acme/customer:customer#customer"));
}
