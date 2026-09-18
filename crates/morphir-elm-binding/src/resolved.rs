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
    /// The package path.
    pub package: Vec<String>,
    /// The module path, relative to the package.
    pub module: Vec<String>,
    /// The local name.
    pub name: String,
}

/// Whether a module, type, or set of constructors is exposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Access {
    /// Visible outside its module or package.
    Public,
    /// Visible only where it is declared.
    Private,
}

/// A module whose type references have all been resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedModule {
    /// The module's name, as the frontend and the host see it.
    pub name: Vec<String>,
    /// Whether the package exposes this module.
    pub access: Access,
    /// The module's documentation comment.
    pub doc: Option<String>,
    /// The module's type declarations, in source order.
    pub types: Vec<ResolvedType>,
    /// In-package modules this module references, deduplicated and sorted.
    pub depends_on: Vec<Vec<String>>,
    /// Value declarations this frontend skipped, with where they were.
    pub skipped_values: Vec<(String, Span)>,
}

/// A resolved type declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedType {
    /// The declared name.
    pub name: String,
    /// Whether the module exposes this type.
    pub access: Access,
    /// The declaration's documentation comment.
    pub doc: Option<String>,
    /// The type parameters, in order.
    pub params: Vec<String>,
    /// What the declaration declares.
    pub body: ResolvedBody,
}

/// The body of a resolved type declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ResolvedBody {
    /// An alias for the type it stands for.
    Alias(RType),
    /// A custom type and its constructors.
    Custom {
        /// Whether the module exposes the constructors.
        constructor_access: Access,
        /// The constructors, in declaration order.
        constructors: Vec<RConstructor>,
    },
}

/// A custom type constructor with resolved argument types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RConstructor {
    /// The constructor's name.
    pub name: String,
    /// Its positional argument types.
    pub args: Vec<RType>,
}

/// A resolved type expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RType {
    /// A type variable.
    Var(String),
    /// A reference to a declared type, with its arguments.
    Ref(FqName, Vec<RType>),
    /// A record type.
    Record(Vec<RField>),
    /// An extensible record type: the base variable and the added fields.
    ExtensibleRecord(String, Vec<RField>),
    /// A tuple type.
    Tuple(Vec<RType>),
    /// A function type: argument, then result.
    Function(Box<RType>, Box<RType>),
    /// The unit type.
    Unit,
}

/// A record field with a resolved type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RField {
    /// The field's name.
    pub name: String,
    /// The field's type.
    pub ty: RType,
}

/// The public surface of a module: what other modules may refer to.
///
/// This is both the unit the resolver looks types up in (for in-package and
/// dependency modules) and the value the interface digest is computed over.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Interface {
    /// The module's name, in its document spelling.
    pub name: Vec<String>,
    /// The types the module exposes, sorted by name.
    pub types: Vec<InterfaceType>,
}

impl Interface {
    /// A content digest of this interface. Two interfaces that a dependent
    /// cannot tell apart have the same digest.
    pub fn digest(&self) -> String {
        let json = serde_json::to_vec(self).expect("Interface serializes to JSON");
        sha256_hex(&json)
    }
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
    /// The type's name.
    pub name: String,
    /// Its type parameters, in order.
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
        self.interface().digest()
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
