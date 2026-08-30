use super::*;

pub(super) fn type_application_digest(source_name: &str, arguments: &[TypeExpr]) -> String {
    let mut identity = vec![b'G'];
    encode_string(&mut identity, source_name);
    encode_len(&mut identity, arguments.len());
    for argument in arguments {
        encode_type_expr(&mut identity, argument);
    }
    Sha256::digest(identity)[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn encode_type_expr(output: &mut Vec<u8>, tpe: &TypeExpr) {
    match tpe {
        TypeExpr::Variable(name) => {
            output.push(b'v');
            encode_string(output, name);
        }
        TypeExpr::Reference {
            source_name,
            arguments,
        } => {
            output.push(b'r');
            encode_string(output, source_name);
            encode_len(output, arguments.len());
            for argument in arguments {
                encode_type_expr(output, argument);
            }
        }
        TypeExpr::Tuple(elements) => {
            output.push(b't');
            encode_len(output, elements.len());
            for element in elements {
                encode_type_expr(output, element);
            }
        }
        TypeExpr::Record(fields) => {
            output.push(b'c');
            encode_type_fields(output, fields);
        }
        TypeExpr::ExtensibleRecord { variable, fields } => {
            output.push(b'e');
            encode_string(output, variable);
            encode_type_fields(output, fields);
        }
        TypeExpr::Function {
            input,
            output: result,
        } => {
            output.push(b'f');
            encode_type_expr(output, input);
            encode_type_expr(output, result);
        }
        TypeExpr::Unit => output.push(b'u'),
    }
}

pub(super) fn encode_type_fields(output: &mut Vec<u8>, fields: &[NamedType]) {
    encode_len(output, fields.len());
    for field in fields {
        encode_string(output, &field.name);
        encode_type_expr(output, &field.tpe);
    }
}

/// Encode projected Avro types for stable synthetic-name hashing.
///
/// Every node starts with a one-byte tag. Variable-length values use an
/// unsigned 64-bit big-endian byte length followed by raw UTF-8 bytes. Maps
/// are key-sorted, and JSON values recursively use explicit scalar, array, and
/// sorted-object tags. This encoding is independent of Rust formatting traits.
pub(super) fn canonical_tuple_identity(elements: &[AvroType]) -> Vec<u8> {
    let mut output = vec![b'T'];
    encode_len(&mut output, elements.len());
    for element in elements {
        encode_avro_type(&mut output, element);
    }
    output
}

pub(super) fn encode_avro_type(output: &mut Vec<u8>, tpe: &AvroType) {
    match tpe {
        AvroType::Null => output.push(b'n'),
        AvroType::Boolean => output.push(b'b'),
        AvroType::Int => output.push(b'i'),
        AvroType::Long => output.push(b'l'),
        AvroType::Float => output.push(b'f'),
        AvroType::Double => output.push(b'd'),
        AvroType::Bytes => output.push(b'y'),
        AvroType::String => output.push(b's'),
        AvroType::Array(element, properties) => {
            output.push(b'a');
            encode_avro_type(output, element);
            encode_properties(output, properties);
        }
        AvroType::Map(value, properties) => {
            output.push(b'm');
            encode_avro_type(output, value);
            encode_properties(output, properties);
        }
        AvroType::Union(union) => {
            output.push(b'u');
            encode_len(output, union.branches().len());
            for branch in union.branches() {
                encode_avro_type(output, branch);
            }
        }
        AvroType::Named(name) => {
            output.push(b'r');
            encode_string(output, &name.to_string());
        }
        AvroType::Logical {
            physical,
            name,
            properties,
        } => {
            output.push(b'g');
            encode_avro_type(output, physical);
            encode_string(output, name);
            encode_properties(output, properties);
        }
        AvroType::Annotated {
            physical,
            properties,
        } => {
            output.push(b't');
            encode_avro_type(output, physical);
            encode_properties(output, properties);
        }
    }
}

pub(super) fn encode_properties(output: &mut Vec<u8>, properties: &Properties) {
    encode_len(output, properties.len());
    for (key, value) in properties {
        encode_string(output, key);
        encode_json(output, value);
    }
}

pub(super) fn encode_json(output: &mut Vec<u8>, value: &Value) {
    match value {
        Value::Null => output.push(b'0'),
        Value::Bool(value) => output.extend_from_slice(if *value { b"b1" } else { b"b0" }),
        Value::Number(value) => {
            output.push(b'd');
            encode_string(output, &value.to_string());
        }
        Value::String(value) => {
            output.push(b's');
            encode_string(output, value);
        }
        Value::Array(values) => {
            output.push(b'a');
            encode_len(output, values.len());
            for value in values {
                encode_json(output, value);
            }
        }
        Value::Object(values) => {
            output.push(b'o');
            encode_len(output, values.len());
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (key, value) in entries {
                encode_string(output, key);
                encode_json(output, value);
            }
        }
    }
}

pub(super) fn encode_string(output: &mut Vec<u8>, value: &str) {
    encode_len(output, value.len());
    output.extend_from_slice(value.as_bytes());
}

pub(super) fn encode_len(output: &mut Vec<u8>, value: usize) {
    output.extend_from_slice(&(value as u64).to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_type_bytes_sort_nested_json_objects_and_encode_arrays_and_scalars() {
        let first = AvroType::Annotated {
            physical: Box::new(AvroType::String),
            properties: BTreeMap::from([(
                "metadata".to_owned(),
                json!({"z": [true, null, 4], "a": {"y": "two", "x": "one"}}),
            )]),
        };
        let second = AvroType::Annotated {
            physical: Box::new(AvroType::String),
            properties: BTreeMap::from([(
                "metadata".to_owned(),
                json!({"a": {"x": "one", "y": "two"}, "z": [true, null, 4]}),
            )]),
        };

        assert_eq!(
            canonical_tuple_identity(&[first]),
            canonical_tuple_identity(&[second])
        );
    }

    #[test]
    fn type_application_digest_has_a_stable_exact_encoding() {
        assert_eq!(
            type_application_digest(
                SDK_RESULT,
                &[
                    TypeExpr::Reference {
                        source_name: SDK_STRING.to_owned(),
                        arguments: Vec::new(),
                    },
                    TypeExpr::Reference {
                        source_name: "acme/one:domain#customer".to_owned(),
                        arguments: Vec::new(),
                    },
                ],
            ),
            "d920df848bb1"
        );
    }

    #[test]
    fn failed_declaration_projection_rolls_back_all_scratch_state() {
        let bad = TypeDeclaration::Alias {
            source_name: "acme/customer:domain#bad".to_owned(),
            name: "bad".to_owned(),
            type_params: Vec::new(),
            value: TypeExpr::Record(vec![
                NamedType {
                    name: "tuple".to_owned(),
                    tpe: TypeExpr::Tuple(vec![TypeExpr::Unit, TypeExpr::Unit]),
                },
                NamedType {
                    name: "unsupported".to_owned(),
                    tpe: TypeExpr::Function {
                        input: Box::new(TypeExpr::Unit),
                        output: Box::new(TypeExpr::Unit),
                    },
                },
            ]),
            doc: None,
        };
        let good = TypeDeclaration::Alias {
            source_name: "acme/customer:domain#good".to_owned(),
            name: "good".to_owned(),
            type_params: Vec::new(),
            value: TypeExpr::Record(vec![NamedType {
                name: "value".to_owned(),
                tpe: TypeExpr::Unit,
            }]),
            doc: None,
        };
        let module = ProjectionModule {
            path: vec!["domain".to_owned()],
            types: vec![bad.clone(), good.clone()],
            values: Vec::new(),
            doc: None,
        };
        let options = AvroOptions::default();
        let mut projector = Projector::new(&options);
        assert!(
            projector
                .register_package(&ProjectionPackage {
                    kind: crate::DistributionKind::Library,
                    package_name: "acme/customer".to_owned(),
                    dependencies: Vec::new(),
                    modules: vec![module.clone()],
                })
                .is_empty()
        );
        let indexed_registry = projector.registry.clone();

        assert!(
            projector
                .project_declaration("acme/customer", &module, &bad)
                .is_err()
        );
        assert!(projector.schemas.is_empty());
        assert!(projector.roots.is_empty());
        assert!(projector.building_schemas.is_empty());
        assert!(projector.active_declarations.is_empty());
        assert_eq!(projector.registry, indexed_registry);

        projector
            .project_declaration("acme/customer", &module, &good)
            .unwrap();
        assert!(projector.schemas.contains_key("acme.customer.domain.Good"));
        assert!(!projector.schemas.keys().any(|name| name.contains("Tuple_")));
    }
}
