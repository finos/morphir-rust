mod advanced_types {
    use super::*;
    use pretty_assertions::assert_eq;

    const STRING: &str = "morphir/SDK:string#string";
    const INT: &str = "morphir/SDK:basics#int";
    const RESULT: &str = "morphir/SDK:result#result";
    const LOCAL_DATE: &str = "morphir/SDK:local-date#local-date";
    const LOCAL_TIME: &str = "morphir/SDK:local-time#local-time";
    const INSTANT: &str = "morphir/SDK:instant#instant";
    const DATE_TIME: &str = "morphir/SDK:date-time#date-time";
    const UUID: &str = "morphir/SDK:uuid#uuid";
    const DECIMAL: &str = "morphir/SDK:decimal#decimal";

    fn custom(
        source_name: &str,
        name: &str,
        type_params: Vec<&str>,
        constructors: Vec<Constructor>,
    ) -> TypeDeclaration {
        TypeDeclaration::Custom {
            source_name: source_name.to_owned(),
            name: name.to_owned(),
            type_params: type_params.into_iter().map(str::to_owned).collect(),
            constructors,
            doc: None,
        }
    }

    fn constructor(name: &str, arguments: Vec<morphir_avro_extension::NamedType>) -> Constructor {
        Constructor {
            source_name: format!("acme/customer:customer#{name}"),
            name: name.to_owned(),
            arguments,
        }
    }

    fn alias_with_params(
        source_name: &str,
        name: &str,
        type_params: Vec<&str>,
        value: TypeExpr,
    ) -> TypeDeclaration {
        TypeDeclaration::Alias {
            source_name: source_name.to_owned(),
            name: name.to_owned(),
            type_params: type_params.into_iter().map(str::to_owned).collect(),
            value,
            doc: None,
        }
    }

    fn logical(
        physical: AvroType,
        name: &str,
        properties: &[(&str, serde_json::Value)],
    ) -> AvroType {
        AvroType::Logical {
            physical: Box::new(physical),
            name: name.to_owned(),
            properties: properties
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect(),
        }
    }

    include!("advanced_types/custom_types.rs");
    include!("advanced_types/logical_mappings.rs");
    include!("advanced_types/recursion.rs");
    include!("advanced_types/generic_naming.rs");
}
