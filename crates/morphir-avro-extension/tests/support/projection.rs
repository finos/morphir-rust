use morphir_avro_extension::{
    DistributionKind, EntryPointMetadata, NamedType, ProjectionModule, ProjectionPackage,
    TypeDeclaration, TypeExpr, ValueKind, ValueSpecification,
};

pub fn reference(source_name: &str, arguments: Vec<TypeExpr>) -> TypeExpr {
    TypeExpr::Reference {
        source_name: source_name.to_owned(),
        arguments,
    }
}

pub fn field(name: &str, tpe: TypeExpr) -> NamedType {
    NamedType {
        name: name.to_owned(),
        tpe,
    }
}

pub fn value_specification(
    source_name: &str,
    name: &str,
    inputs: Vec<NamedType>,
    output: Option<TypeExpr>,
    value_kind: ValueKind,
    entry_point: Option<EntryPointMetadata>,
) -> ValueSpecification {
    ValueSpecification {
        source_name: source_name.to_owned(),
        name: name.to_owned(),
        inputs,
        output,
        value_kind,
        entry_point,
        doc: Some(format!("Documentation for {name}.")),
    }
}

pub fn package(types: Vec<TypeDeclaration>) -> ProjectionPackage {
    ProjectionPackage {
        kind: DistributionKind::Library,
        package_name: "acme/customer".to_owned(),
        dependencies: Vec::new(),
        modules: vec![ProjectionModule {
            path: vec!["customer".to_owned()],
            types,
            values: Vec::new(),
            doc: None,
        }],
    }
}

pub fn alias(source_name: &str, name: &str, value: TypeExpr) -> TypeDeclaration {
    TypeDeclaration::Alias {
        source_name: source_name.to_owned(),
        name: name.to_owned(),
        type_params: Vec::new(),
        value,
        doc: None,
    }
}

pub fn customer_record() -> TypeDeclaration {
    alias(
        "Acme:Customer:Customer",
        "customer",
        TypeExpr::Record(vec![
            field("active", reference("morphir/SDK:basics#bool", Vec::new())),
            field("age", reference("morphir/SDK:basics#int", Vec::new())),
            field("name", reference("morphir/SDK:string#string", Vec::new())),
        ]),
    )
}
