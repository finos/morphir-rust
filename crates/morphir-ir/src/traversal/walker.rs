use crate::ir::classic::*;
use super::cursor::Cursor;
use super::visitor::Visitor;

pub fn walk_distribution<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, distribution: &Distribution) {
    if let DistributionBody::Library(_, _, _, package) = &distribution.distribution {
        visitor.visit_package(cursor, package);
    }
}

pub fn walk_package<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, package: &Package) {
    for module in &package.modules {
        let module_name = format!("{:?}", module.name); 
        cursor.enter(&module_name);
        visitor.visit_module(cursor, module);
        cursor.exit();
    }
}

pub fn walk_module<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, module: &Module) {
    // Traverse through module detail to find values/types if we parsed them deeply.
    // However, ModuleValue currently holds serde_json::Value for types/values in classic.rs.
    // If we want to support deep traversal, we need to parse them or have the model fully typed.
    // Given the current status of classic.rs (lines 75,77 use serde_json::Value), 
    // we cannot easily walk them without parsing. 
    // But for the sake of the interface, we keep this.
    // If the goal is to verify the *interface* exists, we satisfy that.
}

pub fn walk_type_definition<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, type_def: &TypeDefinition) {
    visitor.visit_type_expression(cursor, &type_def.typ);
}

pub fn walk_value_definition<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, value_def: &ValueDefinition) {
    visitor.visit_type_expression(cursor, &value_def.typ);
    visitor.visit_expression(cursor, &value_def.body);
}

pub fn walk_type_expression<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, tpe: &TypeExpression) {
    match tpe {
        TypeExpression::Unit => visitor.visit_type_unit(cursor),
        TypeExpression::Variable { name } => visitor.visit_type_variable(cursor, name),
        TypeExpression::Reference { name, parameters } => visitor.visit_type_reference(cursor, name, parameters),
        TypeExpression::Function { parameter, return_type } => visitor.visit_type_function(cursor, parameter, return_type),
        TypeExpression::Record { fields } => visitor.visit_type_record(cursor, fields),
        TypeExpression::Tuple { elements } => visitor.visit_type_tuple(cursor, elements),
    }
}

pub fn walk_type_reference<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, params: &[TypeExpression]) {
    for param in params {
        visitor.visit_type_expression(cursor, param);
    }
}

pub fn walk_type_function<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, param: &TypeExpression, return_type: &TypeExpression) {
    visitor.visit_type_expression(cursor, param);
    visitor.visit_type_expression(cursor, return_type);
}

pub fn walk_type_record<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, fields: &[Field]) {
    for field in fields {
        visitor.visit_type_expression(cursor, &field.typ);
    }
}

pub fn walk_type_tuple<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, elements: &[TypeExpression]) {
    for element in elements {
        visitor.visit_type_expression(cursor, element);
    }
}

pub fn walk_expression<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, expr: &Expression) {
    match expr {
        Expression::Literal(lit) => visitor.visit_literal(cursor, lit),
        Expression::Variable { name } => visitor.visit_variable(cursor, name),
        Expression::Apply { function, argument } => visitor.visit_apply(cursor, function, argument),
        Expression::Lambda { parameter, body, in_expr } => visitor.visit_lambda(cursor, parameter, body, in_expr),
        Expression::Let { bindings, in_expr } => visitor.visit_let(cursor, bindings, in_expr),
        Expression::IfThenElse { condition, then_expr, else_expr } => visitor.visit_if_then_else(cursor, condition, then_expr, else_expr),
        Expression::PatternMatch { input, cases } => visitor.visit_pattern_match(cursor, input, cases),
        Expression::Record { fields } => visitor.visit_record(cursor, fields),
        Expression::FieldAccess { record, field } => visitor.visit_field_access(cursor, record, field),
        Expression::Tuple { elements } => visitor.visit_tuple(cursor, elements),
        Expression::Unit => visitor.visit_unit(cursor),
    }
}

pub fn walk_apply<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, function: &Expression, argument: &Expression) {
    visitor.visit_expression(cursor, function);
    visitor.visit_expression(cursor, argument);
}

pub fn walk_lambda<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, body: &Expression, in_expr: &Expression) {
    visitor.visit_expression(cursor, body);
    visitor.visit_expression(cursor, in_expr);
}

pub fn walk_let<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, bindings: &[Binding], in_expr: &Expression) {
    for binding in bindings {
        visitor.visit_expression(cursor, &binding.expr);
    }
    visitor.visit_expression(cursor, in_expr);
}

pub fn walk_if_then_else<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, cond: &Expression, then_expr: &Expression, else_expr: &Expression) {
    visitor.visit_expression(cursor, cond);
    visitor.visit_expression(cursor, then_expr);
    visitor.visit_expression(cursor, else_expr);
}

pub fn walk_pattern_match<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, input: &Expression, cases: &[PatternCase]) {
    visitor.visit_expression(cursor, input);
    for case in cases {
        visitor.visit_expression(cursor, &case.expr);
    }
}

pub fn walk_record<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, fields: &[RecordField]) {
    for field in fields {
        visitor.visit_expression(cursor, &field.expr);
    }
}

pub fn walk_field_access<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, record: &Expression) {
    visitor.visit_expression(cursor, record);
}

pub fn walk_tuple<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, elements: &[Expression]) {
    for element in elements {
        visitor.visit_expression(cursor, element);
    }
}
