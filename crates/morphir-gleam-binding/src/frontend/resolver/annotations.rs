//! Traverse expression bodies to validate every nested type annotation.
use super::*;

pub(super) fn resolve_body_annotations(
    body: &mut crate::frontend::ast::Expr,
    scope: &mut Scope<'_>,
) {
    use crate::frontend::ast::{Expr, Statement};
    match body {
        Expr::Block { statements } => {
            for statement in statements {
                match statement {
                    Statement::Assignment {
                        annotation, value, ..
                    } => {
                        if let Some(ty) = annotation {
                            *ty = scope.ty(ty);
                        }
                        resolve_body_annotations(value, scope);
                    }
                    Statement::Expression(expr) => resolve_body_annotations(expr, scope),
                    Statement::Use { function, .. } => resolve_body_annotations(function, scope),
                }
            }
        }
        Expr::Lambda { body, .. } => resolve_body_annotations(body, scope),
        Expr::Let { value, body, .. } => {
            resolve_body_annotations(value, scope);
            resolve_body_annotations(body, scope);
        }
        Expr::Case { subjects, clauses } => {
            for expr in subjects {
                resolve_body_annotations(expr, scope);
            }
            for clause in clauses {
                resolve_body_annotations(&mut clause.body, scope);
            }
        }
        Expr::If {
            condition,
            then_branch,
            else_branch,
        } => {
            resolve_body_annotations(condition, scope);
            resolve_body_annotations(then_branch, scope);
            resolve_body_annotations(else_branch, scope);
        }
        Expr::Apply {
            function,
            arguments,
        } => {
            resolve_body_annotations(function, scope);
            for argument in arguments {
                resolve_field(argument, scope);
            }
        }
        Expr::FnCapture {
            function,
            arguments_before,
            arguments_after,
        } => {
            resolve_body_annotations(function, scope);
            for argument in arguments_before.iter_mut().chain(arguments_after) {
                resolve_field(argument, scope);
            }
        }
        Expr::BinaryOp { left, right, .. } => {
            resolve_body_annotations(left, scope);
            resolve_body_annotations(right, scope);
        }
        Expr::NegateInt { value } | Expr::NegateBool { value } => {
            resolve_body_annotations(value, scope)
        }
        Expr::FieldAccess { container, .. } => resolve_body_annotations(container, scope),
        Expr::TupleIndex { tuple, .. } => resolve_body_annotations(tuple, scope),
        Expr::Tuple { elements } => {
            for element in elements {
                resolve_body_annotations(element, scope);
            }
        }
        Expr::List { elements, tail } => {
            for element in elements {
                resolve_body_annotations(element, scope);
            }
            if let Some(tail) = tail {
                resolve_body_annotations(tail, scope);
            }
        }
        Expr::Record { fields } => {
            for (_, field) in fields {
                resolve_body_annotations(field, scope);
            }
        }
        Expr::RecordUpdate { record, fields, .. } => {
            resolve_body_annotations(record, scope);
            for (_, field) in fields {
                resolve_body_annotations(field, scope);
            }
        }
        Expr::Panic { message } | Expr::Todo { message } => {
            if let Some(message) = message {
                resolve_body_annotations(message, scope);
            }
        }
        Expr::Echo { expression, body } => {
            resolve_body_annotations(expression, scope);
            if let Some(body) = body {
                resolve_body_annotations(body, scope);
            }
        }
        Expr::BitString { segments } => {
            for segment in segments {
                resolve_body_annotations(&mut segment.value, scope);
                for option in &mut segment.options {
                    if let crate::frontend::ast::BitStringOption::Size(expr) = option {
                        resolve_body_annotations(expr, scope);
                    }
                }
            }
        }
        Expr::Literal { .. } | Expr::Variable { .. } | Expr::Constructor { .. } => {}
    }
}

fn resolve_field(
    field: &mut crate::frontend::ast::Field<crate::frontend::ast::Expr>,
    scope: &mut Scope<'_>,
) {
    use crate::frontend::ast::Field;
    match field {
        Field::Labelled { item, .. } | Field::Unlabelled { item } => {
            resolve_body_annotations(item, scope)
        }
        Field::Shorthand { .. } => {}
    }
}
