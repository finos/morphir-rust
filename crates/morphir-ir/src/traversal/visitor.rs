use crate::ir::classic::*;
use super::cursor::Cursor;
use super::walker::*;

/// A simple Visitor trait for traversing the IR with generic Context (Cursor).
pub trait Visitor: Sized {
    /// Entry point for traversal - automatically initializes a Cursor.
    fn traverse(&mut self, distribution: &Distribution) {
        let mut cursor = Cursor::new();
        self.visit_distribution(&mut cursor, distribution);
    }
    
    fn visit_distribution(&mut self, cursor: &mut Cursor, distribution: &Distribution) {
        walk_distribution(self, cursor, distribution);
    }
    
    fn visit_package(&mut self, cursor: &mut Cursor, package: &Package) {
        walk_package(self, cursor, package);
    }
    
    fn visit_module(&mut self, cursor: &mut Cursor, module: &Module) {
        walk_module(self, cursor, module);
    }
    
    fn visit_type_definition(&mut self, cursor: &mut Cursor, type_def: &TypeDefinition) {
        walk_type_definition(self, cursor, type_def);
    }
    
    fn visit_value_definition(&mut self, cursor: &mut Cursor, value_def: &ValueDefinition) {
        walk_value_definition(self, cursor, value_def);
    }
    
    fn visit_type_expression(&mut self, cursor: &mut Cursor, tpe: &TypeExpression) {
        walk_type_expression(self, cursor, tpe);
    }
    
    // Type Visitor Methods
    fn visit_type_unit(&mut self, _cursor: &mut Cursor) { 
        // No children to walk
    }
    
    fn visit_type_variable(&mut self, _cursor: &mut Cursor, _name: &str) {
        // No children to walk
    }
    
    fn visit_type_reference(&mut self, cursor: &mut Cursor, _name: &str, params: &[TypeExpression]) {
        walk_type_reference(self, cursor, params);
    }
    
    fn visit_type_function(&mut self, cursor: &mut Cursor, param: &TypeExpression, return_type: &TypeExpression) {
        walk_type_function(self, cursor, param, return_type);
    }
    
    fn visit_type_record(&mut self, cursor: &mut Cursor, fields: &[Field]) {
        walk_type_record(self, cursor, fields);
    }
    
    fn visit_type_tuple(&mut self, cursor: &mut Cursor, elements: &[TypeExpression]) {
        walk_type_tuple(self, cursor, elements);
    }

    fn visit_expression(&mut self, cursor: &mut Cursor, expr: &Expression) {
        walk_expression(self, cursor, expr);
    }
    
    // Value Visitor Methods
    fn visit_literal(&mut self, _cursor: &mut Cursor, _literal: &Literal) {
        // No children
    }
    
    fn visit_variable(&mut self, _cursor: &mut Cursor, _name: &str) {
        // No children
    }
    
    fn visit_apply(&mut self, cursor: &mut Cursor, function: &Expression, argument: &Expression) {
        walk_apply(self, cursor, function, argument);
    }
    
    fn visit_lambda(&mut self, cursor: &mut Cursor, _parameter: &str, body: &Expression, _in_expr: &Expression) {
        walk_lambda(self, cursor, body, _in_expr);
    }
    
    fn visit_let(&mut self, cursor: &mut Cursor, bindings: &[Binding], in_expr: &Expression) {
        walk_let(self, cursor, bindings, in_expr);
    }
    
    fn visit_if_then_else(&mut self, cursor: &mut Cursor, cond: &Expression, then_expr: &Expression, else_expr: &Expression) {
        walk_if_then_else(self, cursor, cond, then_expr, else_expr);
    }
    
    fn visit_pattern_match(&mut self, cursor: &mut Cursor, input: &Expression, cases: &[PatternCase]) {
        walk_pattern_match(self, cursor, input, cases);
    }
    
    fn visit_record(&mut self, cursor: &mut Cursor, fields: &[RecordField]) {
        walk_record(self, cursor, fields);
    }
    
    fn visit_field_access(&mut self, cursor: &mut Cursor, record: &Expression, _field: &str) {
        walk_field_access(self, cursor, record);
    }
    
    fn visit_tuple(&mut self, cursor: &mut Cursor, elements: &[Expression]) {
        walk_tuple(self, cursor, elements);
    }
    
    fn visit_unit(&mut self, _cursor: &mut Cursor) {
        // No children
    }
}
