use super::super::*;

pub(in crate::avro::project) fn canonical_type(tpe: &TypeExpr) -> String {
    match tpe {
        TypeExpr::Variable(name) => format!("var({name})"),
        TypeExpr::Reference {
            source_name,
            arguments,
        } => format!(
            "ref({source_name};{})",
            arguments
                .iter()
                .map(canonical_type)
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeExpr::Tuple(elements) => format!(
            "tuple({})",
            elements
                .iter()
                .map(canonical_type)
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeExpr::Record(fields) => format!(
            "record({})",
            fields
                .iter()
                .map(|field| format!("{}:{}", field.name, canonical_type(&field.tpe)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeExpr::ExtensibleRecord { variable, fields } => format!(
            "extensible({variable};{})",
            fields
                .iter()
                .map(|field| format!("{}:{}", field.name, canonical_type(&field.tpe)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        TypeExpr::Function { input, output } => {
            format!(
                "function({};{})",
                canonical_type(input),
                canonical_type(output)
            )
        }
        TypeExpr::Unit => "unit".to_owned(),
    }
}

pub(super) fn type_complexity(tpe: &TypeExpr) -> usize {
    1 + match tpe {
        TypeExpr::Variable(_) | TypeExpr::Unit => 0,
        TypeExpr::Reference { arguments, .. } | TypeExpr::Tuple(arguments) => {
            arguments.iter().map(type_complexity).sum()
        }
        TypeExpr::Record(fields) => fields.iter().map(|field| type_complexity(&field.tpe)).sum(),
        TypeExpr::ExtensibleRecord { fields, .. } => {
            fields.iter().map(|field| type_complexity(&field.tpe)).sum()
        }
        TypeExpr::Function { input, output } => type_complexity(input) + type_complexity(output),
    }
}
