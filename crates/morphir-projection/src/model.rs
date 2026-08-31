use serde::Serialize;

/// Morphir distribution category retained by the projection model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DistributionKind {
    /// A Morphir library distribution.
    Library,
    /// A specification-only distribution.
    Specs,
    /// An application distribution with declared entry points.
    Application,
}

/// A body-free package view used by Avro projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectionPackage {
    /// Distribution category.
    pub kind: DistributionKind,
    /// Canonical Morphir package name.
    pub package_name: String,
    /// Dependency specifications available to projection.
    pub dependencies: Vec<ProjectionDependency>,
    /// Public modules owned by this package.
    pub modules: Vec<ProjectionModule>,
}

/// A one-level package specification supplied as a distribution dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectionDependency {
    /// Canonical dependency package name.
    pub package_name: String,
    /// Public dependency modules.
    pub modules: Vec<ProjectionModule>,
}

/// A public Morphir module after access filtering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectionModule {
    /// Canonical module path components.
    pub path: Vec<String>,
    /// Public type declarations in canonical order.
    pub types: Vec<TypeDeclaration>,
    /// Public value specifications in canonical order.
    pub values: Vec<ValueSpecification>,
    /// Optional source documentation.
    pub doc: Option<String>,
}

/// A public type declaration or specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "declaration", rename_all = "kebab-case")]
pub enum TypeDeclaration {
    /// A transparent type alias.
    Alias {
        /// Exact canonical Morphir FQName.
        source_name: String,
        /// Canonical local Morphir name.
        name: String,
        /// Declared type parameters.
        type_params: Vec<String>,
        /// Alias target type.
        value: TypeExpr,
        /// Optional source documentation.
        doc: Option<String>,
    },
    /// An opaque type specification without a visible representation.
    Opaque {
        /// Exact canonical Morphir FQName.
        source_name: String,
        /// Canonical local Morphir name.
        name: String,
        /// Declared type parameters.
        type_params: Vec<String>,
        /// Optional source documentation.
        doc: Option<String>,
    },
    /// An algebraic custom type.
    Custom {
        /// Exact canonical Morphir FQName.
        source_name: String,
        /// Canonical local Morphir name.
        name: String,
        /// Declared type parameters.
        type_params: Vec<String>,
        /// Public constructors.
        constructors: Vec<Constructor>,
        /// Optional source documentation.
        doc: Option<String>,
    },
    /// A v4 declaration whose definition is incomplete.
    Incomplete {
        /// Exact canonical Morphir FQName.
        source_name: String,
        /// Canonical local Morphir name.
        name: String,
        /// Declared type parameters.
        type_params: Vec<String>,
        /// Kind of incompleteness reported by the IR.
        incompleteness: IncompletenessKind,
        /// Partial type information, when available.
        partial_type: Option<TypeExpr>,
        /// Optional source documentation.
        doc: Option<String>,
    },
}

impl TypeDeclaration {
    /// Canonical local Morphir name of this declaration.
    pub fn name(&self) -> &str {
        match self {
            Self::Alias { name, .. }
            | Self::Opaque { name, .. }
            | Self::Custom { name, .. }
            | Self::Incomplete { name, .. } => name,
        }
    }

    /// Canonical fully qualified Morphir source name.
    pub fn source_name(&self) -> &str {
        match self {
            Self::Alias { source_name, .. }
            | Self::Opaque { source_name, .. }
            | Self::Custom { source_name, .. }
            | Self::Incomplete { source_name, .. } => source_name,
        }
    }
}

/// The amount of information available for an incomplete v4 type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IncompletenessKind {
    /// A deliberately unfinished draft.
    Draft,
    /// A missing type hole.
    Hole,
}

/// A custom type constructor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Constructor {
    /// Exact canonical Morphir constructor FQName.
    pub source_name: String,
    /// Canonical local constructor name.
    pub name: String,
    /// Constructor payload arguments.
    pub arguments: Vec<NamedType>,
}

/// A named field, constructor argument, or value input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NamedType {
    /// Canonical field or argument name.
    pub name: String,
    /// Associated type expression.
    pub tpe: TypeExpr,
}

/// Morphir type forms that can contribute to Avro schemas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
pub enum TypeExpr {
    /// A type parameter reference.
    Variable(String),
    /// A fully qualified type reference with concrete arguments.
    Reference {
        /// Exact canonical referenced Morphir FQName.
        source_name: String,
        /// Applied type arguments.
        arguments: Vec<TypeExpr>,
    },
    /// A positional product type.
    Tuple(Vec<TypeExpr>),
    /// A closed structural record.
    Record(Vec<NamedType>),
    /// An open structural record.
    ExtensibleRecord {
        /// Row variable name.
        variable: String,
        /// Known record fields.
        fields: Vec<NamedType>,
    },
    /// A function type stored within another type expression.
    Function {
        /// Function input type.
        input: Box<TypeExpr>,
        /// Function output type.
        output: Box<TypeExpr>,
    },
    /// The unit type.
    Unit,
}

/// A public value signature. It intentionally has no value body field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValueSpecification {
    /// Exact canonical Morphir value FQName.
    pub source_name: String,
    /// Canonical local value name.
    pub name: String,
    /// Flattened function inputs.
    pub inputs: Vec<NamedType>,
    /// Output type, absent for incomplete values.
    pub output: Option<TypeExpr>,
    /// Function or constant classification.
    pub value_kind: ValueKind,
    /// Declared application entry-point metadata.
    pub entry_point: Option<EntryPointMetadata>,
    /// Optional source documentation.
    pub doc: Option<String>,
}

/// Whether a value is invoked with arguments or denotes a constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ValueKind {
    /// A value accepting one or more inputs.
    Function,
    /// A zero-argument value specification.
    Constant,
}

/// Morphir v4 application entry-point category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryPointKind {
    /// The primary application entry point.
    Main,
    /// A command entry point.
    Command,
    /// An event or request handler.
    Handler,
}

/// Metadata attached to a declared v4 application entry point.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryPointMetadata {
    /// Stable application-level entry-point identifier.
    pub identifier: String,
    /// Declared entry-point category.
    pub kind: EntryPointKind,
    /// Optional entry-point documentation.
    pub doc: Option<String>,
}
