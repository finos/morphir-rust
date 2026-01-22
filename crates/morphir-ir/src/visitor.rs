//! Visitor pattern for Morphir IR.
//! 
//! Note: The Cursor struct provides context for traversal (path, depth).
//! Version-specific visitors (ClassicVisitor, V4Visitor) are needed for deep traversal.

use crate::ir::classic::*;
use crate::naming::Path;

/// A Cursor for navigating the IR with context.
#[derive(Debug, Clone, Default)]
pub struct Cursor {
    path_stack: Vec<String>,
    depth: usize,
}

impl Cursor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enter a named segment of the IR tree (pushes to path stack).
    pub fn enter(&mut self, segment: &str) {
        self.path_stack.push(segment.to_string());
        self.depth += 1;
    }

    /// Exit the current segment (pops from path stack).
    pub fn exit(&mut self) {
        self.path_stack.pop();
        self.depth -= 1;
    }

    /// Advance to the next sibling (currently a placeholder for sibling tracking).
    pub fn next(&mut self) {
        // Future: track index or sibling state
    }

    /// Get the current traversal path as a Morphir Path.
    pub fn path(&self) -> Path {
        // Simple conversion: treating stack segments as path words
        // In reality, this might need refinement based on exact Path structure
        // Assuming Name::from takes string
        let segments: Vec<crate::naming::Name> = self.path_stack.iter()
            .map(|s| crate::naming::Name::from(s.as_str()))
            .collect();
        Path { segments }
    }

    /// Get current nesting depth.
    pub fn depth(&self) -> usize {
        self.depth
    }
}

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
    
    fn visit_type_expression(&mut self, _cursor: &mut Cursor, _tpe: &TypeExpression) {
        // Default: no-op
    }
    
    fn visit_expression(&mut self, _cursor: &mut Cursor, _expr: &Expression) {
        // Default: no-op
    }
}

pub fn walk_distribution<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, distribution: &Distribution) {
    if let DistributionBody::Library(_, _, _, package) = &distribution.distribution {
        // Note: Package usually defines the root namespace, but we might want to push its name?
        // For now, simple delegation.
        visitor.visit_package(cursor, package);
    }
}

pub fn walk_package<V: Visitor>(visitor: &mut V, cursor: &mut Cursor, package: &Package) {
    for module in &package.modules {
        // Module name is a Path, we convert to string representation for cursor
        // This is a simplification; ideally Cursor handles Path objects directly
        let module_name = format!("{:?}", module.name); 
        cursor.enter(&module_name);
        visitor.visit_module(cursor, module);
        cursor.exit();
    }
}

pub fn walk_module<V: Visitor>(_visitor: &mut V, _cursor: &mut Cursor, _module: &Module) {
    // types/values are serde_json::Value for flexible legacy format parsing.
    // Version-specific visitors are needed for deep traversal into type/value definitions.
}

pub fn walk_type_definition<V: Visitor>(_visitor: &mut V, _cursor: &mut Cursor, _type_def: &TypeDefinition) {
    // Placeholder - version-specific visitors needed
}

pub fn walk_value_definition<V: Visitor>(_visitor: &mut V, _cursor: &mut Cursor, _value_def: &ValueDefinition) {
    // Placeholder - version-specific visitors needed
}

pub fn walk_expression<V: Visitor>(_visitor: &mut V, _cursor: &mut Cursor, _expr: &Expression) {
    // Placeholder - version-specific visitors needed
}
