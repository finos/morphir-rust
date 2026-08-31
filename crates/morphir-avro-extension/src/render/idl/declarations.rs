use super::*;

impl<'package> IdlRenderer<'package> {
    pub(super) fn render_synthetic_root(
        &self,
        output: &mut String,
        root: &AvroRoot,
    ) -> Result<(), AvroDiagnostic> {
        render_doc(output, "  ", root.doc());
        annotation(
            output,
            "  ",
            "namespace",
            &Value::String(root.full_name().namespace().to_owned()),
        )?;
        render_annotations(output, "  ", root.properties())?;
        output.push_str("  record ");
        output.push_str(&escape_idl_identifier(root.full_name().name()));
        output.push_str(" {\n    ");
        output.push_str(&render_type(root.tpe())?);
        output.push_str(" value;\n  }\n");
        Ok(())
    }

    pub(super) fn declarations_in_dependency_order(
        &self,
        selected: &BTreeSet<String>,
    ) -> Result<Vec<&'package NamedSchema>, AvroDiagnostic> {
        detect_cycle(&self.graph, selected)?;
        let mut visited = BTreeSet::new();
        let mut ordered = Vec::new();
        for name in selected {
            self.visit_declaration(name, selected, &mut visited, &mut ordered);
        }
        ordered
            .into_iter()
            .map(|name| {
                self.schemas
                    .get(&name)
                    .copied()
                    .ok_or_else(|| AvroDiagnostic::missing_linked_dependency(name))
            })
            .collect()
    }

    fn visit_declaration(
        &self,
        name: &str,
        selected: &BTreeSet<String>,
        visited: &mut BTreeSet<String>,
        ordered: &mut Vec<String>,
    ) {
        if !visited.insert(name.to_owned()) {
            return;
        }
        if let Some(dependencies) = self.graph.get(name) {
            for dependency in dependencies {
                if dependency != name && selected.contains(dependency) {
                    self.visit_declaration(dependency, selected, visited, ordered);
                }
            }
        }
        ordered.push(name.to_owned());
    }

    pub(super) fn render_named(
        &self,
        output: &mut String,
        schema: &NamedSchema,
    ) -> Result<(), AvroDiagnostic> {
        render_doc(output, "  ", schema.doc());
        annotation(
            output,
            "  ",
            "namespace",
            &Value::String(schema.full_name().namespace().to_owned()),
        )?;
        match schema {
            NamedSchema::Record(record) => {
                render_annotations(output, "  ", record.properties())?;
                output.push_str("  record ");
                output.push_str(&escape_idl_identifier(record.full_name().name()));
                output.push_str(" {\n");
                for field in record.fields() {
                    self.render_field(output, field, "    ")?;
                }
                output.push_str("  }\n");
            }
            NamedSchema::Enum(schema) => {
                render_annotations(output, "  ", schema.properties())?;
                output.push_str("  enum ");
                output.push_str(&escape_idl_identifier(schema.full_name().name()));
                output.push_str(" { ");
                output.push_str(
                    &schema
                        .symbols()
                        .iter()
                        .map(|symbol| escape_idl_identifier(symbol))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                output.push_str(" }\n");
            }
            NamedSchema::Fixed(schema) => {
                render_annotations(output, "  ", schema.properties())?;
                output.push_str("  fixed ");
                output.push_str(&escape_idl_identifier(schema.full_name().name()));
                output.push('(');
                output.push_str(&schema.size().to_string());
                output.push_str(");\n");
            }
        }
        Ok(())
    }

    fn render_field(
        &self,
        output: &mut String,
        field: &AvroField,
        indent: &str,
    ) -> Result<(), AvroDiagnostic> {
        output.push_str(indent);
        output.push_str(&render_type(field.tpe())?);
        output.push(' ');
        render_inline_annotations(output, field.properties())?;
        output.push_str(&escape_idl_identifier(field.name()));
        output.push_str(";\n");
        Ok(())
    }

    pub(super) fn render_message(
        &self,
        output: &mut String,
        message: &AvroMessage,
    ) -> Result<(), AvroDiagnostic> {
        render_doc(
            output,
            "  ",
            property_string(message.properties(), "morphir.doc"),
        );
        render_annotations(output, "  ", message.properties())?;
        output.push_str("  ");
        output.push_str(&render_type(message.response())?);
        output.push(' ');
        output.push_str(&escape_idl_identifier(message.name()));
        output.push('(');
        for (index, field) in message.request().fields().iter().enumerate() {
            if index > 0 {
                output.push_str(", ");
            }
            output.push_str(&render_type(field.tpe())?);
            output.push(' ');
            render_inline_annotations(output, field.properties())?;
            output.push_str(&escape_idl_identifier(field.name()));
        }
        output.push(')');
        if !message.errors().is_empty() {
            output.push_str(" throws ");
            let errors = message
                .errors()
                .iter()
                .map(render_type)
                .collect::<Result<Vec<_>, _>>()?;
            output.push_str(&errors.join(", "));
        }
        output.push_str(";\n");
        Ok(())
    }
}
