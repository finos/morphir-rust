use super::*;

#[test]
fn duplicate_named_full_names_are_rejected_by_unions() {
    let name = AvroFullName::new("acme".to_owned(), "Customer".to_owned()).unwrap();
    assert_eq!(
        AvroUnion::new(vec![AvroType::Named(name.clone()), AvroType::Named(name)]),
        Err(UnionError::DuplicateBranch(
            "named:acme.Customer".to_owned()
        ))
    );
}

#[test]
fn packages_reject_unresolved_root_and_schema_references() {
    let missing = AvroFullName::new("acme.missing".to_owned(), "Type".to_owned()).unwrap();
    let root = AvroRoot::new(
        "acme/customer:domain#root".to_owned(),
        AvroFullName::new("acme.customer.domain".to_owned(), "Root".to_owned()).unwrap(),
        AvroType::Named(missing.clone()),
        None,
    )
    .unwrap();
    let error =
        AvroPackage::new(vec![root], Vec::new(), Vec::new(), Vec::new(), Vec::new()).unwrap_err();
    assert_eq!(error.code(), "AVRO006");
    assert_eq!(
        error.message(),
        "missing linked dependency: acme.missing.Type"
    );

    let schema = NamedSchema::Record(
        RecordSchema::new(
            AvroFullName::new("acme.customer".to_owned(), "Wrapper".to_owned()).unwrap(),
            vec![
                AvroField::new(
                    "value".to_owned(),
                    AvroType::Named(missing),
                    Properties::new(),
                )
                .unwrap(),
            ],
            None,
            Properties::new(),
        )
        .unwrap(),
    );
    let error =
        AvroPackage::new(Vec::new(), vec![schema], Vec::new(), Vec::new(), Vec::new()).unwrap_err();
    assert_eq!(error.code(), "AVRO006");
    assert!(error.message().contains("acme.missing.Type"));
}
