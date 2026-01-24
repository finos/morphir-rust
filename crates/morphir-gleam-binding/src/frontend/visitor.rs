//! Frontend visitor - converts Gleam AST (ModuleIR) to Morphir IR V4
//!
//! This visitor traverses the parsed Gleam AST and converts it to Morphir IR V4
//! format, producing a Document Tree structure by default.

use crate::frontend::parser::{Access, Expr, Literal, ModuleIR, Pattern, TypeDef, TypeExpr, ValueDef};
use indexmap::IndexMap;
use morphir_common::vfs::Vfs;
use morphir_ir::ir::attributes::{TypeAttributes, ValueAttributes};
use morphir_ir::ir::literal::Literal as MorphirLiteral;
use morphir_ir::ir::pattern::Pattern as MorphirPattern;
use morphir_ir::ir::type_expr::{Field, Type};
use morphir_ir::ir::value_expr::{RecordFieldEntry, Value, ValueDefinition, ValueBody};
use morphir_ir::ir::type_def::TypeDefinition;
use morphir_ir::ir::v4::{
    Access as MorphirAccess, AccessControlledConstructors, AccessControlledModuleDefinition,
    AccessControlledTypeDefinition, AccessControlledValueDefinition, ConstructorArg,
    ConstructorDefinition, ModuleDefinition, PackageDefinition,
};
use morphir_ir::naming::{FQName, ModuleName, Name, PackageName};
use serde_json;
use std::io::Result;
use std::path::PathBuf;

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
    fn write_module_manifest(&self, module_dir: &PathBuf, module_ir: &ModuleIR) -> Result<()> {
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
    fn write_type_definition(&self, types_dir: &PathBuf, type_def: &TypeDef) -> Result<()> {
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
    fn write_value_definition(&self, values_dir: &PathBuf, value_def: &ValueDef) -> Result<()> {
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
    fn convert_type_def(
        &self,
        type_def: &TypeDef,
    ) -> Result<AccessControlledTypeDefinition> {
        use morphir_ir::ir::type_def::TypeDefinition;

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
                                    arg_type: serde_json::to_value(&morphir_type)
                                        .unwrap_or(serde_json::Value::Null),
                                }
                            })
                            .collect();

                        ConstructorDefinition {
                            name: Name::from(v.name.as_str()),
                            args,
                        }
                    })
                    .collect();

                let type_def = TypeDefinition::CustomTypeDefinition {
                    type_params: type_params.clone(),
                    constructors: AccessControlledConstructors {
                        access: MorphirAccess::Public,
                        value: constructors,
                    },
                };

                Ok(AccessControlledTypeDefinition {
                    access,
                    value: type_def,
                })
            }
            _ => {
                // Type alias
                let type_expr = self.convert_type_expr(&type_def.body);
                let type_def = TypeDefinition::TypeAliasDefinition {
                    type_params: type_params.clone(),
                    type_expr: serde_json::to_value(&type_expr)
                        .unwrap_or(serde_json::Value::Null),
                };

                Ok(AccessControlledTypeDefinition {
                    access,
                    value: type_def,
                })
            }
        }
    }

    /// Convert ValueDef to Morphir IR ValueDefinition
    fn convert_value_def(
        &self,
        value_def: &ValueDef,
    ) -> Result<AccessControlledValueDefinition> {
        let access = match value_def.access {
            Access::Public => MorphirAccess::Public,
            Access::Private => MorphirAccess::Private,
        };

        // Convert input types (from type annotation if present)
        // V4 uses Vec<InputType> not IndexMap
        let input_types: Vec<morphir_ir::ir::value_expr::InputType<TypeAttributes, ValueAttributes>> = 
            if let Some(type_ann) = &value_def.type_annotation {
                // Extract input types from function type
                self.extract_input_types_vec(type_ann)
            } else {
                vec![]
            };

        // Convert output type
        let output_type = if let Some(type_ann) = &value_def.type_annotation {
            self.extract_output_type_type(type_ann)
        } else {
            Type::Unit(TypeAttributes::default()) // Infer later
        };

        // Convert body expression
        let body_value = self.convert_expr(&value_def.body);
        let body = ValueBody::Expression(body_value);

        let value_def = morphir_ir::ir::value_expr::ValueDefinition {
            input_types,
            output_type,
            body,
        };

        Ok(AccessControlledValueDefinition {
            access,
            value: value_def,
        })
    }

    /// Extract input types from function type annotation (returns Vec<InputType>)
    fn extract_input_types_vec(
        &self,
        type_expr: &TypeExpr,
    ) -> Vec<morphir_ir::ir::value_expr::InputType<TypeAttributes, ValueAttributes>> {
        let mut inputs = Vec::new();
        
        // Extract function argument types
        // For now, simple extraction - can be enhanced to handle curried functions
        if let TypeExpr::Function { from, to: _ } = type_expr {
            let morphir_type = self.convert_type_expr(from);
            inputs.push(morphir_ir::ir::value_expr::InputType(
                Name::from("arg1"),
                ValueAttributes::default(),
                morphir_type,
            ));
        }
        
        inputs
    }

    /// Extract output type from function type annotation (returns Type)
    fn extract_output_type_type(&self, type_expr: &TypeExpr) -> Type<TypeAttributes> {
        match type_expr {
            TypeExpr::Function { from: _, to } => self.convert_type_expr(to),
            _ => self.convert_type_expr(type_expr),
        }
    }

    /// Convert TypeExpr to Morphir IR Type
    pub(crate) fn convert_type_expr(&self, type_expr: &TypeExpr) -> Type<TypeAttributes> {
        let attrs = TypeAttributes::default();
        
        match type_expr {
            TypeExpr::Variable { name } => {
                Type::Variable(attrs, Name::from(name.as_str()))
            }
            TypeExpr::Unit => Type::Unit(attrs),
            TypeExpr::Function { from, to } => {
                Type::Function(
                    attrs,
                    Box::new(self.convert_type_expr(from)),
                    Box::new(self.convert_type_expr(to)),
                )
            }
            TypeExpr::Record { fields } => {
                let morphir_fields: Vec<Field<TypeAttributes>> = fields
                    .iter()
                    .map(|(name, tpe)| Field {
                        name: Name::from(name.as_str()),
                        tpe: self.convert_type_expr(tpe),
                    })
                    .collect();
                Type::Record(attrs, morphir_fields)
            }
            TypeExpr::Tuple { elements } => {
                let morphir_elements: Vec<Type<TypeAttributes>> = elements
                    .iter()
                    .map(|e| self.convert_type_expr(e))
                    .collect();
                Type::Tuple(attrs, morphir_elements)
            }
            TypeExpr::Reference { name, args } => {
                // Build FQName from reference name
                // For now, assume it's in the current module
                let fqname = FQName {
                    package_path: self.package_name.clone().into(),
                    module_path: self.module_name.clone().into(),
                    local_name: Name::from(name.as_str()),
                };
                
                let morphir_args: Vec<Type<TypeAttributes>> = args
                    .iter()
                    .map(|a| self.convert_type_expr(a))
                    .collect();
                
                Type::Reference(attrs, fqname, morphir_args)
            }
            TypeExpr::CustomType { .. } => {
                // Custom types are handled at definition level
                Type::Unit(attrs) // Placeholder
            }
        }
    }

    /// Convert Expr to Morphir IR Value
    pub(crate) fn convert_expr(&self, expr: &Expr) -> Value<TypeAttributes, ValueAttributes> {
        let attrs = ValueAttributes::default();
        
        match expr {
            Expr::Literal { value } => {
                Value::Literal(attrs, self.convert_literal(value))
            }
            Expr::Variable { name } => {
                Value::Variable(attrs, Name::from(name.as_str()))
            }
            Expr::Apply { function, argument } => {
                Value::Apply(
                    attrs,
                    Box::new(self.convert_expr(function)),
                    Box::new(self.convert_expr(argument)),
                )
            }
            Expr::Lambda { param, body } => {
                Value::Lambda(
                    attrs,
                    MorphirPattern::WildcardPattern(ValueAttributes::default()), // TODO: Convert param
                    Box::new(self.convert_expr(body)),
                )
            }
            Expr::Let { name, value, body } => {
                // Convert to LetDefinition
                let def = morphir_ir::ir::value_expr::ValueDefinition {
                    input_types: IndexMap::new(),
                    output_type: serde_json::Value::Null,
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
            } => {
                Value::IfThenElse(
                    attrs,
                    Box::new(self.convert_expr(condition)),
                    Box::new(self.convert_expr(then_branch)),
                    Box::new(self.convert_expr(else_branch)),
                )
            }
            Expr::Record { fields } => {
                let morphir_fields: Vec<RecordFieldEntry<TypeAttributes, ValueAttributes>> = fields
                    .iter()
                    .map(|(name, expr)| {
                        RecordFieldEntry(
                            Name::from(name.as_str()),
                            self.convert_expr(expr),
                        )
                    })
                    .collect();
                Value::Record(attrs, morphir_fields)
            }
            Expr::Field { record, field } => {
                Value::Field(
                    attrs,
                    Box::new(self.convert_expr(record)),
                    Name::from(field.as_str()),
                )
            }
            Expr::Tuple { elements } => {
                let morphir_elements: Vec<Value<TypeAttributes, ValueAttributes>> = elements
                    .iter()
                    .map(|e| self.convert_expr(e))
                    .collect();
                Value::Tuple(attrs, morphir_elements)
            }
            Expr::Case { subject, branches } => {
                let morphir_cases: Vec<morphir_ir::ir::value_expr::PatternCase<TypeAttributes, ValueAttributes>> = branches
                    .iter()
                    .map(|branch| {
                        morphir_ir::ir::value_expr::PatternCase(
                            self.convert_pattern(&branch.pattern),
                            self.convert_expr(&branch.body),
                        )
                    })
                    .collect();
                
                Value::PatternMatch(
                    attrs,
                    Box::new(self.convert_expr(subject)),
                    morphir_cases,
                )
            }
            Expr::Constructor { name } => {
                // Build FQName for constructor
                let fqname = FQName {
                    package_path: self.package_name.clone().into(),
                    module_path: self.module_name.clone().into(),
                    local_name: Name::from(name.as_str()),
                };
                Value::Constructor(attrs, fqname)
            }
        }
    }

    /// Convert Pattern to Morphir IR Pattern
    fn convert_pattern(&self, pattern: &Pattern) -> MorphirPattern<ValueAttributes> {
        let attrs = ValueAttributes::default();
        
        match pattern {
            Pattern::Wildcard => MorphirPattern::WildcardPattern(attrs),
            Pattern::Variable { name } => {
                MorphirPattern::AsPattern(
                    attrs,
                    Box::new(MorphirPattern::WildcardPattern(attrs)),
                    Name::from(name.as_str()),
                )
            }
            Pattern::Literal { value } => {
                MorphirPattern::LiteralPattern(attrs, self.convert_literal(value))
            }
            Pattern::Constructor { name, args } => {
                let fqname = FQName {
                    package_path: self.package_name.clone().into(),
                    module_path: self.module_name.clone().into(),
                    local_name: Name::from(name.as_str()),
                };
                
                let morphir_args: Vec<MorphirPattern<ValueAttributes>> = args
                    .iter()
                    .map(|a| self.convert_pattern(a))
                    .collect();
                
                MorphirPattern::ConstructorPattern(attrs, fqname, morphir_args)
            }
            Pattern::Tuple { elements } => {
                let morphir_elements: Vec<MorphirPattern<ValueAttributes>> = elements
                    .iter()
                    .map(|e| self.convert_pattern(e))
                    .collect();
                MorphirPattern::TuplePattern(attrs, morphir_elements)
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
    use morphir_common::vfs::MemoryVfs;
    use morphir_ir::naming::{ModuleName, PackageName};
    use std::path::PathBuf;
    use crate::frontend::parser::{ModuleIR, ValueDef, TypeExpr, Expr, Literal, Access};

    #[test]
    fn test_visit_simple_module() {
        let vfs = MemoryVfs::new();
        let output_dir = PathBuf::from("/test");
        let package_name = PackageName::parse("test-package");
        let module_name = ModuleName::parse("test_module");

        let visitor = GleamToMorphirVisitor::new(
            vfs,
            output_dir,
            package_name,
            module_name,
        );

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
