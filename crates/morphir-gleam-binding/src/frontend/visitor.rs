//! Frontend visitor - converts Gleam AST (ModuleIR) to Morphir IR V4
//!
//! This visitor traverses the parsed Gleam AST and converts it to Morphir IR V4
//! format, producing a Document Tree structure by default.

use crate::frontend::ast::{
    Access, Expr, Field as AstField, Literal, ModuleIR, Pattern, TypeDef, TypeExpr, ValueDef,
};
use indexmap::IndexMap;
use morphir_common::vfs::Vfs;
use morphir_core::ir::attributes::{TypeAttributes, ValueAttributes};
use morphir_core::ir::literal::Literal as MorphirLiteral;
use morphir_core::ir::pattern::Pattern as MorphirPattern;
use morphir_core::ir::type_expr::{Field, Type};
use morphir_core::ir::v4::{
    Access as MorphirAccess, AccessControlledConstructors, AccessControlledTypeDefinition,
    AccessControlledValueDefinition, ConstructorArg, ConstructorDefinition, InputTypeEntry,
    TypeDefinition as V4TypeDefinition, ValueBody as V4ValueBody,
    ValueDefinition as V4ValueDefinition,
};
use morphir_core::ir::value_expr::{RecordFieldEntry, Value, ValueBody};
use morphir_core::naming::{FQName, ModuleName, Name, PackageName};
use serde_json;
use std::io::Result;
use std::path::{Path, PathBuf};


/// Distribution layout mode
#[derive(Debug, Clone, Copy)]
pub enum DistributionLayout {
    /// Document Tree (VFS mode) - default
    VfsMode,
    /// Classic single JSON blob
    Classic,
}

/// Visitor that converts Gleam AST to Morphir IR V4
pub struct GleamToMorphirVisitor<V: Vfs> {
    vfs: V,
    output_dir: PathBuf,
    package_name: PackageName,
    module_name: ModuleName,
    layout: DistributionLayout,
}

impl<V: Vfs> GleamToMorphirVisitor<V> {
    /// Create a new visitor
    pub fn new(
        vfs: V,
        output_dir: PathBuf,
        package_name: PackageName,
        module_name: ModuleName,
    ) -> Self {
        Self {
            vfs,
            output_dir,
            package_name,
            module_name,
            layout: DistributionLayout::VfsMode, // Default to document tree
        }
    }

    /// Convert ModuleIR to Morphir IR V4 in Document Tree format
    pub fn visit_module_v4(&self, module_ir: &ModuleIR) -> Result<()> {
        match self.layout {
            DistributionLayout::VfsMode => self.visit_module_vfs_mode(module_ir),
            DistributionLayout::Classic => self.visit_module_classic(module_ir),
        }
    }

    /// Build format.json structure in memory without disk I/O
    pub fn build_format_json(&self) -> serde_json::Value {
        serde_json::json!({
            "formatVersion": "4.0.0",
            "distribution": "Library",
            "packageName": self.package_name.to_string(),
            "layout": "VfsMode"
        })
    }

    /// Visit module and write Document Tree structure
    fn visit_module_vfs_mode(&self, module_ir: &ModuleIR) -> Result<()> {
        // Create .morphir-dist/pkg/package-name/module-path/ structure
        let module_dir = self
            .output_dir
            .join(".morphir-dist")
            .join("pkg")
            .join(self.package_name.to_string())
            .join(self.module_name.to_string());

        self.vfs.create_dir_all(&module_dir)?;

        // Write module.json manifest
        self.write_module_manifest(&module_dir, module_ir)?;

        // Create types/ and values/ directories
        let types_dir = module_dir.join("types");
        let values_dir = module_dir.join("values");
        self.vfs.create_dir_all(&types_dir)?;
        self.vfs.create_dir_all(&values_dir)?;

        // Write individual type definition files
        for type_def in &module_ir.types {
            self.write_type_definition(&types_dir, type_def)?;
        }

        // Write individual value definition files
        for value_def in &module_ir.values {
            self.write_value_definition(&values_dir, value_def)?;
        }

        // Write format.json at root if it doesn't exist
        self.ensure_format_json()?;

        Ok(())
    }

    /// Write module.json manifest
    fn write_module_manifest(&self, module_dir: &Path, module_ir: &ModuleIR) -> Result<()> {
        let manifest = serde_json::json!({
            "module": self.module_name.to_string(),
            "doc": module_ir.doc,
            "types": module_ir.types.iter().map(|t| t.name.clone()).collect::<Vec<_>>(),
            "values": module_ir.values.iter().map(|v| v.name.clone()).collect::<Vec<_>>(),
        });

        let manifest_path = module_dir.join("module.json");
        let content = serde_json::to_string_pretty(&manifest)?;
        self.vfs.write_from_string(&manifest_path, &content)?;
        Ok(())
    }

    /// Write individual type definition file
    fn write_type_definition(&self, types_dir: &Path, type_def: &TypeDef) -> Result<()> {
        // Convert to Morphir IR type definition
        let morphir_type_def = self.convert_type_def(type_def)?;

        // Serialize to JSON using V4 format
        let json = serde_json::to_string_pretty(&morphir_type_def)?;

        // Write to types/{type-name}.type.json
        let file_path = types_dir.join(format!("{}.type.json", type_def.name));
        self.vfs.write_from_string(&file_path, &json)?;
        Ok(())
    }

    /// Write individual value definition file
    fn write_value_definition(&self, values_dir: &Path, value_def: &ValueDef) -> Result<()> {
        // Convert to Morphir IR value definition
        let morphir_value_def = self.convert_value_def(value_def)?;

        // Serialize to JSON using V4 format
        let json = serde_json::to_string_pretty(&morphir_value_def)?;

        // Write to values/{value-name}.value.json
        let file_path = values_dir.join(format!("{}.value.json", value_def.name));
        self.vfs.write_from_string(&file_path, &json)?;
        Ok(())
    }

    /// Ensure format.json exists at output root
    fn ensure_format_json(&self) -> Result<()> {
        // output_dir is already .morphir/out/<project>/compile/<language>/
        let format_path = self.output_dir.join("format.json");

        // Only create if it doesn't exist
        if !self.vfs.exists(&format_path) {
            let format_json = serde_json::json!({
                "formatVersion": "4.0.0",
                "distribution": "Library",
                "packageName": self.package_name.to_string(),
                "layout": "VfsMode"
            });

            let content = serde_json::to_string_pretty(&format_json)?;
            self.vfs.write_from_string(&format_path, &content)?;
        }

        Ok(())
    }

    /// Visit module and return single PackageDefinition (Classic mode)
    fn visit_module_classic(&self, module_ir: &ModuleIR) -> Result<()> {
        // For Classic mode, we could build a PackageDefinition in memory
        // For now, just delegate to VfsMode
        self.visit_module_vfs_mode(module_ir)
    }

    // ========================================================================
    // Conversion Methods
    // ========================================================================

    /// Convert TypeDef to Morphir IR TypeDefinition
    fn convert_type_def(&self, type_def: &TypeDef) -> Result<AccessControlledTypeDefinition> {
        let access = match type_def.access {
            Access::Public => MorphirAccess::Public,
            Access::Private => MorphirAccess::Private,
        };

        // Convert type parameters
        let type_params: Vec<Name> = type_def
            .params
            .iter()
            .map(|p| Name::from(p.as_str()))
            .collect();

        // Convert type body
        match &type_def.body {
            TypeExpr::CustomType { variants } => {
                let constructors: Vec<ConstructorDefinition> = variants
                    .iter()
                    .map(|v| {
                        let args: Vec<ConstructorArg> = v
                            .fields
                            .iter()
                            .map(|field_type| {
                                let morphir_type = self.convert_type_expr(field_type);
                                ConstructorArg {
                                    name: Name::from(""), // Gleam doesn't name constructor args
                                    arg_type: morphir_type,
                                }
                            })
                            .collect();

                        ConstructorDefinition {
                            name: Name::from(v.name.as_str()),
                            args,
                        }
                    })
                    .collect();

                let v4_type_def = V4TypeDefinition::CustomTypeDefinition {
                    type_params: type_params.clone(),
                    constructors: AccessControlledConstructors {
                        access: MorphirAccess::Public,
                        value: constructors,
                    },
                };

                Ok(AccessControlledTypeDefinition {
                    access,
                    value: v4_type_def,
                })
            }
            _ => {
                // Type alias
                let type_expr = self.convert_type_expr(&type_def.body);
                let v4_type_def = V4TypeDefinition::TypeAliasDefinition {
                    type_params: type_params.clone(),
                    type_expr,
                };

                Ok(AccessControlledTypeDefinition {
                    access,
                    value: v4_type_def,
                })
            }
        }
    }

    /// Convert ValueDef to Morphir IR ValueDefinition
    fn convert_value_def(&self, value_def: &ValueDef) -> Result<AccessControlledValueDefinition> {
        let access = match value_def.access {
            Access::Public => MorphirAccess::Public,
            Access::Private => MorphirAccess::Private,
        };

        // Convert input types (from type annotation if present)
        // V4 uses IndexMap<String, InputTypeEntry>
        let input_types: IndexMap<String, InputTypeEntry> =
            if let Some(type_ann) = &value_def.type_annotation {
                self.extract_input_types_v4(type_ann)
            } else {
                IndexMap::new()
            };

        // Convert output type
        let output_type = if let Some(type_ann) = &value_def.type_annotation {
            self.extract_output_type_type(type_ann)
        } else {
            Type::Unit(TypeAttributes::default())
        };

        // Convert body expression
        let body_value = self.convert_expr(&value_def.body);
        let body = V4ValueBody::ExpressionBody { body: body_value };

        let v4_value_def = V4ValueDefinition {
            input_types,
            output_type,
            body,
        };

        Ok(AccessControlledValueDefinition {
            access,
            value: v4_value_def,
        })
    }

    /// Extract input types from function type annotation (returns V4 IndexMap format)
    fn extract_input_types_v4(&self, type_expr: &TypeExpr) -> IndexMap<String, InputTypeEntry> {
        let mut inputs = IndexMap::new();

        // Extract function argument types
        // For now, simple extraction - can be enhanced to handle curried functions
        if let TypeExpr::Function {
            parameters,
            return_type: _,
        } = type_expr
        {
            for (i, param) in parameters.iter().enumerate() {
                let morphir_type = self.convert_type_expr(param);
                inputs.insert(
                    format!("arg{}", i + 1),
                    InputTypeEntry {
                        type_attributes: None,
                        input_type: morphir_type,
                    },
                );
            }
        }

        inputs
    }

    /// Extract output type from function type annotation (returns Type)
    fn extract_output_type_type(&self, type_expr: &TypeExpr) -> Type {
        match type_expr {
            TypeExpr::Function {
                parameters: _,
                return_type,
            } => self.convert_type_expr(return_type),
            _ => self.convert_type_expr(type_expr),
        }
    }

    /// Convert TypeExpr to Morphir IR Type
    pub(crate) fn convert_type_expr(&self, type_expr: &TypeExpr) -> Type {
        let attrs = TypeAttributes::default();

        match type_expr {
            TypeExpr::Variable { name } => Type::Variable(attrs, Name::from(name.as_str())),
            TypeExpr::Unit => Type::Unit(attrs),
            TypeExpr::Function {
                parameters,
                return_type,
            } => {
                // Convert multi-param function to curried form
                let mut result = self.convert_type_expr(return_type);
                for param in parameters.iter().rev() {
                    result = Type::Function(
                        attrs.clone(),
                        Box::new(self.convert_type_expr(param)),
                        Box::new(result),
                    );
                }
                result
            }
            TypeExpr::Record { fields } => {
                let morphir_fields: Vec<Field> = fields
                    .iter()
                    .map(|(name, tpe)| Field {
                        name: Name::from(name.as_str()),
                        tpe: self.convert_type_expr(tpe),
                    })
                    .collect();
                Type::Record(attrs, morphir_fields)
            }
            TypeExpr::Tuple { elements } => {
                let morphir_elements: Vec<Type> =
                    elements.iter().map(|e| self.convert_type_expr(e)).collect();
                Type::Tuple(attrs, morphir_elements)
            }
            TypeExpr::Named {
                module: _,
                name,
                parameters,
            } => {
                // Build FQName from reference name
                // For now, assume it's in the current module
                let fqname = FQName {
                    package_path: self.package_name.clone().into(),
                    module_path: self.module_name.clone().into(),
                    local_name: Name::from(name.as_str()),
                };

                let morphir_args: Vec<Type> = parameters
                    .iter()
                    .map(|a| self.convert_type_expr(a))
                    .collect();

                Type::Reference(attrs, fqname, morphir_args)
            }
            TypeExpr::CustomType { .. } => {
                // Custom types are handled at definition level
                Type::Unit(attrs) // Placeholder
            }
            TypeExpr::Hole { .. } => {
                // Type holes are placeholders
                Type::Unit(attrs)
            }
        }
    }

    /// Helper to extract expression from Field<Expr>
    fn extract_field_expr(&self, field: &AstField<Expr>) -> Value {
        match field {
            AstField::Labelled { item, .. } => self.convert_expr(item),
            AstField::Shorthand { name } => {
                Value::Variable(ValueAttributes::default(), Name::from(name.as_str()))
            }
            AstField::Unlabelled { item } => self.convert_expr(item),
        }
    }

    /// Convert Expr to Morphir IR Value
    pub(crate) fn convert_expr(&self, expr: &Expr) -> Value {
        let attrs = ValueAttributes::default();

        match expr {
            Expr::Literal { value } => Value::Literal(attrs, self.convert_literal(value)),
            Expr::Variable { name } => Value::Variable(attrs, Name::from(name.as_str())),
            Expr::Apply {
                function,
                arguments,
            } => {
                // Convert multiple arguments to curried apply
                let mut result = self.convert_expr(function);
                for arg in arguments {
                    result = Value::Apply(
                        attrs.clone(),
                        Box::new(result),
                        Box::new(self.extract_field_expr(arg)),
                    );
                }
                result
            }
            Expr::Lambda { params, body } => {
                // Convert multi-param lambda to curried form
                let mut result = self.convert_expr(body);
                for param in params.iter().rev() {
                    result = Value::Lambda(
                        attrs.clone(),
                        MorphirPattern::AsPattern(
                            ValueAttributes::default(),
                            Box::new(MorphirPattern::WildcardPattern(ValueAttributes::default())),
                            Name::from(param.as_str()),
                        ),
                        Box::new(result),
                    );
                }
                result
            }
            Expr::Let { name, value, body } => {
                // Convert to LetDefinition
                let def = morphir_core::ir::value_expr::ValueDefinition {
                    input_types: vec![],
                    output_type: Type::Unit(TypeAttributes::default()),
                    body: ValueBody::Expression(self.convert_expr(value)),
                };

                Value::LetDefinition(
                    attrs,
                    Name::from(name.as_str()),
                    Box::new(def),
                    Box::new(self.convert_expr(body)),
                )
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => Value::IfThenElse(
                attrs,
                Box::new(self.convert_expr(condition)),
                Box::new(self.convert_expr(then_branch)),
                Box::new(self.convert_expr(else_branch)),
            ),
            Expr::Record { fields } => {
                let morphir_fields: Vec<RecordFieldEntry> = fields
                    .iter()
                    .map(|(name, expr)| {
                        RecordFieldEntry(Name::from(name.as_str()), self.convert_expr(expr))
                    })
                    .collect();
                Value::Record(attrs, morphir_fields)
            }
            Expr::FieldAccess { container, label } => Value::Field(
                attrs,
                Box::new(self.convert_expr(container)),
                Name::from(label.as_str()),
            ),
            Expr::Tuple { elements } => {
                let morphir_elements: Vec<Value> =
                    elements.iter().map(|e| self.convert_expr(e)).collect();
                Value::Tuple(attrs, morphir_elements)
            }
            Expr::TupleIndex { tuple, index } => {
                // Convert to field access with numeric index
                Value::Field(
                    attrs,
                    Box::new(self.convert_expr(tuple)),
                    Name::from(format!("{}", index).as_str()),
                )
            }
            Expr::Case { subjects, clauses } => {
                // For now, handle single subject case
                let subject = subjects
                    .first()
                    .map(|s| self.convert_expr(s))
                    .unwrap_or_else(|| Value::Unit(attrs.clone()));

                let morphir_cases: Vec<morphir_core::ir::value_expr::PatternCase> = clauses
                    .iter()
                    .map(|branch| {
                        morphir_core::ir::value_expr::PatternCase(
                            self.convert_pattern(&branch.pattern),
                            self.convert_expr(&branch.body),
                        )
                    })
                    .collect();

                Value::PatternMatch(attrs, Box::new(subject), morphir_cases)
            }
            Expr::Constructor { module: _, name } => {
                // Build FQName for constructor
                let fqname = FQName {
                    package_path: self.package_name.clone().into(),
                    module_path: self.module_name.clone().into(),
                    local_name: Name::from(name.as_str()),
                };
                Value::Constructor(attrs, fqname)
            }
            Expr::BinaryOp { op, left, right } => {
                // Convert binary op to function application
                let op_name = format!("{:?}", op).to_lowercase();
                let fqname = FQName {
                    package_path: self.package_name.clone().into(),
                    module_path: self.module_name.clone().into(),
                    local_name: Name::from(op_name.as_str()),
                };
                let op_ref = Value::Reference(attrs.clone(), fqname);
                Value::Apply(
                    attrs.clone(),
                    Box::new(Value::Apply(
                        attrs.clone(),
                        Box::new(op_ref),
                        Box::new(self.convert_expr(left)),
                    )),
                    Box::new(self.convert_expr(right)),
                )
            }
            Expr::NegateInt { value } => {
                // Convert to negate function application
                let fqname = FQName {
                    package_path: self.package_name.clone().into(),
                    module_path: self.module_name.clone().into(),
                    local_name: Name::from("negate"),
                };
                Value::Apply(
                    attrs.clone(),
                    Box::new(Value::Reference(attrs, fqname)),
                    Box::new(self.convert_expr(value)),
                )
            }
            Expr::NegateBool { value } => {
                // Convert to not function application
                let fqname = FQName {
                    package_path: self.package_name.clone().into(),
                    module_path: self.module_name.clone().into(),
                    local_name: Name::from("not"),
                };
                Value::Apply(
                    attrs.clone(),
                    Box::new(Value::Reference(attrs, fqname)),
                    Box::new(self.convert_expr(value)),
                )
            }
            Expr::List { elements, tail } => {
                let morphir_elements: Vec<Value> =
                    elements.iter().map(|e| self.convert_expr(e)).collect();
                // For now, ignore tail and just create a list
                let _ = tail; // TODO: Handle tail properly
                Value::List(attrs, morphir_elements)
            }
            Expr::Block { statements } => {
                // Convert block to sequence of let bindings
                // For now, just return the last expression
                if let Some(last) = statements.last() {
                    match last {
                        crate::frontend::ast::Statement::Expression(e) => self.convert_expr(e),
                        _ => Value::Unit(attrs),
                    }
                } else {
                    Value::Unit(attrs)
                }
            }
            Expr::Panic { message } => {
                // Convert to panic function call
                let msg_expr = message
                    .as_ref()
                    .map(|m| self.convert_expr(m))
                    .unwrap_or_else(|| {
                        Value::Literal(attrs.clone(), MorphirLiteral::string("panic"))
                    });
                let _ = msg_expr; // TODO: Use message in panic representation
                Value::Unit(attrs) // Placeholder - Morphir IR doesn't have panic
            }
            Expr::Todo { message } => {
                // Convert to todo placeholder
                let _ = message;
                Value::Unit(attrs) // Placeholder
            }
            Expr::Echo {
                expression,
                body: _,
            } => {
                // Echo just returns its expression
                self.convert_expr(expression)
            }
            Expr::BitString { .. } => {
                // Bit strings not yet supported
                Value::Unit(attrs)
            }
            Expr::FnCapture { .. } => {
                // Function capture not yet supported
                Value::Unit(attrs)
            }
            Expr::RecordUpdate { .. } => {
                // Record update not yet supported
                Value::Unit(attrs)
            }
        }
    }

    /// Helper to extract pattern from Field<Pattern>
    fn extract_field_pattern(&self, field: &AstField<Pattern>) -> MorphirPattern {
        match field {
            AstField::Labelled { item, .. } => self.convert_pattern(item),
            AstField::Shorthand { name } => MorphirPattern::AsPattern(
                ValueAttributes::default(),
                Box::new(MorphirPattern::WildcardPattern(ValueAttributes::default())),
                Name::from(name.as_str()),
            ),
            AstField::Unlabelled { item } => self.convert_pattern(item),
        }
    }

    /// Convert Pattern to Morphir IR Pattern
    fn convert_pattern(&self, pattern: &Pattern) -> MorphirPattern {
        let attrs = ValueAttributes::default();

        match pattern {
            Pattern::Wildcard => MorphirPattern::WildcardPattern(attrs),
            Pattern::Variable { name } => MorphirPattern::AsPattern(
                attrs.clone(),
                Box::new(MorphirPattern::WildcardPattern(attrs)),
                Name::from(name.as_str()),
            ),
            Pattern::Discard { name: _ } => MorphirPattern::WildcardPattern(attrs),
            Pattern::Literal { value } => {
                MorphirPattern::LiteralPattern(attrs, self.convert_literal(value))
            }
            Pattern::Constructor {
                module: _,
                name,
                arguments,
                with_spread: _,
            } => {
                let fqname = FQName {
                    package_path: self.package_name.clone().into(),
                    module_path: self.module_name.clone().into(),
                    local_name: Name::from(name.as_str()),
                };

                let morphir_args: Vec<MorphirPattern> = arguments
                    .iter()
                    .map(|a| self.extract_field_pattern(a))
                    .collect();

                MorphirPattern::ConstructorPattern(attrs, fqname, morphir_args)
            }
            Pattern::Tuple { elements } => {
                let morphir_elements: Vec<MorphirPattern> =
                    elements.iter().map(|e| self.convert_pattern(e)).collect();
                MorphirPattern::TuplePattern(attrs, morphir_elements)
            }
            Pattern::List { elements, tail } => {
                // Convert to nested HeadTailPattern or EmptyListPattern
                let morphir_elements: Vec<MorphirPattern> =
                    elements.iter().map(|e| self.convert_pattern(e)).collect();
                // For now, just convert to tuple pattern (Morphir IR list patterns)
                let _ = tail; // TODO: Handle tail properly
                MorphirPattern::TuplePattern(attrs, morphir_elements)
            }
            Pattern::Assignment { pattern, name } => MorphirPattern::AsPattern(
                attrs.clone(),
                Box::new(self.convert_pattern(pattern)),
                Name::from(name.as_str()),
            ),
            Pattern::Concatenate { .. } => {
                // String concatenation patterns not directly supported
                MorphirPattern::WildcardPattern(attrs)
            }
            Pattern::BitString { .. } => {
                // Bit string patterns not yet supported
                MorphirPattern::WildcardPattern(attrs)
            }
        }
    }

    /// Convert Literal to Morphir IR Literal
    fn convert_literal(&self, literal: &Literal) -> MorphirLiteral {
        match literal {
            Literal::Bool { value } => MorphirLiteral::Bool(*value),
            Literal::Int { value } => MorphirLiteral::Integer(*value),
            Literal::Float { value } => MorphirLiteral::Float(*value),
            Literal::String { value } => MorphirLiteral::String(value.clone()),
            Literal::Char { value } => MorphirLiteral::Char(*value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::ast::{Access, Expr, Literal, ModuleIR, ValueDef};
    use morphir_common::vfs::MemoryVfs;
    use morphir_core::naming::{ModuleName, PackageName};
    use std::path::PathBuf;

    #[test]
    fn test_visit_simple_module() {
        let vfs = MemoryVfs::new();
        let output_dir = PathBuf::from("/test");
        let package_name = PackageName::parse("test-package");
        let module_name = ModuleName::parse("test_module");

        let visitor = GleamToMorphirVisitor::new(vfs, output_dir, package_name, module_name);

        let module_ir = ModuleIR {
            name: "test_module".to_string(),
            doc: None,
            types: vec![],
            values: vec![ValueDef {
                name: "hello".to_string(),
                type_annotation: None,
                body: Expr::Literal {
                    value: Literal::String {
                        value: "world".to_string(),
                    },
                },
                access: Access::Public,
            }],
        };

        // Test that visitor can process module
        let result = visitor.visit_module_v4(&module_ir);
        assert!(result.is_ok());
    }
}
