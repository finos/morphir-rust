//! Transform visitors for Morphir IR.
//!
//! This module provides traits for transforming IR structures between different
//! attribute types. Unlike the read-only Visitor, TransformVisitor produces new
//! IR with potentially different attribute types.
//!
//! # Example
//!
//! ```ignore
//! // Convert Classic (serde_json::Value attrs) to V4 (TypeAttributes)
//! struct ClassicToV4;
//!
//! impl TypeTransformVisitor<serde_json::Value, TypeAttributes> for ClassicToV4 {
//!     type Error = ConversionError;
//!
//!     fn transform_attrs(&self, attrs: &serde_json::Value) -> Result<TypeAttributes, Self::Error> {
//!         Ok(TypeAttributes::default())
//!     }
//! }
//! ```

use crate::ir::pattern::Pattern;
use crate::ir::type_def::{AccessControlled, Constructor, ConstructorArg, TypeDefinition};
use crate::ir::type_expr::{Field, Type};
use crate::ir::value_expr::{
    HoleReason, InputType, LetBinding, NativeInfo, PatternCase, RecordFieldEntry, Value, ValueBody,
    ValueDefinition,
};
use crate::naming::FQName;

// =============================================================================
// Type Transform Visitor
// =============================================================================

/// Trait for transforming Type<AI> to Type<AO>.
///
/// Implementors define how to transform attributes, and the walker
/// handles recursive transformation of nested types.
pub trait TypeTransformVisitor<AI: Clone, AO: Clone> {
    /// Error type for transformation failures
    type Error;

    /// Transform attributes from input type to output type.
    /// This is the primary method implementors must define.
    fn transform_type_attrs(&self, attrs: &AI) -> Result<AO, Self::Error>;

    /// Transform a complete type. Uses the default walker implementation.
    fn transform_type(&self, tpe: &Type<AI>) -> Result<Type<AO>, Self::Error> {
        walk_transform_type(self, tpe)
    }

    /// Transform a field in a record type.
    fn transform_field(&self, field: &Field<AI>) -> Result<Field<AO>, Self::Error> {
        Ok(Field {
            name: field.name.clone(),
            tpe: self.transform_type(&field.tpe)?,
        })
    }
}

/// Walk and transform a type, recursively transforming nested types.
pub fn walk_transform_type<AI: Clone, AO: Clone, V>(
    visitor: &V,
    tpe: &Type<AI>,
) -> Result<Type<AO>, V::Error>
where
    V: TypeTransformVisitor<AI, AO> + ?Sized,
{
    match tpe {
        Type::Variable(attrs, name) => Ok(Type::Variable(
            visitor.transform_type_attrs(attrs)?,
            name.clone(),
        )),
        Type::Reference(attrs, fqname, params) => {
            let new_params: Result<Vec<_>, _> =
                params.iter().map(|p| visitor.transform_type(p)).collect();
            Ok(Type::Reference(
                visitor.transform_type_attrs(attrs)?,
                fqname.clone(),
                new_params?,
            ))
        }
        Type::Tuple(attrs, elements) => {
            let new_elements: Result<Vec<_>, _> =
                elements.iter().map(|e| visitor.transform_type(e)).collect();
            Ok(Type::Tuple(
                visitor.transform_type_attrs(attrs)?,
                new_elements?,
            ))
        }
        Type::Record(attrs, fields) => {
            let new_fields: Result<Vec<_>, _> =
                fields.iter().map(|f| visitor.transform_field(f)).collect();
            Ok(Type::Record(
                visitor.transform_type_attrs(attrs)?,
                new_fields?,
            ))
        }
        Type::ExtensibleRecord(attrs, var, fields) => {
            let new_fields: Result<Vec<_>, _> =
                fields.iter().map(|f| visitor.transform_field(f)).collect();
            Ok(Type::ExtensibleRecord(
                visitor.transform_type_attrs(attrs)?,
                var.clone(),
                new_fields?,
            ))
        }
        Type::Function(attrs, arg, result) => Ok(Type::Function(
            visitor.transform_type_attrs(attrs)?,
            Box::new(visitor.transform_type(arg)?),
            Box::new(visitor.transform_type(result)?),
        )),
        Type::Unit(attrs) => Ok(Type::Unit(visitor.transform_type_attrs(attrs)?)),
    }
}

// =============================================================================
// Pattern Transform Visitor
// =============================================================================

/// Trait for transforming Pattern<AI> to Pattern<AO>.
pub trait PatternTransformVisitor<AI: Clone, AO: Clone> {
    /// Error type for transformation failures
    type Error;

    /// Transform pattern attributes from input type to output type.
    fn transform_pattern_attrs(&self, attrs: &AI) -> Result<AO, Self::Error>;

    /// Transform a complete pattern. Uses the default walker implementation.
    fn transform_pattern(&self, pattern: &Pattern<AI>) -> Result<Pattern<AO>, Self::Error> {
        walk_transform_pattern(self, pattern)
    }
}

/// Walk and transform a pattern, recursively transforming nested patterns.
pub fn walk_transform_pattern<AI: Clone, AO: Clone, V>(
    visitor: &V,
    pattern: &Pattern<AI>,
) -> Result<Pattern<AO>, V::Error>
where
    V: PatternTransformVisitor<AI, AO> + ?Sized,
{
    match pattern {
        Pattern::WildcardPattern(attrs) => Ok(Pattern::WildcardPattern(
            visitor.transform_pattern_attrs(attrs)?,
        )),
        Pattern::AsPattern(attrs, inner, name) => Ok(Pattern::AsPattern(
            visitor.transform_pattern_attrs(attrs)?,
            Box::new(visitor.transform_pattern(inner)?),
            name.clone(),
        )),
        Pattern::TuplePattern(attrs, elements) => {
            let new_elements: Result<Vec<_>, _> = elements
                .iter()
                .map(|e| visitor.transform_pattern(e))
                .collect();
            Ok(Pattern::TuplePattern(
                visitor.transform_pattern_attrs(attrs)?,
                new_elements?,
            ))
        }
        Pattern::ConstructorPattern(attrs, name, args) => {
            let new_args: Result<Vec<_>, _> =
                args.iter().map(|a| visitor.transform_pattern(a)).collect();
            Ok(Pattern::ConstructorPattern(
                visitor.transform_pattern_attrs(attrs)?,
                name.clone(),
                new_args?,
            ))
        }
        Pattern::EmptyListPattern(attrs) => Ok(Pattern::EmptyListPattern(
            visitor.transform_pattern_attrs(attrs)?,
        )),
        Pattern::HeadTailPattern(attrs, head, tail) => Ok(Pattern::HeadTailPattern(
            visitor.transform_pattern_attrs(attrs)?,
            Box::new(visitor.transform_pattern(head)?),
            Box::new(visitor.transform_pattern(tail)?),
        )),
        Pattern::LiteralPattern(attrs, lit) => Ok(Pattern::LiteralPattern(
            visitor.transform_pattern_attrs(attrs)?,
            lit.clone(),
        )),
        Pattern::UnitPattern(attrs) => Ok(Pattern::UnitPattern(
            visitor.transform_pattern_attrs(attrs)?,
        )),
    }
}

// =============================================================================
// Value Transform Visitor
// =============================================================================

/// Trait for transforming Value<TAI, VAI> to Value<TAO, VAO>.
///
/// This is the most complex transform visitor as Values contain both
/// type attributes and value attributes.
pub trait ValueTransformVisitor<TAI: Clone, TAO: Clone, VAI: Clone, VAO: Clone> {
    /// Error type for transformation failures
    type Error;

    /// Transform type attributes from input type to output type.
    fn transform_type_attrs(&self, attrs: &TAI) -> Result<TAO, Self::Error>;

    /// Transform value attributes from input type to output type.
    fn transform_value_attrs(&self, attrs: &VAI) -> Result<VAO, Self::Error>;

    /// Transform pattern attributes (same as value attributes).
    fn transform_pattern_attrs(&self, attrs: &VAI) -> Result<VAO, Self::Error> {
        self.transform_value_attrs(attrs)
    }

    /// Transform a type.
    fn transform_type(&self, tpe: &Type<TAI>) -> Result<Type<TAO>, Self::Error> {
        // Inline the type transformation logic
        match tpe {
            Type::Variable(attrs, name) => Ok(Type::Variable(
                self.transform_type_attrs(attrs)?,
                name.clone(),
            )),
            Type::Reference(attrs, fqname, params) => {
                let new_params: Result<Vec<_>, _> =
                    params.iter().map(|p| self.transform_type(p)).collect();
                Ok(Type::Reference(
                    self.transform_type_attrs(attrs)?,
                    fqname.clone(),
                    new_params?,
                ))
            }
            Type::Tuple(attrs, elements) => {
                let new_elements: Result<Vec<_>, _> =
                    elements.iter().map(|e| self.transform_type(e)).collect();
                Ok(Type::Tuple(
                    self.transform_type_attrs(attrs)?,
                    new_elements?,
                ))
            }
            Type::Record(attrs, fields) => {
                let new_fields: Result<Vec<_>, _> = fields
                    .iter()
                    .map(|f| {
                        Ok(Field {
                            name: f.name.clone(),
                            tpe: self.transform_type(&f.tpe)?,
                        })
                    })
                    .collect();
                Ok(Type::Record(self.transform_type_attrs(attrs)?, new_fields?))
            }
            Type::ExtensibleRecord(attrs, var, fields) => {
                let new_fields: Result<Vec<_>, _> = fields
                    .iter()
                    .map(|f| {
                        Ok(Field {
                            name: f.name.clone(),
                            tpe: self.transform_type(&f.tpe)?,
                        })
                    })
                    .collect();
                Ok(Type::ExtensibleRecord(
                    self.transform_type_attrs(attrs)?,
                    var.clone(),
                    new_fields?,
                ))
            }
            Type::Function(attrs, arg, result) => Ok(Type::Function(
                self.transform_type_attrs(attrs)?,
                Box::new(self.transform_type(arg)?),
                Box::new(self.transform_type(result)?),
            )),
            Type::Unit(attrs) => Ok(Type::Unit(self.transform_type_attrs(attrs)?)),
        }
    }

    /// Transform a pattern.
    fn transform_pattern(&self, pattern: &Pattern<VAI>) -> Result<Pattern<VAO>, Self::Error> {
        match pattern {
            Pattern::WildcardPattern(attrs) => Ok(Pattern::WildcardPattern(
                self.transform_pattern_attrs(attrs)?,
            )),
            Pattern::AsPattern(attrs, inner, name) => Ok(Pattern::AsPattern(
                self.transform_pattern_attrs(attrs)?,
                Box::new(self.transform_pattern(inner)?),
                name.clone(),
            )),
            Pattern::TuplePattern(attrs, elements) => {
                let new_elements: Result<Vec<_>, _> =
                    elements.iter().map(|e| self.transform_pattern(e)).collect();
                Ok(Pattern::TuplePattern(
                    self.transform_pattern_attrs(attrs)?,
                    new_elements?,
                ))
            }
            Pattern::ConstructorPattern(attrs, name, args) => {
                let new_args: Result<Vec<_>, _> =
                    args.iter().map(|a| self.transform_pattern(a)).collect();
                Ok(Pattern::ConstructorPattern(
                    self.transform_pattern_attrs(attrs)?,
                    name.clone(),
                    new_args?,
                ))
            }
            Pattern::EmptyListPattern(attrs) => Ok(Pattern::EmptyListPattern(
                self.transform_pattern_attrs(attrs)?,
            )),
            Pattern::HeadTailPattern(attrs, head, tail) => Ok(Pattern::HeadTailPattern(
                self.transform_pattern_attrs(attrs)?,
                Box::new(self.transform_pattern(head)?),
                Box::new(self.transform_pattern(tail)?),
            )),
            Pattern::LiteralPattern(attrs, lit) => Ok(Pattern::LiteralPattern(
                self.transform_pattern_attrs(attrs)?,
                lit.clone(),
            )),
            Pattern::UnitPattern(attrs) => {
                Ok(Pattern::UnitPattern(self.transform_pattern_attrs(attrs)?))
            }
        }
    }

    /// Transform a complete value. Uses the default walker implementation.
    fn transform_value(&self, value: &Value<TAI, VAI>) -> Result<Value<TAO, VAO>, Self::Error> {
        walk_transform_value(self, value)
    }

    /// Transform a value definition.
    fn transform_value_definition(
        &self,
        def: &ValueDefinition<TAI, VAI>,
    ) -> Result<ValueDefinition<TAO, VAO>, Self::Error> {
        walk_transform_value_definition(self, def)
    }

    /// Handle V4-only Hole values during transformation.
    /// Override to customize handling (e.g., error on downgrade).
    fn transform_hole(
        &self,
        attrs: &VAI,
        reason: &HoleReason,
        expected_type: &Option<Box<Type<TAI>>>,
    ) -> Result<Value<TAO, VAO>, Self::Error> {
        let new_expected = match expected_type {
            Some(t) => Some(Box::new(self.transform_type(t)?)),
            None => None,
        };
        Ok(Value::Hole(
            self.transform_value_attrs(attrs)?,
            reason.clone(),
            new_expected,
        ))
    }

    /// Handle V4-only Native values during transformation.
    /// Override to customize handling (e.g., error on downgrade).
    fn transform_native(
        &self,
        attrs: &VAI,
        fqname: &FQName,
        info: &NativeInfo,
    ) -> Result<Value<TAO, VAO>, Self::Error> {
        Ok(Value::Native(
            self.transform_value_attrs(attrs)?,
            fqname.clone(),
            info.clone(),
        ))
    }

    /// Handle V4-only External values during transformation.
    /// Override to customize handling (e.g., error on downgrade).
    fn transform_external(
        &self,
        attrs: &VAI,
        external_name: &str,
        target_platform: &str,
    ) -> Result<Value<TAO, VAO>, Self::Error> {
        Ok(Value::External(
            self.transform_value_attrs(attrs)?,
            external_name.to_string(),
            target_platform.to_string(),
        ))
    }
}

/// Walk and transform a value, recursively transforming nested values.
pub fn walk_transform_value<TAI: Clone, TAO: Clone, VAI: Clone, VAO: Clone, V>(
    visitor: &V,
    value: &Value<TAI, VAI>,
) -> Result<Value<TAO, VAO>, V::Error>
where
    V: ValueTransformVisitor<TAI, TAO, VAI, VAO> + ?Sized,
{
    match value {
        Value::Literal(attrs, lit) => Ok(Value::Literal(
            visitor.transform_value_attrs(attrs)?,
            lit.clone(),
        )),
        Value::Constructor(attrs, name) => Ok(Value::Constructor(
            visitor.transform_value_attrs(attrs)?,
            name.clone(),
        )),
        Value::Tuple(attrs, elements) => {
            let new_elements: Result<Vec<_>, _> = elements
                .iter()
                .map(|e| visitor.transform_value(e))
                .collect();
            Ok(Value::Tuple(
                visitor.transform_value_attrs(attrs)?,
                new_elements?,
            ))
        }
        Value::List(attrs, elements) => {
            let new_elements: Result<Vec<_>, _> = elements
                .iter()
                .map(|e| visitor.transform_value(e))
                .collect();
            Ok(Value::List(
                visitor.transform_value_attrs(attrs)?,
                new_elements?,
            ))
        }
        Value::Record(attrs, fields) => {
            let new_fields: Result<Vec<_>, _> = fields
                .iter()
                .map(|entry| {
                    Ok(RecordFieldEntry::new(
                        entry.name().clone(),
                        visitor.transform_value(entry.value())?,
                    ))
                })
                .collect();
            Ok(Value::Record(
                visitor.transform_value_attrs(attrs)?,
                new_fields?,
            ))
        }
        Value::Variable(attrs, name) => Ok(Value::Variable(
            visitor.transform_value_attrs(attrs)?,
            name.clone(),
        )),
        Value::Reference(attrs, fqname) => Ok(Value::Reference(
            visitor.transform_value_attrs(attrs)?,
            fqname.clone(),
        )),
        Value::Field(attrs, record, field_name) => Ok(Value::Field(
            visitor.transform_value_attrs(attrs)?,
            Box::new(visitor.transform_value(record)?),
            field_name.clone(),
        )),
        Value::FieldFunction(attrs, name) => Ok(Value::FieldFunction(
            visitor.transform_value_attrs(attrs)?,
            name.clone(),
        )),
        Value::Apply(attrs, func, arg) => Ok(Value::Apply(
            visitor.transform_value_attrs(attrs)?,
            Box::new(visitor.transform_value(func)?),
            Box::new(visitor.transform_value(arg)?),
        )),
        Value::Lambda(attrs, pattern, body) => Ok(Value::Lambda(
            visitor.transform_value_attrs(attrs)?,
            visitor.transform_pattern(pattern)?,
            Box::new(visitor.transform_value(body)?),
        )),
        Value::LetDefinition(attrs, name, def, body) => Ok(Value::LetDefinition(
            visitor.transform_value_attrs(attrs)?,
            name.clone(),
            Box::new(visitor.transform_value_definition(def)?),
            Box::new(visitor.transform_value(body)?),
        )),
        Value::LetRecursion(attrs, defs, body) => {
            let new_defs: Result<Vec<_>, _> = defs
                .iter()
                .map(|binding| {
                    Ok(LetBinding::new(
                        binding.name().clone(),
                        visitor.transform_value_definition(binding.definition())?,
                    ))
                })
                .collect();
            Ok(Value::LetRecursion(
                visitor.transform_value_attrs(attrs)?,
                new_defs?,
                Box::new(visitor.transform_value(body)?),
            ))
        }
        Value::Destructure(attrs, pattern, val, body) => Ok(Value::Destructure(
            visitor.transform_value_attrs(attrs)?,
            visitor.transform_pattern(pattern)?,
            Box::new(visitor.transform_value(val)?),
            Box::new(visitor.transform_value(body)?),
        )),
        Value::IfThenElse(attrs, cond, then_branch, else_branch) => Ok(Value::IfThenElse(
            visitor.transform_value_attrs(attrs)?,
            Box::new(visitor.transform_value(cond)?),
            Box::new(visitor.transform_value(then_branch)?),
            Box::new(visitor.transform_value(else_branch)?),
        )),
        Value::PatternMatch(attrs, input, cases) => {
            let new_cases: Result<Vec<_>, _> = cases
                .iter()
                .map(|case| {
                    Ok(PatternCase::new(
                        visitor.transform_pattern(case.pattern())?,
                        visitor.transform_value(case.body())?,
                    ))
                })
                .collect();
            Ok(Value::PatternMatch(
                visitor.transform_value_attrs(attrs)?,
                Box::new(visitor.transform_value(input)?),
                new_cases?,
            ))
        }
        Value::UpdateRecord(attrs, record, updates) => {
            let new_updates: Result<Vec<_>, _> = updates
                .iter()
                .map(|entry| {
                    Ok(RecordFieldEntry::new(
                        entry.name().clone(),
                        visitor.transform_value(entry.value())?,
                    ))
                })
                .collect();
            Ok(Value::UpdateRecord(
                visitor.transform_value_attrs(attrs)?,
                Box::new(visitor.transform_value(record)?),
                new_updates?,
            ))
        }
        Value::Unit(attrs) => Ok(Value::Unit(visitor.transform_value_attrs(attrs)?)),
        // V4-only variants - delegate to visitor methods
        Value::Hole(attrs, reason, expected_type) => {
            visitor.transform_hole(attrs, reason, expected_type)
        }
        Value::Native(attrs, fqname, info) => visitor.transform_native(attrs, fqname, info),
        Value::External(attrs, external_name, target_platform) => {
            visitor.transform_external(attrs, external_name, target_platform)
        }
    }
}

/// Walk and transform a value definition.
pub fn walk_transform_value_definition<TAI: Clone, TAO: Clone, VAI: Clone, VAO: Clone, V>(
    visitor: &V,
    def: &ValueDefinition<TAI, VAI>,
) -> Result<ValueDefinition<TAO, VAO>, V::Error>
where
    V: ValueTransformVisitor<TAI, TAO, VAI, VAO> + ?Sized,
{
    let new_input_types: Result<Vec<_>, _> = def
        .input_types
        .iter()
        .map(|input| {
            Ok(InputType::new(
                input.name().clone(),
                visitor.transform_value_attrs(input.attrs())?,
                visitor.transform_type(input.tpe())?,
            ))
        })
        .collect();

    let new_body = match &def.body {
        ValueBody::Expression(val) => ValueBody::Expression(visitor.transform_value(val)?),
        ValueBody::Native(info) => ValueBody::Native(info.clone()),
        ValueBody::External {
            external_name,
            target_platform,
        } => ValueBody::External {
            external_name: external_name.clone(),
            target_platform: target_platform.clone(),
        },
        ValueBody::Incomplete(reason) => ValueBody::Incomplete(reason.clone()),
    };

    Ok(ValueDefinition {
        input_types: new_input_types?,
        output_type: visitor.transform_type(&def.output_type)?,
        body: new_body,
    })
}

// =============================================================================
// Type Definition Transform
// =============================================================================

/// Transform a type definition using a TypeTransformVisitor.
pub fn transform_type_definition<AI: Clone, AO: Clone, V>(
    visitor: &V,
    def: &TypeDefinition<AI>,
) -> Result<TypeDefinition<AO>, V::Error>
where
    V: TypeTransformVisitor<AI, AO> + ?Sized,
{
    match def {
        TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr,
        } => Ok(TypeDefinition::TypeAliasDefinition {
            type_params: type_params.clone(),
            type_expr: visitor.transform_type(type_expr)?,
        }),
        TypeDefinition::CustomTypeDefinition {
            type_params,
            access_controlled_ctors,
        } => {
            let transform_ctors =
                |ctors: &Vec<Constructor<AI>>| -> Result<Vec<Constructor<AO>>, V::Error> {
                    ctors
                        .iter()
                        .map(|ctor| {
                            let new_args: Result<Vec<_>, _> = ctor
                                .args
                                .iter()
                                .map(|arg| {
                                    Ok(ConstructorArg::new(
                                        arg.name().clone(),
                                        visitor.transform_type(arg.tpe())?,
                                    ))
                                })
                                .collect();
                            Ok(Constructor {
                                name: ctor.name.clone(),
                                args: new_args?,
                            })
                        })
                        .collect()
                };

            let new_ctors = match access_controlled_ctors {
                AccessControlled::Public(ctors) => {
                    AccessControlled::Public(transform_ctors(ctors)?)
                }
                AccessControlled::Private(ctors) => {
                    AccessControlled::Private(transform_ctors(ctors)?)
                }
            };

            Ok(TypeDefinition::CustomTypeDefinition {
                type_params: type_params.clone(),
                access_controlled_ctors: new_ctors,
            })
        }
        TypeDefinition::IncompleteTypeDefinition {
            type_params,
            incompleteness,
        } => Ok(TypeDefinition::IncompleteTypeDefinition {
            type_params: type_params.clone(),
            incompleteness: incompleteness.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::naming::Name;

    // Test visitor that transforms () attributes to i32
    struct TestTypeTransformer;

    impl TypeTransformVisitor<(), i32> for TestTypeTransformer {
        type Error = std::convert::Infallible;

        fn transform_type_attrs(&self, _attrs: &()) -> Result<i32, Self::Error> {
            Ok(42)
        }
    }

    #[test]
    fn test_transform_variable() {
        let var: Type<()> = Type::Variable((), Name::from("a"));
        let transformer = TestTypeTransformer;
        let result = transformer.transform_type(&var).unwrap();
        assert!(matches!(result, Type::Variable(42, _)));
    }

    #[test]
    fn test_transform_unit() {
        let unit: Type<()> = Type::Unit(());
        let transformer = TestTypeTransformer;
        let result = transformer.transform_type(&unit).unwrap();
        assert!(matches!(result, Type::Unit(42)));
    }

    #[test]
    fn test_transform_function() {
        let func: Type<()> = Type::Function(
            (),
            Box::new(Type::Unit(())),
            Box::new(Type::Variable((), Name::from("a"))),
        );
        let transformer = TestTypeTransformer;
        let result = transformer.transform_type(&func).unwrap();

        if let Type::Function(attrs, arg, ret) = result {
            assert_eq!(attrs, 42);
            assert!(matches!(*arg, Type::Unit(42)));
            assert!(matches!(*ret, Type::Variable(42, _)));
        } else {
            panic!("Expected Function type");
        }
    }

    // Test pattern transformer
    struct TestPatternTransformer;

    impl PatternTransformVisitor<(), String> for TestPatternTransformer {
        type Error = std::convert::Infallible;

        fn transform_pattern_attrs(&self, _attrs: &()) -> Result<String, Self::Error> {
            Ok("transformed".to_string())
        }
    }

    #[test]
    fn test_transform_wildcard_pattern() {
        let pattern: Pattern<()> = Pattern::WildcardPattern(());
        let transformer = TestPatternTransformer;
        let result = transformer.transform_pattern(&pattern).unwrap();
        assert!(matches!(result, Pattern::WildcardPattern(s) if s == "transformed"));
    }

    #[test]
    fn test_transform_tuple_pattern() {
        let pattern: Pattern<()> = Pattern::TuplePattern(
            (),
            vec![Pattern::WildcardPattern(()), Pattern::UnitPattern(())],
        );
        let transformer = TestPatternTransformer;
        let result = transformer.transform_pattern(&pattern).unwrap();

        if let Pattern::TuplePattern(attrs, elements) = result {
            assert_eq!(attrs, "transformed");
            assert_eq!(elements.len(), 2);
        } else {
            panic!("Expected TuplePattern");
        }
    }

    // Test value transformer
    struct TestValueTransformer;

    impl ValueTransformVisitor<(), i32, (), String> for TestValueTransformer {
        type Error = std::convert::Infallible;

        fn transform_type_attrs(&self, _attrs: &()) -> Result<i32, Self::Error> {
            Ok(42)
        }

        fn transform_value_attrs(&self, _attrs: &()) -> Result<String, Self::Error> {
            Ok("value".to_string())
        }
    }

    #[test]
    fn test_transform_value_unit() {
        let val: Value<(), ()> = Value::Unit(());
        let transformer = TestValueTransformer;
        let result = transformer.transform_value(&val).unwrap();
        assert!(matches!(result, Value::Unit(s) if s == "value"));
    }

    #[test]
    fn test_transform_value_tuple() {
        let val: Value<(), ()> = Value::Tuple((), vec![Value::Unit(()), Value::Unit(())]);
        let transformer = TestValueTransformer;
        let result = transformer.transform_value(&val).unwrap();

        if let Value::Tuple(attrs, elements) = result {
            assert_eq!(attrs, "value");
            assert_eq!(elements.len(), 2);
        } else {
            panic!("Expected Tuple value");
        }
    }
}
