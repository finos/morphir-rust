//! Reject syntax and name uses that the value lowerer cannot preserve.

use crate::frontend::ast::{Expr, Field, ModuleIR, Pattern, Span, Statement, TypeExpr};
use std::collections::BTreeSet;
use std::io::{Error, ErrorKind, Result};

/// A valid source expression that cannot yet be lowered without changing meaning.
#[derive(Debug)]
pub(crate) struct UnsupportedValue {
    pub(crate) feature: String,
    pub(crate) span: Span,
}
impl std::fmt::Display for UnsupportedValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "Unsupported Gleam value syntax: {}",
            self.feature
        )
    }
}
impl std::error::Error for UnsupportedValue {}

pub(super) fn unsupported(feature: impl Into<String>, span: Span) -> Error {
    Error::new(
        ErrorKind::Unsupported,
        UnsupportedValue {
            feature: feature.into(),
            span,
        },
    )
}

pub(super) fn validate_values(module: &ModuleIR) -> Result<()> {
    let imported = module
        .imports
        .iter()
        .flat_map(|import| import.values.iter().map(|(_, alias)| alias.as_str()))
        .collect::<BTreeSet<_>>();
    let qualifiers = module
        .imports
        .iter()
        .map(|import| {
            import
                .alias
                .as_deref()
                .unwrap_or_else(|| import.module.rsplit('/').next().unwrap_or(&import.module))
        })
        .collect::<BTreeSet<_>>();
    let mut local = module
        .values
        .iter()
        .map(|value| value.name.clone())
        .collect::<BTreeSet<_>>();
    for definition in &module.types {
        if let TypeExpr::CustomType { variants } = &definition.body {
            local.extend(variants.iter().map(|variant| variant.name.clone()));
        }
    }
    for value in &module.values {
        if value.param_labels.iter().any(Option::is_some) {
            return Err(unsupported(
                "labelled function parameters require external label preservation",
                value.span,
            ));
        }
        let mut bound = local.clone();
        bound.extend(value.params.iter().cloned());
        Scope {
            imported: &imported,
            qualifiers: &qualifiers,
            bound,
            span: value.span,
        }
        .expression(&value.body)?;
    }
    Ok(())
}

#[derive(Clone)]
struct Scope<'a> {
    imported: &'a BTreeSet<&'a str>,
    qualifiers: &'a BTreeSet<&'a str>,
    bound: BTreeSet<String>,
    span: Span,
}
impl Scope<'_> {
    fn reject<T>(&self, feature: impl Into<String>) -> Result<T> {
        Err(unsupported(feature, self.span))
    }
    fn value_name(&self, name: &str) -> Result<()> {
        if self.imported.contains(name) && !self.bound.contains(name) {
            self.reject(format!(
                "imported value '{name}' requires value-name resolution"
            ))
        } else {
            Ok(())
        }
    }
    fn qualifier(&self, name: &str) -> Result<()> {
        if self.qualifiers.contains(name) {
            self.reject(format!(
                "imported module value '{name}' requires value-name resolution"
            ))
        } else {
            Ok(())
        }
    }
    fn expression(&self, value: &Expr) -> Result<()> {
        match value {
            Expr::Unsupported { feature, span } => Err(unsupported(feature.clone(), *span)),
            Expr::Literal { .. } => Ok(()),
            Expr::Variable { name } => self.value_name(name),
            Expr::Apply {
                function,
                arguments,
            } => {
                self.expression(function)?;
                for argument in arguments {
                    match argument {
                        Field::Unlabelled { item } => self.expression(item)?,
                        Field::Labelled { .. } | Field::Shorthand { .. } => {
                            return self.reject(
                                "labelled call arguments require signature-based ordering",
                            );
                        }
                    }
                }
                Ok(())
            }
            Expr::Lambda { params, body } => {
                let mut inner = self.clone();
                inner.bound.extend(params.iter().cloned());
                inner.expression(body)
            }
            Expr::Let { name, value, body } => {
                self.expression(value)?;
                let mut inner = self.clone();
                inner.bound.insert(name.clone());
                inner.expression(body)
            }
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expression(condition)?;
                self.expression(then_branch)?;
                self.expression(else_branch)
            }
            Expr::Record { fields } => {
                for (_, value) in fields {
                    self.expression(value)?;
                }
                Ok(())
            }
            Expr::FieldAccess { container, .. } => {
                if let Expr::Variable { name } = container.as_ref() {
                    self.qualifier(name)?;
                }
                self.expression(container)
            }
            Expr::Tuple { elements }
            | Expr::List {
                elements,
                tail: None,
            } => {
                for value in elements {
                    self.expression(value)?;
                }
                Ok(())
            }
            Expr::List {
                elements,
                tail: Some(tail),
            } => {
                for value in elements {
                    self.expression(value)?;
                }
                self.expression(tail)
            }
            Expr::TupleIndex { tuple, .. } => self.expression(tuple),
            Expr::Case { subjects, clauses } => {
                if subjects.len() != 1 {
                    return self.reject("case expressions require exactly one subject");
                }
                for value in subjects {
                    self.expression(value)?;
                }
                for clause in clauses {
                    let mut inner = self.clone();
                    inner.pattern(&clause.pattern)?;
                    inner.expression(&clause.body)?;
                }
                Ok(())
            }
            Expr::Constructor { module, name } => {
                if let Some(module) = module {
                    return self.reject(format!(
                        "qualified constructor {module}.{name} requires value-name resolution"
                    ));
                }
                self.value_name(name)
            }
            Expr::BinaryOp { left, right, .. } => {
                self.expression(left)?;
                self.expression(right)
            }
            Expr::NegateInt { value } | Expr::NegateBool { value } => self.expression(value),
            Expr::Block { statements } => {
                let mut inner = self.clone();
                for statement in statements {
                    match statement {
                        Statement::Expression(value) => inner.expression(value)?,
                        Statement::Assignment { pattern, value, .. } => {
                            inner.expression(value)?;
                            inner.pattern(pattern)?;
                        }
                        Statement::Use { .. } => return inner.reject("use expressions"),
                    }
                }
                Ok(())
            }
            Expr::Panic { .. } => self.reject("panic expressions"),
            Expr::Todo { .. } => self.reject("todo expressions"),
            Expr::Echo { .. } => self.reject("echo expressions"),
            Expr::BitString { .. } => self.reject("bit strings"),
            Expr::FnCapture { .. } => self.reject("function captures"),
            Expr::RecordUpdate { .. } => self.reject("record updates"),
        }
    }
    fn pattern(&mut self, value: &Pattern) -> Result<()> {
        match value {
            Pattern::Variable { name } => {
                self.bound.insert(name.clone());
            }
            Pattern::Assignment { pattern, name } => {
                self.pattern(pattern)?;
                self.bound.insert(name.clone());
            }
            Pattern::Constructor {
                module,
                name,
                arguments,
                with_spread,
            } => {
                if let Some(module) = module {
                    return self.reject(format!(
                        "qualified constructor {module}.{name} requires value-name resolution"
                    ));
                }
                self.value_name(name)?;
                if *with_spread {
                    return self
                        .reject("constructor spreads require signature-based pattern ordering");
                }
                for argument in arguments {
                    match argument {
                        Field::Unlabelled { item } => self.pattern(item)?,
                        Field::Labelled { .. } | Field::Shorthand { .. } => {
                            return self.reject(
                                "labelled constructor patterns require signature-based ordering",
                            );
                        }
                    }
                }
            }
            Pattern::Tuple { elements } => {
                for pattern in elements {
                    self.pattern(pattern)?;
                }
            }
            Pattern::List { elements, tail } => {
                for pattern in elements {
                    self.pattern(pattern)?;
                }
                if let Some(tail) = tail {
                    self.pattern(tail)?;
                }
            }
            Pattern::Concatenate { .. } => return self.reject("string prefix patterns"),
            Pattern::BitString { .. } => return self.reject("bit string patterns"),
            Pattern::Wildcard | Pattern::Discard { .. } | Pattern::Literal { .. } => {}
        }
        Ok(())
    }
}
