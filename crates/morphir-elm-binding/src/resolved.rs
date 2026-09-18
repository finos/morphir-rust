//! The version-neutral resolved model.
//!
//! Resolution turns the syntactic [`crate::ast`] into a model where every type
//! reference is a fully qualified name. Nothing here depends on a Morphir IR
//! version, so both the v3 and v4 backends lower from the same model, and the
//! interface digest computed here is stable across IR versions.

use serde::Serialize;

use crate::digest::sha256_hex;
use crate::span::Span;

/// A fully qualified name: package path, module path, local name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FqName {
    pub package: Vec<String>,
    pub module: Vec<String>,
    pub name: String,
}

/// Whether a module, type, or set of constructors is exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Access {
    Public,
    Private,
}

/// A module whose type references have all been resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedModule {
    pub name: Vec<String>,
    pub access: Access,
    pub doc: Option<String>,
    pub types: Vec<ResolvedType>,
    /// In-package modules this module references, deduplicated and sorted.
    pub depends_on: Vec<Vec<String>>,
    pub skipped_values: Vec<(String, Span)>,
}

/// A resolved type declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedType {
    pub name: String,
    pub access: Access,
    pub doc: Option<String>,
    pub params: Vec<String>,
    pub body: ResolvedBody,
}

/// The body of a resolved type declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ResolvedBody {
    Alias(RType),
    Custom {
        constructor_access: Access,
        constructors: Vec<RConstructor>,
    },
}

/// A custom type constructor with resolved argument types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RConstructor {
    pub name: String,
    pub args: Vec<RType>,
}

/// A resolved type expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RType {
    Var(String),
    Ref(FqName, Vec<RType>),
    Record(Vec<RField>),
    ExtensibleRecord(String, Vec<RField>),
    Tuple(Vec<RType>),
    Function(Box<RType>, Box<RType>),
    Unit,
}

/// A record field with a resolved type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RField {
    pub name: String,
    pub ty: RType,
}

/// The public surface of a module: what other modules may refer to.
///
/// This is both the unit the resolver looks types up in (for in-package and
/// dependency modules) and the value the interface digest is computed over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Interface {
    pub name: Vec<String>,
    pub types: Vec<InterfaceType>,
}

/// A publicly exposed type. `constructors` is `Some` only when the type's
/// constructors are exposed too.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InterfaceType {
    pub name: String,
    pub params: Vec<String>,
    pub constructors: Option<Vec<String>>,
}

impl ResolvedModule {
    /// The module's public interface: public types only, sorted by name, with
    /// constructor names recorded only when the constructors are public.
    ///
    /// Docs, private types and private constructors are excluded, so the
    /// digest of this value changes only when a dependent module could
    /// observe the difference.
    pub fn interface(&self) -> Interface {
        let mut types: Vec<InterfaceType> = self
            .types
            .iter()
            .filter(|ty| ty.access == Access::Public)
            .map(|ty| InterfaceType {
                name: ty.name.clone(),
                params: ty.params.clone(),
                constructors: match &ty.body {
                    ResolvedBody::Custom {
                        constructor_access: Access::Public,
                        constructors,
                    } => Some(constructors.iter().map(|c| c.name.clone()).collect()),
                    _ => None,
                },
            })
            .collect();
        types.sort_by(|a, b| a.name.cmp(&b.name));
        Interface {
            name: self.name.clone(),
            types,
        }
    }

    /// A content digest of [`ResolvedModule::interface`], used to decide
    /// whether dependents need recompiling.
    pub fn interface_digest(&self) -> String {
        let json = serde_json::to_vec(&self.interface()).expect("Interface serializes to JSON");
        sha256_hex(&json)
    }
}
