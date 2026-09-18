//! The version-neutral resolved model.
//!
//! Resolution turns the syntactic [`crate::ast`] into a model where every type
//! reference is a fully qualified name. Nothing here depends on a Morphir IR
//! version, so both the v3 and v4 backends lower from the same model, and the
//! interface digest computed here is stable across IR versions.

use serde::Serialize;

use crate::digest::sha256_hex;
use crate::names;
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

/// A publicly exposed type, with everything a dependent can observe about it.
///
/// The *specification*, not just the name: an alias publishes the type it
/// stands for and a custom type publishes its constructors' argument types, so
/// that retyping either is an interface change its dependents are recompiled
/// for. `alias` and `constructors` are never both `Some`; both are `None` for a
/// custom type whose constructors are private, which is opaque to a dependent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InterfaceType {
    pub name: String,
    pub params: Vec<String>,
    /// The type this alias stands for, when the declaration is an alias.
    pub alias: Option<RType>,
    /// The public constructors, each with its argument types.
    pub constructors: Option<Vec<(String, Vec<RType>)>>,
}

impl ResolvedModule {
    /// The module's public interface: the specification of every public type,
    /// sorted by name, with constructors recorded only when they are public.
    ///
    /// Docs, private types and private constructors are excluded, so the digest
    /// of this value changes only when a dependent module could observe the
    /// difference.
    ///
    /// Every identifier is written in its [`crate::names`] spelling, because
    /// this value is compared against the one
    /// [`crate::frontend::dependencies::interface_from_module_ir`] reads back
    /// out of a Morphir document, and a document keeps only a name's words. Two
    /// identifiers a document cannot tell apart are the same identifier here.
    pub fn interface(&self) -> Interface {
        let mut types: Vec<InterfaceType> = self
            .types
            .iter()
            .filter(|ty| ty.access == Access::Public)
            .map(|ty| InterfaceType {
                name: names::type_spelling(&ty.name),
                params: ty.params.iter().map(|p| names::value_spelling(p)).collect(),
                alias: match &ty.body {
                    ResolvedBody::Alias(body) => Some(spelled_type(body)),
                    ResolvedBody::Custom { .. } => None,
                },
                constructors: match &ty.body {
                    ResolvedBody::Custom {
                        constructor_access: Access::Public,
                        constructors,
                    } => Some(
                        constructors
                            .iter()
                            .map(|c| {
                                (
                                    names::type_spelling(&c.name),
                                    c.args.iter().map(spelled_type).collect(),
                                )
                            })
                            .collect(),
                    ),
                    _ => None,
                },
            })
            .collect();
        types.sort_by(|a, b| a.name.cmp(&b.name));
        Interface {
            name: self.name.iter().map(|s| names::type_spelling(s)).collect(),
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

/// A resolved type with every identifier in its document spelling.
fn spelled_type(ty: &RType) -> RType {
    match ty {
        RType::Var(variable) => RType::Var(names::value_spelling(variable)),
        RType::Ref(reference, arguments) => RType::Ref(
            spelled_fqname(reference),
            arguments.iter().map(spelled_type).collect(),
        ),
        RType::Record(fields) => RType::Record(fields.iter().map(spelled_field).collect()),
        RType::ExtensibleRecord(variable, fields) => RType::ExtensibleRecord(
            names::value_spelling(variable),
            fields.iter().map(spelled_field).collect(),
        ),
        RType::Tuple(elements) => RType::Tuple(elements.iter().map(spelled_type).collect()),
        RType::Function(argument, result) => RType::Function(
            Box::new(spelled_type(argument)),
            Box::new(spelled_type(result)),
        ),
        RType::Unit => RType::Unit,
    }
}

fn spelled_field(field: &RField) -> RField {
    RField {
        name: names::value_spelling(&field.name),
        ty: spelled_type(&field.ty),
    }
}

fn spelled_fqname(reference: &FqName) -> FqName {
    FqName {
        package: reference
            .package
            .iter()
            .map(|segment| names::type_spelling(segment))
            .collect(),
        module: reference
            .module
            .iter()
            .map(|segment| names::type_spelling(segment))
            .collect(),
        name: names::type_spelling(&reference.name),
    }
}
