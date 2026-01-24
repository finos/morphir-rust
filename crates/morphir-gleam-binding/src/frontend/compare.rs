//! ModuleIR comparison utilities for roundtrip testing
//!
//! This module provides utilities for comparing two ModuleIR structures
//! for semantic equivalence, allowing for differences that don't affect
//! the meaning of the code.

use super::ast::{
    CaseBranch, Expr, Literal, ModuleIR, Pattern, TypeDef, TypeExpr, ValueDef, Variant,
};

/// Result of comparing two ModuleIRs
#[derive(Debug, Clone)]
pub struct ComparisonResult {
    /// Whether the modules are semantically equivalent
    pub equivalent: bool,
    /// List of differences found
    pub differences: Vec<Difference>,
}

/// A difference between two modules
#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
pub enum Difference {
    /// Module names differ (this is usually allowed)
    ModuleNameDifference {
        original: String,
        regenerated: String,
    },
    /// Different number of type definitions
    TypeCountMismatch { original: usize, regenerated: usize },
    /// Different number of value definitions
    ValueCountMismatch { original: usize, regenerated: usize },
    /// Missing type definition
    MissingType { name: String },
    /// Missing value definition
    MissingValue { name: String },
    /// Type definition differs
    TypeDifference { name: String, detail: String },
    /// Value definition differs
    ValueDifference { name: String, detail: String },
    /// Expression differs
    ExpressionDifference {
        context: String,
        original: String,
        regenerated: String,
    },
    /// Pattern differs
    PatternDifference {
        context: String,
        original: String,
        regenerated: String,
    },
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Difference::ModuleNameDifference {
                original,
                regenerated,
            } => {
                write!(f, "Module name: '{}' vs '{}'", original, regenerated)
            }
            Difference::TypeCountMismatch {
                original,
                regenerated,
            } => {
                write!(f, "Type count: {} vs {}", original, regenerated)
            }
            Difference::ValueCountMismatch {
                original,
                regenerated,
            } => {
                write!(f, "Value count: {} vs {}", original, regenerated)
            }
            Difference::MissingType { name } => {
                write!(f, "Missing type: {}", name)
            }
            Difference::MissingValue { name } => {
                write!(f, "Missing value: {}", name)
            }
            Difference::TypeDifference { name, detail } => {
                write!(f, "Type '{}' differs: {}", name, detail)
            }
            Difference::ValueDifference { name, detail } => {
                write!(f, "Value '{}' differs: {}", name, detail)
            }
            Difference::ExpressionDifference {
                context,
                original,
                regenerated,
            } => {
                write!(
                    f,
                    "Expression in '{}': '{}' vs '{}'",
                    context, original, regenerated
                )
            }
            Difference::PatternDifference {
                context,
                original,
                regenerated,
            } => {
                write!(
                    f,
                    "Pattern in '{}': '{}' vs '{}'",
                    context, original, regenerated
                )
            }
        }
    }
}

/// Compare two ModuleIRs for semantic equivalence
///
/// This function compares the structure and content of two ModuleIRs,
/// returning a detailed comparison result.
///
/// Note: Module names are compared but differences don't affect equivalence
/// since they may change during roundtrip.
pub fn compare_modules(original: &ModuleIR, regenerated: &ModuleIR) -> ComparisonResult {
    let mut differences = Vec::new();

    // Compare module names (informational, doesn't affect equivalence)
    if original.name != regenerated.name {
        differences.push(Difference::ModuleNameDifference {
            original: original.name.clone(),
            regenerated: regenerated.name.clone(),
        });
    }

    // Compare type counts
    if original.types.len() != regenerated.types.len() {
        differences.push(Difference::TypeCountMismatch {
            original: original.types.len(),
            regenerated: regenerated.types.len(),
        });
    }

    // Compare value counts
    if original.values.len() != regenerated.values.len() {
        differences.push(Difference::ValueCountMismatch {
            original: original.values.len(),
            regenerated: regenerated.values.len(),
        });
    }

    // Compare types by name
    for orig_type in &original.types {
        if let Some(regen_type) = regenerated.types.iter().find(|t| t.name == orig_type.name) {
            compare_types(orig_type, regen_type, &mut differences);
        } else {
            differences.push(Difference::MissingType {
                name: orig_type.name.clone(),
            });
        }
    }

    // Compare values by name
    for orig_value in &original.values {
        if let Some(regen_value) = regenerated
            .values
            .iter()
            .find(|v| v.name == orig_value.name)
        {
            compare_values(orig_value, regen_value, &mut differences);
        } else {
            differences.push(Difference::MissingValue {
                name: orig_value.name.clone(),
            });
        }
    }

    // Determine equivalence: module name differences are allowed,
    // but structural differences are not
    let equivalent = differences
        .iter()
        .all(|d| matches!(d, Difference::ModuleNameDifference { .. }));

    ComparisonResult {
        equivalent,
        differences,
    }
}

/// Check if two modules are semantically equivalent
pub fn modules_equivalent(original: &ModuleIR, regenerated: &ModuleIR) -> bool {
    compare_modules(original, regenerated).equivalent
}

/// Compare two type definitions
fn compare_types(original: &TypeDef, regenerated: &TypeDef, differences: &mut Vec<Difference>) {
    // Compare access
    if original.access != regenerated.access {
        differences.push(Difference::TypeDifference {
            name: original.name.clone(),
            detail: format!(
                "Access differs: {:?} vs {:?}",
                original.access, regenerated.access
            ),
        });
    }

    // Compare type parameters
    if original.params != regenerated.params {
        differences.push(Difference::TypeDifference {
            name: original.name.clone(),
            detail: format!(
                "Parameters differ: {:?} vs {:?}",
                original.params, regenerated.params
            ),
        });
    }

    // Compare body
    if !type_expr_equivalent(&original.body, &regenerated.body) {
        differences.push(Difference::TypeDifference {
            name: original.name.clone(),
            detail: format!(
                "Body differs: {:?} vs {:?}",
                original.body, regenerated.body
            ),
        });
    }
}

/// Compare two value definitions
fn compare_values(original: &ValueDef, regenerated: &ValueDef, differences: &mut Vec<Difference>) {
    // Compare access
    if original.access != regenerated.access {
        differences.push(Difference::ValueDifference {
            name: original.name.clone(),
            detail: format!(
                "Access differs: {:?} vs {:?}",
                original.access, regenerated.access
            ),
        });
    }

    // Compare body
    if !expr_equivalent(&original.body, &regenerated.body) {
        differences.push(Difference::ExpressionDifference {
            context: original.name.clone(),
            original: format!("{:?}", original.body),
            regenerated: format!("{:?}", regenerated.body),
        });
    }
}

/// Check if two type expressions are equivalent
fn type_expr_equivalent(a: &TypeExpr, b: &TypeExpr) -> bool {
    match (a, b) {
        (TypeExpr::Variable { name: name_a }, TypeExpr::Variable { name: name_b }) => {
            name_a == name_b
        }
        (TypeExpr::Unit, TypeExpr::Unit) => true,
        (
            TypeExpr::Function {
                from: from_a,
                to: to_a,
            },
            TypeExpr::Function {
                from: from_b,
                to: to_b,
            },
        ) => type_expr_equivalent(from_a, from_b) && type_expr_equivalent(to_a, to_b),
        (TypeExpr::Record { fields: fields_a }, TypeExpr::Record { fields: fields_b }) => {
            fields_a.len() == fields_b.len()
                && fields_a.iter().zip(fields_b.iter()).all(
                    |((name_a, type_a), (name_b, type_b))| {
                        name_a == name_b && type_expr_equivalent(type_a, type_b)
                    },
                )
        }
        (TypeExpr::Tuple { elements: elems_a }, TypeExpr::Tuple { elements: elems_b }) => {
            elems_a.len() == elems_b.len()
                && elems_a
                    .iter()
                    .zip(elems_b.iter())
                    .all(|(a, b)| type_expr_equivalent(a, b))
        }
        (
            TypeExpr::Reference {
                name: name_a,
                args: args_a,
            },
            TypeExpr::Reference {
                name: name_b,
                args: args_b,
            },
        ) => {
            name_a == name_b
                && args_a.len() == args_b.len()
                && args_a
                    .iter()
                    .zip(args_b.iter())
                    .all(|(a, b)| type_expr_equivalent(a, b))
        }
        (
            TypeExpr::CustomType {
                variants: variants_a,
            },
            TypeExpr::CustomType {
                variants: variants_b,
            },
        ) => {
            variants_a.len() == variants_b.len()
                && variants_a
                    .iter()
                    .zip(variants_b.iter())
                    .all(|(a, b)| variant_equivalent(a, b))
        }
        _ => false,
    }
}

/// Check if two variants are equivalent
fn variant_equivalent(a: &Variant, b: &Variant) -> bool {
    a.name == b.name
        && a.fields.len() == b.fields.len()
        && a.fields
            .iter()
            .zip(b.fields.iter())
            .all(|(fa, fb)| type_expr_equivalent(fa, fb))
}

/// Check if two expressions are equivalent
fn expr_equivalent(a: &Expr, b: &Expr) -> bool {
    match (a, b) {
        (Expr::Literal { value: lit_a }, Expr::Literal { value: lit_b }) => {
            literal_equivalent(lit_a, lit_b)
        }
        (Expr::Variable { name: name_a }, Expr::Variable { name: name_b }) => name_a == name_b,
        (
            Expr::Apply {
                function: func_a,
                argument: arg_a,
            },
            Expr::Apply {
                function: func_b,
                argument: arg_b,
            },
        ) => expr_equivalent(func_a, func_b) && expr_equivalent(arg_a, arg_b),
        (
            Expr::Lambda {
                param: param_a,
                body: body_a,
            },
            Expr::Lambda {
                param: param_b,
                body: body_b,
            },
        ) => param_a == param_b && expr_equivalent(body_a, body_b),
        (
            Expr::Let {
                name: name_a,
                value: value_a,
                body: body_a,
            },
            Expr::Let {
                name: name_b,
                value: value_b,
                body: body_b,
            },
        ) => {
            name_a == name_b && expr_equivalent(value_a, value_b) && expr_equivalent(body_a, body_b)
        }
        (
            Expr::If {
                condition: cond_a,
                then_branch: then_a,
                else_branch: else_a,
            },
            Expr::If {
                condition: cond_b,
                then_branch: then_b,
                else_branch: else_b,
            },
        ) => {
            expr_equivalent(cond_a, cond_b)
                && expr_equivalent(then_a, then_b)
                && expr_equivalent(else_a, else_b)
        }
        (Expr::Record { fields: fields_a }, Expr::Record { fields: fields_b }) => {
            fields_a.len() == fields_b.len()
                && fields_a
                    .iter()
                    .zip(fields_b.iter())
                    .all(|((name_a, val_a), (name_b, val_b))| {
                        name_a == name_b && expr_equivalent(val_a, val_b)
                    })
        }
        (
            Expr::Field {
                record: record_a,
                field: field_a,
            },
            Expr::Field {
                record: record_b,
                field: field_b,
            },
        ) => expr_equivalent(record_a, record_b) && field_a == field_b,
        (Expr::Tuple { elements: elems_a }, Expr::Tuple { elements: elems_b }) => {
            elems_a.len() == elems_b.len()
                && elems_a
                    .iter()
                    .zip(elems_b.iter())
                    .all(|(a, b)| expr_equivalent(a, b))
        }
        (
            Expr::Case {
                subject: subject_a,
                branches: branches_a,
            },
            Expr::Case {
                subject: subject_b,
                branches: branches_b,
            },
        ) => {
            expr_equivalent(subject_a, subject_b)
                && branches_a.len() == branches_b.len()
                && branches_a
                    .iter()
                    .zip(branches_b.iter())
                    .all(|(a, b)| case_branch_equivalent(a, b))
        }
        (Expr::Constructor { name: name_a }, Expr::Constructor { name: name_b }) => {
            name_a == name_b
        }
        _ => false,
    }
}

/// Check if two case branches are equivalent
fn case_branch_equivalent(a: &CaseBranch, b: &CaseBranch) -> bool {
    pattern_equivalent(&a.pattern, &b.pattern) && expr_equivalent(&a.body, &b.body)
}

/// Check if two patterns are equivalent
fn pattern_equivalent(a: &Pattern, b: &Pattern) -> bool {
    match (a, b) {
        (Pattern::Wildcard, Pattern::Wildcard) => true,
        (Pattern::Variable { name: name_a }, Pattern::Variable { name: name_b }) => {
            name_a == name_b
        }
        (Pattern::Literal { value: lit_a }, Pattern::Literal { value: lit_b }) => {
            literal_equivalent(lit_a, lit_b)
        }
        (
            Pattern::Constructor {
                name: name_a,
                args: args_a,
            },
            Pattern::Constructor {
                name: name_b,
                args: args_b,
            },
        ) => {
            name_a == name_b
                && args_a.len() == args_b.len()
                && args_a
                    .iter()
                    .zip(args_b.iter())
                    .all(|(a, b)| pattern_equivalent(a, b))
        }
        (Pattern::Tuple { elements: elems_a }, Pattern::Tuple { elements: elems_b }) => {
            elems_a.len() == elems_b.len()
                && elems_a
                    .iter()
                    .zip(elems_b.iter())
                    .all(|(a, b)| pattern_equivalent(a, b))
        }
        _ => false,
    }
}

/// Check if two literals are equivalent
fn literal_equivalent(a: &Literal, b: &Literal) -> bool {
    match (a, b) {
        (Literal::Int { value: val_a }, Literal::Int { value: val_b }) => val_a == val_b,
        (Literal::Float { value: val_a }, Literal::Float { value: val_b }) => val_a == val_b,
        (Literal::String { value: val_a }, Literal::String { value: val_b }) => val_a == val_b,
        (Literal::Bool { value: val_a }, Literal::Bool { value: val_b }) => val_a == val_b,
        (Literal::Char { value: val_a }, Literal::Char { value: val_b }) => val_a == val_b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::ast::Access;
    use super::*;

    fn make_simple_module(name: &str, values: Vec<ValueDef>) -> ModuleIR {
        ModuleIR {
            name: name.to_string(),
            doc: None,
            types: vec![],
            values,
        }
    }

    fn make_value(name: &str, body: Expr) -> ValueDef {
        ValueDef {
            access: Access::Public,
            name: name.to_string(),
            type_annotation: None,
            body,
        }
    }

    #[test]
    fn test_identical_modules_are_equivalent() {
        let value = make_value(
            "hello",
            Expr::Literal {
                value: Literal::String {
                    value: "world".to_string(),
                },
            },
        );
        let module = make_simple_module("test", vec![value.clone()]);

        let result = compare_modules(&module, &module);
        assert!(result.equivalent);
        assert!(result.differences.is_empty());
    }

    #[test]
    fn test_different_module_names_are_allowed() {
        let value = make_value(
            "hello",
            Expr::Literal {
                value: Literal::Int { value: 42 },
            },
        );
        let module1 = make_simple_module("original", vec![value.clone()]);
        let module2 = make_simple_module("generated", vec![value]);

        let result = compare_modules(&module1, &module2);
        assert!(result.equivalent);
        assert_eq!(result.differences.len(), 1);
        assert!(matches!(
            &result.differences[0],
            Difference::ModuleNameDifference { .. }
        ));
    }

    #[test]
    fn test_different_value_count_not_equivalent() {
        let value = make_value(
            "hello",
            Expr::Literal {
                value: Literal::Int { value: 42 },
            },
        );
        let module1 = make_simple_module("test", vec![value.clone()]);
        let module2 = make_simple_module("test", vec![value.clone(), value]);

        let result = compare_modules(&module1, &module2);
        assert!(!result.equivalent);
    }

    #[test]
    fn test_different_expression_not_equivalent() {
        let value1 = make_value(
            "hello",
            Expr::Literal {
                value: Literal::Int { value: 42 },
            },
        );
        let value2 = make_value(
            "hello",
            Expr::Literal {
                value: Literal::Int { value: 43 },
            },
        );
        let module1 = make_simple_module("test", vec![value1]);
        let module2 = make_simple_module("test", vec![value2]);

        let result = compare_modules(&module1, &module2);
        assert!(!result.equivalent);
    }
}
