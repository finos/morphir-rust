//! Callable signatures and lexical capture checks shared by both Rust boundaries.
mod captures;
mod cycles;
pub(crate) use captures::{copy_type, free_variables};
pub(crate) use cycles::check_cycles;
use morphir_core::{ir::v4::Type, naming::Name};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default)]
pub(crate) struct Context {
    pub patterns: crate::patterns::Context,
    pub signatures: BTreeMap<String, Signature>,
}
#[derive(Clone)]
pub(crate) struct Signature {
    pub parameters: Vec<Name>,
    pub inputs: Vec<Type>,
    pub output: Type,
}
impl Signature {
    pub fn ty(&self) -> Type {
        self.inputs
            .iter()
            .rev()
            .fold(self.output.clone(), |output, input| {
                Type::Function(
                    Default::default(),
                    Box::new(input.clone()),
                    Box::new(output),
                )
            })
    }
    pub fn substitutions(&self, actual: &Type) -> Result<BTreeMap<String, Type>, String> {
        let flexible = self
            .parameters
            .iter()
            .map(Name::to_canonical_string)
            .collect();
        let mut substitutions = BTreeMap::new();
        unify(&self.ty(), actual, &flexible, &mut substitutions)?;
        Ok(substitutions)
    }
    pub fn instantiate(&self, annotated: Option<&Type>) -> Result<Type, String> {
        let ty = self.ty();
        let Some(actual) = annotated else {
            if self.parameters.is_empty() {
                return Ok(ty);
            }
            return Err(
                "Generic function reference requires an instantiated type annotation".into(),
            );
        };
        self.substitutions(actual)?;
        Ok(actual.clone())
    }
}
/// Only variables quantified by the callee are flexible. Caller variables remain rigid.
fn unify(
    expected: &Type,
    actual: &Type,
    flexible: &BTreeSet<String>,
    substitutions: &mut BTreeMap<String, Type>,
) -> Result<(), String> {
    if let Type::Variable(_, name) = expected {
        let key = name.to_canonical_string();
        if flexible.contains(&key) {
            if let Some(previous) = substitutions.get(&key) {
                if !crate::values::same_type(previous, actual) {
                    return Err("Inconsistent generic function instantiation".into());
                }
            } else {
                substitutions.insert(key, actual.clone());
            }
            return Ok(());
        }
    }
    match (expected, actual) {
        (Type::Function(_, a, b), Type::Function(_, c, d)) => {
            unify(a, c, flexible, substitutions)?;
            unify(b, d, flexible, substitutions)
        }
        (Type::Tuple(_, a), Type::Tuple(_, b)) if a.len() == b.len() => {
            for (a, b) in a.iter().zip(b) {
                unify(a, b, flexible, substitutions)?;
            }
            Ok(())
        }
        (Type::Reference(_, an, a), Type::Reference(_, bn, b))
            if an == bn && a.len() == b.len() =>
        {
            for (a, b) in a.iter().zip(b) {
                unify(a, b, flexible, substitutions)?;
            }
            Ok(())
        }
        _ if crate::values::same_type(expected, actual) => Ok(()),
        _ => {
            Err("Function reference annotation does not instantiate its declared signature".into())
        }
    }
}
