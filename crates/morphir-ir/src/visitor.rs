//! Visitor pattern for Morphir IR.
//! 
//! Note: The Cursor struct is a placeholder for future cursor-based navigation.
//! Version-specific visitors (ClassicVisitor, V4Visitor) are needed for deep traversal.

use crate::ir::classic::*;

/// A simple Visitor trait for traversing the IR.
/// 
/// Default implementations call the corresponding `walk_*` function, enabling deep traversal.
/// To stop traversal, override the method and do not call `walk_*`.
pub trait Visitor: Sized {
    fn visit_distribution(&mut self, distribution: &Distribution) {
        walk_distribution(self, distribution);
    }
    
    fn visit_package(&mut self, package: &Package) {
        walk_package(self, package);
    }
    
    fn visit_module(&mut self, module: &Module) {
        walk_module(self, module);
    }
    
    fn visit_type_definition(&mut self, type_def: &TypeDefinition) {
        walk_type_definition(self, type_def);
    }
    
    fn visit_value_definition(&mut self, value_def: &ValueDefinition) {
        walk_value_definition(self, value_def);
    }
    
    fn visit_type_expression(&mut self, _tpe: &TypeExpression) {
        // Default: no-op
    }
    
    fn visit_expression(&mut self, _expr: &Expression) {
        // Default: no-op
    }
}

pub fn walk_distribution<V: Visitor>(visitor: &mut V, distribution: &Distribution) {
    if let DistributionBody::Library(_, _, _, package) = &distribution.distribution {
        visitor.visit_package(package);
    }
}

pub fn walk_package<V: Visitor>(visitor: &mut V, package: &Package) {
    for module in &package.modules {
        visitor.visit_module(module);
    }
}

pub fn walk_module<V: Visitor>(_visitor: &mut V, _module: &Module) {
    // types/values are serde_json::Value for flexible legacy format parsing.
    // Version-specific visitors are needed for deep traversal into type/value definitions.
}

pub fn walk_type_definition<V: Visitor>(_visitor: &mut V, _type_def: &TypeDefinition) {
    // Placeholder - version-specific visitors needed
}

pub fn walk_value_definition<V: Visitor>(_visitor: &mut V, _value_def: &ValueDefinition) {
    // Placeholder - version-specific visitors needed
}

pub fn walk_expression<V: Visitor>(_visitor: &mut V, _expr: &Expression) {
    // Placeholder - version-specific visitors needed
}

/// A Cursor for navigating the IR (placeholder for future enhancement).
#[derive(Debug, Clone, PartialEq)]
pub struct Cursor<'a> {
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> Cursor<'a> {
    pub fn new() -> Self {
        Self { _marker: std::marker::PhantomData }
    }
}
