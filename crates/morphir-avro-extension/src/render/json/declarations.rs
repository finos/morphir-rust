use super::*;

impl JsonRenderer<'_> {
    pub(super) fn render_named_inline(
        &self,
        name: &AvroFullName,
        state: &mut InlineState,
        scope: DefinitionScope<'_>,
    ) -> Value {
        let key = name.to_string();
        if state.active.contains(&key)
            || state.defined.contains(&key)
            || !scope.allows(&key, &self.linked_names)
        {
            return Value::String(key);
        }
        let Some(schema) = self.schemas.get(&key) else {
            return Value::String(key);
        };
        state.active.insert(key.clone());
        state.defined.insert(key.clone());
        let value = self.render_named(schema, |tpe| self.render_type_inline(tpe, state, scope));
        state.active.remove(&key);
        value
    }

    pub(super) fn render_named_reference_only(&self, schema: &NamedSchema) -> Value {
        self.render_named(schema, |tpe| self.render_type_reference_only(tpe))
    }

    fn render_named(
        &self,
        schema: &NamedSchema,
        mut render_type: impl FnMut(&AvroType) -> Value,
    ) -> Value {
        match schema {
            NamedSchema::Record(record) => {
                let mut object = properties_object(record.properties());
                insert_doc(&mut object, record.doc());
                object.insert(
                    "fields".to_owned(),
                    Value::Array(
                        record
                            .fields()
                            .iter()
                            .map(|field| render_field(field, &mut render_type))
                            .collect(),
                    ),
                );
                insert_name(&mut object, record.full_name());
                object.insert("type".to_owned(), Value::String("record".to_owned()));
                Value::Object(object)
            }
            NamedSchema::Enum(schema) => {
                let mut object = properties_object(schema.properties());
                insert_doc(&mut object, schema.doc());
                insert_name(&mut object, schema.full_name());
                object.insert(
                    "symbols".to_owned(),
                    Value::Array(
                        schema
                            .symbols()
                            .iter()
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                );
                object.insert("type".to_owned(), Value::String("enum".to_owned()));
                Value::Object(object)
            }
            NamedSchema::Fixed(schema) => {
                let mut object = properties_object(schema.properties());
                insert_doc(&mut object, schema.doc());
                insert_name(&mut object, schema.full_name());
                object.insert(
                    "size".to_owned(),
                    Value::Number(Number::from(schema.size())),
                );
                object.insert("type".to_owned(), Value::String("fixed".to_owned()));
                Value::Object(object)
            }
        }
    }

    pub(super) fn render_field_reference_only(&self, field: &AvroField) -> Value {
        render_field(field, &mut |tpe| self.render_type_reference_only(tpe))
    }

    pub(super) fn render_type_inline(
        &self,
        tpe: &AvroType,
        state: &mut InlineState,
        scope: DefinitionScope<'_>,
    ) -> Value {
        match tpe {
            AvroType::Named(name) => self.render_named_inline(name, state, scope),
            AvroType::Array(items, properties) => {
                let mut object = properties_object(properties);
                object.insert(
                    "items".to_owned(),
                    self.render_type_inline(items, state, scope),
                );
                object.insert("type".to_owned(), Value::String("array".to_owned()));
                Value::Object(object)
            }
            AvroType::Map(values, properties) => {
                let mut object = properties_object(properties);
                object.insert("type".to_owned(), Value::String("map".to_owned()));
                object.insert(
                    "values".to_owned(),
                    self.render_type_inline(values, state, scope),
                );
                Value::Object(object)
            }
            AvroType::Union(union) => Value::Array(
                union
                    .branches()
                    .iter()
                    .map(|branch| self.render_type_inline(branch, state, scope))
                    .collect(),
            ),
            AvroType::Logical {
                physical,
                name,
                properties,
            } => decorate_type(
                self.render_type_inline(physical, state, scope),
                properties,
                Some(name),
            ),
            AvroType::Annotated {
                physical,
                properties,
            } => decorate_type(
                self.render_type_inline(physical, state, scope),
                properties,
                None,
            ),
            AvroType::Null => Value::String("null".to_owned()),
            AvroType::Boolean => Value::String("boolean".to_owned()),
            AvroType::Int => Value::String("int".to_owned()),
            AvroType::Long => Value::String("long".to_owned()),
            AvroType::Float => Value::String("float".to_owned()),
            AvroType::Double => Value::String("double".to_owned()),
            AvroType::Bytes => Value::String("bytes".to_owned()),
            AvroType::String => Value::String("string".to_owned()),
        }
    }

    pub(super) fn render_type_reference_only(&self, tpe: &AvroType) -> Value {
        match tpe {
            AvroType::Named(name) => Value::String(name.to_string()),
            AvroType::Array(items, properties) => {
                let mut object = properties_object(properties);
                object.insert("items".to_owned(), self.render_type_reference_only(items));
                object.insert("type".to_owned(), Value::String("array".to_owned()));
                Value::Object(object)
            }
            AvroType::Map(values, properties) => {
                let mut object = properties_object(properties);
                object.insert("type".to_owned(), Value::String("map".to_owned()));
                object.insert("values".to_owned(), self.render_type_reference_only(values));
                Value::Object(object)
            }
            AvroType::Union(union) => Value::Array(
                union
                    .branches()
                    .iter()
                    .map(|branch| self.render_type_reference_only(branch))
                    .collect(),
            ),
            AvroType::Logical {
                physical,
                name,
                properties,
            } => decorate_type(
                self.render_type_reference_only(physical),
                properties,
                Some(name),
            ),
            AvroType::Annotated {
                physical,
                properties,
            } => decorate_type(self.render_type_reference_only(physical), properties, None),
            AvroType::Null => Value::String("null".to_owned()),
            AvroType::Boolean => Value::String("boolean".to_owned()),
            AvroType::Int => Value::String("int".to_owned()),
            AvroType::Long => Value::String("long".to_owned()),
            AvroType::Float => Value::String("float".to_owned()),
            AvroType::Double => Value::String("double".to_owned()),
            AvroType::Bytes => Value::String("bytes".to_owned()),
            AvroType::String => Value::String("string".to_owned()),
        }
    }
}
