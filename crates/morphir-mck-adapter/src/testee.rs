//! The operations that answer each request in the kit's adapter protocol.
//!
//! `decode` reads one node in one profile at one IR version and answers with the canonical
//! spelling of what it read, the node kind a `rejected expect=<Kind>` fence names, and the
//! legacy spellings accepted on the way (`protocol.schema.json`'s `DecodeSuccess`). The
//! spellings themselves are morphir-core's; nothing here decides what a member is called.
//!
//! # `current` and `pinned`
//!
//! The kit's two path modes are the two *module paths* a binding exposes — its newest pinned
//! version module and the current alias that points at it — and the driver requires the two to
//! agree fence by fence (kit README, "Before any of that, the driver asks the testee for its
//! capabilities"). They are not a spelling window: this binding has one set of readers, so both
//! modes decode identically, and a legacy spelling accepted under decision 0006's window warns
//! the same way on each. morphir-core's `SpellingMode::Pinned` is the mechanism that closes that
//! window at a later release, not something a path mode selects.
//!
//! # Version 3
//!
//! A version 3 node is read into the classic model and written back in the classic spelling.
//! `decode` does not migrate: the driver holds the answer against the case's own canonical
//! fence, and a case pinned to version 3 spells its canonical in version 3.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use morphir_core::ir::classic;
use morphir_core::ir::json::write_canonical;
use morphir_core::ir::v4::{
    AccessControlled, Annotation, AnnotationArgument, ApplicationContent, ConstructorArg,
    ConstructorArgSpec, ConstructorDefinition, ConstructorSpecification, Distribution,
    DistributionManifestFile, Documented, Field, FormatVersion, IRFile, Incompleteness, LetBinding,
    LibraryContent, Literal, ModuleDefinition, ModuleEntries, ModuleManifestFile,
    ModuleSpecification, NodeFileBody, PackageDefinition, PackageSpecification, Pattern,
    PatternCase, RecordFieldEntry, SpecsContent, SpellingMode, Type, TypeAttributes,
    TypeDefinition, TypeDefinitionFile, TypeEncoding, TypeSpecification, Value, ValueAttributes,
    ValueBody, ValueDefinition, ValueDefinitionFile, ValueSpecification, with_spelling_mode,
    with_type_encoding,
};
use morphir_core::ir::{Diagnostic, DiagnosticCode, Warning};
use morphir_core::naming::{FQName, Name, Path};

use crate::protocol::{
    DecodeRequest, DecodeResponse, NodeKind, PathMode, Profile, ProtocolDiagnostic,
};

/// The stack every decode runs on.
///
/// morphir-core's readers state their own nesting ceiling (`morphir_core::ir::json::MAX_DEPTH`,
/// `morphir_core::ir::yaml::MAX_DEPTH`) and recurse once per level, and their own document-tree
/// deserializers do too; 1000 levels of an unoptimized build's frames do not fit in the stack a
/// thread is given by default — on Windows the main thread's stack is whatever the linker
/// reserved, which is 1 MiB unless someone says otherwise. So the work runs on a thread with a
/// stack this crate states rather than inherits.
///
/// This is what one request reserves, not what a kit run reserves. [`decode`] spawns one scoped
/// thread per request and joins it before returning, so the stack is gone before the answer is
/// written: a run of hundreds of cases holds one of these at a time, never one per case. It is
/// reserved address space in any event — a shallow document commits the pages it touches and no
/// more.
const DECODE_STACK_BYTES: usize = 64 * 1024 * 1024;

/// Reads one node and answers with its canonical spelling or the diagnostic that refused it.
///
/// The reading itself is [`decode_here`]; this wrapper only supplies the stack it needs (see
/// [`DECODE_STACK_BYTES`]). Scoped so the request does not have to be cloned, and the thread is
/// joined before this returns, so nothing about the answer changes.
pub fn decode(req: &DecodeRequest) -> DecodeResponse {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(DECODE_STACK_BYTES)
            .spawn_scoped(scope, || decode_here(req))
            .expect("a decode thread")
            .join()
            // A panic in the decoder is this adapter's bug, not a statement about the document,
            // and the framing loop has no way to answer one honestly. Resuming it lets the
            // process die the way it would have without the thread.
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

fn decode_here(req: &DecodeRequest) -> DecodeResponse {
    // Neither of these can happen while the driver honours `capabilities`, and neither is a
    // statement about the document, so they answer `protocol_error` rather than spending one of
    // the kit's diagnostic codes on "this binding does not do that".
    //
    // Version 3 is read and written in the classic model, whose only spelling is JSON: the kit's
    // version 3 cases carry no yaml fence, so there is nothing for this binding to answer.
    if req.version == 3 && req.profile == Profile::Yaml {
        return DecodeResponse::Refused {
            diagnostic: ProtocolDiagnostic::new("this binding decodes version 3 as json only"),
        };
    }
    if !matches!(req.version, 3 | 4) {
        return DecodeResponse::Refused {
            diagnostic: ProtocolDiagnostic::new(format!(
                "this binding decodes IR versions 3 and 4, not {}",
                req.version
            )),
        };
    }

    match read(req) {
        Ok((node, warnings)) => {
            let node = if req.strip { node.stripped() } else { node };
            match node.write(req.profile) {
                Ok(text) => DecodeResponse::Ok {
                    kind: node.kind().to_string(),
                    canonical: BTreeMap::from([(profile_key(req.profile).to_string(), text)]),
                    warnings,
                },
                Err(diagnostic) => DecodeResponse::Err { diagnostic },
            }
        }
        Err(diagnostic) => DecodeResponse::Err { diagnostic },
    }
}

/// The key a canonical answer is filed under: the profile the request asked for.
fn profile_key(profile: Profile) -> &'static str {
    match profile {
        Profile::Json => "json",
        Profile::Yaml => "yaml",
    }
}

fn read(req: &DecodeRequest) -> Result<(Node, Vec<Warning>), Diagnostic> {
    let value = match req.profile {
        // morphir-core's reader settles the repeated-member and nesting-ceiling rules before the
        // text becomes a value: `serde_json::Value` folds a repeated member onto the last one
        // written and would hide it.
        Profile::Json => {
            let value = morphir_core::ir::json::read(&req.input)?;
            if req.version == 3 {
                return read_v3(req).map(|node| (node, Vec::new()));
            }
            value
        }
        // The YAML reader walks the document itself, so the repeated member and the nesting
        // ceiling are already its answers, in the kit's codes and with the kit's cursors; there
        // is no separate probe. Version 3 never reaches here (`decode_here` refuses it).
        Profile::Yaml => morphir_core::ir::yaml::read(&req.input)?,
    };
    read_v4(req, value)
}

// =============================================================================
// Version 4
// =============================================================================

fn read_v4(req: &DecodeRequest, value: Json) -> Result<(Node, Vec<Warning>), Diagnostic> {
    // Both path modes read through the same readers, so both decode under the open window (see
    // the module's note on `current` and `pinned`). `req.path` is matched rather than ignored so
    // the day the two paths differ, this is where that shows up.
    let mode = match req.path {
        PathMode::Current | PathMode::Pinned => SpellingMode::Current,
    };
    let (node, warnings) = with_spelling_mode(mode, || read_v4_node(req.node, value));
    Ok((node?, warnings))
}

fn read_v4_node(kind: NodeKind, value: Json) -> Result<Node, Diagnostic> {
    fn of<T: for<'de> Deserialize<'de>>(
        value: Json,
        wrap: fn(T) -> Node,
    ) -> Result<Node, Diagnostic> {
        serde_json::from_value::<T>(value)
            .map(wrap)
            .map_err(|error| recover(&error))
    }

    match kind {
        NodeKind::Name => of(value, Node::Name),
        NodeKind::Path => of(value, Node::Path),
        NodeKind::FQName => of(value, Node::FQName),
        NodeKind::FormatVersion => of(value, Node::FormatVersion),
        NodeKind::Type => of(value, Node::Type),
        NodeKind::Literal => of(value, Node::Literal),
        NodeKind::Pattern => of(value, Node::Pattern),
        NodeKind::Value => of(value, Node::Value),
        NodeKind::TypeSpecification => of(value, Node::TypeSpecification),
        NodeKind::TypeDefinition => of(value, Node::TypeDefinition),
        NodeKind::ValueSpecification => of(value, Node::ValueSpecification),
        NodeKind::ValueDefinition => of(value, Node::ValueDefinition),
        NodeKind::AccessControlledTypeDefinition => of(value, Node::AccessControlledTypeDefinition),
        NodeKind::AccessControlledValueDefinition => {
            of(value, Node::AccessControlledValueDefinition)
        }
        NodeKind::ModuleDefinition => of(value, Node::ModuleDefinition),
        NodeKind::ModuleSpecification => of(value, Node::ModuleSpecification),
        // The kit names the whole document `Distribution` as well as `IRFile`; both spell the
        // same node, a format version beside the distribution it applies to.
        NodeKind::IRFile | NodeKind::Distribution => of(value, Node::IRFile),
        // A tree file read on its own, with no tree around it: a module manifest therefore reads
        // an inline listing as definitions, which is what the reference's own node reader does.
        NodeKind::DistributionManifestFile => of(value, Node::DistributionManifestFile),
        NodeKind::ModuleManifestFile => of(value, Node::ModuleManifestFile),
        NodeKind::TypeDefinitionFile => of(value, Node::TypeDefinitionFile),
        NodeKind::ValueDefinitionFile => of(value, Node::ValueDefinitionFile),
    }
}

/// The diagnostic a serde failure carried, or an `invalid_type` naming what serde said.
///
/// Every decoder morphir-core owns smuggles a [`Diagnostic`] through the serde error, so the
/// fallback only fires where a derived impl is still doing the reading — which is a gap in the
/// codec rather than a real answer about the document.
fn recover(error: &serde_json::Error) -> Diagnostic {
    Diagnostic::from_serde_error(error).unwrap_or_else(|| {
        Diagnostic::normalization(DiagnosticCode::InvalidType, "/", error.to_string())
    })
}

// =============================================================================
// Version 3
// =============================================================================

/// The classic value attribute: `{}` before type inference has run, the inferred type after.
///
/// Reading it as an `Attrs` rather than as a bare `Type` is what lets both kinds of classic
/// document through the same reader, and it is also what makes `strip` expressible: clearing a
/// value's attributes is `Attrs::None`, which writes back as `{}`.
type ClassicAnnotation = classic::Attrs<classic::Type<classic::Attrs>>;

/// A classic value expression as this adapter reads one.
type ClassicValue = classic::Value<classic::Attrs, ClassicAnnotation>;
type ClassicPattern = classic::Pattern<ClassicAnnotation>;
type ClassicValueDefinition = classic::ValueDefinition<classic::Attrs, ClassicAnnotation>;
type ClassicArgument = classic::value::ValueArgument<classic::Attrs, ClassicAnnotation>;

fn read_v3(req: &DecodeRequest) -> Result<Node, Diagnostic> {
    fn of<T: for<'de> Deserialize<'de>>(
        text: &str,
        wrap: fn(T) -> Node,
    ) -> Result<Node, Diagnostic> {
        serde_json::from_str::<T>(text)
            .map(wrap)
            .map_err(|error| recover(&error))
    }

    let text = &req.input;
    match req.node {
        NodeKind::Name => of(text, Node::ClassicName),
        NodeKind::Path => of(text, Node::ClassicPath),
        NodeKind::FQName => of(text, Node::ClassicFQName),
        NodeKind::Literal => of(text, Node::ClassicLiteral),
        NodeKind::Type => of(text, Node::ClassicType),
        NodeKind::Pattern => of(text, Node::ClassicPattern),
        NodeKind::Value => of(text, Node::ClassicValue),
        NodeKind::ValueDefinition => of(text, Node::ClassicValueDefinition),
        NodeKind::TypeSpecification => of(text, Node::ClassicTypeSpecification),
        // The rest are nodes a classic document only ever carries inside a whole distribution,
        // whose value attribute is the inferred type itself rather than something a reader can
        // clear — so there is no version 3 answer this adapter can give for them on their own.
        NodeKind::FormatVersion
        | NodeKind::TypeDefinition
        | NodeKind::ValueSpecification
        | NodeKind::AccessControlledTypeDefinition
        | NodeKind::AccessControlledValueDefinition
        | NodeKind::ModuleDefinition
        | NodeKind::ModuleSpecification
        | NodeKind::IRFile
        | NodeKind::Distribution
        // A document tree is a version 4 layout; version 3 has no file of any of these kinds.
        | NodeKind::DistributionManifestFile
        | NodeKind::ModuleManifestFile
        | NodeKind::TypeDefinitionFile
        | NodeKind::ValueDefinitionFile => Err(Diagnostic::normalization(
            DiagnosticCode::UnknownNode,
            "/",
            format!(
                "{:?} is not a node this binding reads on its own at version 3",
                req.node
            ),
        )),
    }
}

/// Clearing a classic value's attributes is writing `{}` in each attribute position — which is
/// the same thing an untyped classic document already says.
///
/// A classic *type* attribute is `Attrs<()>`, and no input the profile admits decodes it as
/// anything but `Attrs::None`, so a type carries nothing to clear and is returned as it came.
fn strip_classic_value(node: ClassicValue) -> ClassicValue {
    let attributes = classic::Attrs::None;
    match node {
        classic::Value::Apply(_, function, argument) => classic::Value::Apply(
            attributes,
            Box::new(strip_classic_value(*function)),
            Box::new(strip_classic_value(*argument)),
        ),
        classic::Value::Constructor(_, name) => classic::Value::Constructor(attributes, name),
        classic::Value::Destructure(_, pattern, subject, body) => classic::Value::Destructure(
            attributes,
            strip_classic_pattern(pattern),
            Box::new(strip_classic_value(*subject)),
            Box::new(strip_classic_value(*body)),
        ),
        classic::Value::Field(_, target, name) => {
            classic::Value::Field(attributes, Box::new(strip_classic_value(*target)), name)
        }
        classic::Value::FieldFunction(_, name) => classic::Value::FieldFunction(attributes, name),
        classic::Value::IfThenElse(_, condition, then_branch, else_branch) => {
            classic::Value::IfThenElse(
                attributes,
                Box::new(strip_classic_value(*condition)),
                Box::new(strip_classic_value(*then_branch)),
                Box::new(strip_classic_value(*else_branch)),
            )
        }
        classic::Value::Lambda(_, pattern, body) => classic::Value::Lambda(
            attributes,
            strip_classic_pattern(pattern),
            Box::new(strip_classic_value(*body)),
        ),
        classic::Value::LetDefinition(_, name, definition, body) => classic::Value::LetDefinition(
            attributes,
            name,
            Box::new(strip_classic_value_definition(*definition)),
            Box::new(strip_classic_value(*body)),
        ),
        classic::Value::LetRecursion(_, bindings, body) => classic::Value::LetRecursion(
            attributes,
            bindings
                .into_iter()
                .map(|(name, definition)| {
                    (name, Box::new(strip_classic_value_definition(*definition)))
                })
                .collect(),
            Box::new(strip_classic_value(*body)),
        ),
        classic::Value::List(_, values) => classic::Value::List(
            attributes,
            values.into_iter().map(strip_classic_value).collect(),
        ),
        classic::Value::Literal(_, literal) => classic::Value::Literal(attributes, literal),
        classic::Value::PatternMatch(_, subject, cases) => classic::Value::PatternMatch(
            attributes,
            Box::new(strip_classic_value(*subject)),
            cases
                .into_iter()
                .map(|(pattern, body)| (strip_classic_pattern(pattern), strip_classic_value(body)))
                .collect(),
        ),
        classic::Value::Record(_, fields) => classic::Value::Record(
            attributes,
            fields
                .into_iter()
                .map(|(name, value)| (name, strip_classic_value(value)))
                .collect(),
        ),
        classic::Value::Tuple(_, values) => classic::Value::Tuple(
            attributes,
            values.into_iter().map(strip_classic_value).collect(),
        ),
        classic::Value::Unit(_) => classic::Value::Unit(attributes),
        classic::Value::Update(_, record, fields) => classic::Value::Update(
            attributes,
            Box::new(strip_classic_value(*record)),
            fields
                .into_iter()
                .map(|(name, value)| (name, strip_classic_value(value)))
                .collect(),
        ),
        classic::Value::Variable(_, name) => classic::Value::Variable(attributes, name),
        classic::Value::Reference(_, name) => classic::Value::Reference(attributes, name),
    }
}

fn strip_classic_pattern(node: ClassicPattern) -> ClassicPattern {
    let attributes = classic::Attrs::None;
    match node {
        classic::Pattern::Wildcard(_) => classic::Pattern::Wildcard(attributes),
        classic::Pattern::As(_, pattern, name) => {
            classic::Pattern::As(attributes, Box::new(strip_classic_pattern(*pattern)), name)
        }
        classic::Pattern::Tuple(_, patterns) => classic::Pattern::Tuple(
            attributes,
            patterns.into_iter().map(strip_classic_pattern).collect(),
        ),
        classic::Pattern::Constructor(_, name, arguments) => classic::Pattern::Constructor(
            attributes,
            name,
            arguments.into_iter().map(strip_classic_pattern).collect(),
        ),
        classic::Pattern::EmptyList(_) => classic::Pattern::EmptyList(attributes),
        classic::Pattern::HeadTail(_, head, tail) => classic::Pattern::HeadTail(
            attributes,
            Box::new(strip_classic_pattern(*head)),
            Box::new(strip_classic_pattern(*tail)),
        ),
        classic::Pattern::Literal(_, literal) => classic::Pattern::Literal(attributes, literal),
        classic::Pattern::Unit(_) => classic::Pattern::Unit(attributes),
    }
}

fn strip_classic_argument(argument: ClassicArgument) -> ClassicArgument {
    classic::value::ValueArgument {
        name: argument.name,
        annotation: classic::Attrs::None,
        ty: argument.ty,
    }
}

fn strip_classic_value_definition(definition: ClassicValueDefinition) -> ClassicValueDefinition {
    classic::ValueDefinition {
        input_types: definition
            .input_types
            .into_iter()
            .map(strip_classic_argument)
            .collect(),
        output_type: definition.output_type,
        body: strip_classic_value(definition.body),
    }
}

fn classic_literal_kind(literal: &classic::Literal) -> &'static str {
    match literal {
        classic::Literal::Bool(_) => "BoolLiteral",
        classic::Literal::Char(_) => "CharLiteral",
        classic::Literal::String(_) => "StringLiteral",
        classic::Literal::WholeNumber(_) => "WholeNumberLiteral",
        classic::Literal::Float(_) => "FloatLiteral",
        classic::Literal::Decimal(_) => "DecimalLiteral",
    }
}

fn classic_type_kind(node: &classic::Type<classic::Attrs>) -> &'static str {
    match node {
        classic::Type::ExtensibleRecord(..) => "ExtensibleRecord",
        classic::Type::Function(..) => "Function",
        classic::Type::Record(..) => "Record",
        classic::Type::Reference(..) => "Reference",
        classic::Type::Tuple(..) => "Tuple",
        classic::Type::Unit(_) => "Unit",
        classic::Type::Variable(..) => "Variable",
    }
}

fn classic_type_specification_kind(
    node: &classic::TypeSpecification<classic::Attrs>,
) -> &'static str {
    match node {
        classic::TypeSpecification::Alias(..) => "TypeAliasSpecification",
        classic::TypeSpecification::Opaque(_) => "OpaqueTypeSpecification",
        classic::TypeSpecification::Custom(..) => "CustomTypeSpecification",
        classic::TypeSpecification::Derived(..) => "DerivedTypeSpecification",
    }
}

fn classic_pattern_kind(node: &ClassicPattern) -> &'static str {
    match node {
        classic::Pattern::Wildcard(_) => "WildcardPattern",
        classic::Pattern::As(..) => "AsPattern",
        classic::Pattern::Tuple(..) => "TuplePattern",
        classic::Pattern::Constructor(..) => "ConstructorPattern",
        classic::Pattern::EmptyList(_) => "EmptyListPattern",
        classic::Pattern::HeadTail(..) => "HeadTailPattern",
        classic::Pattern::Literal(..) => "LiteralPattern",
        classic::Pattern::Unit(_) => "UnitPattern",
    }
}

fn classic_value_kind(node: &ClassicValue) -> &'static str {
    match node {
        classic::Value::Apply(..) => "Apply",
        classic::Value::Constructor(..) => "Constructor",
        classic::Value::Destructure(..) => "Destructure",
        classic::Value::Field(..) => "Field",
        classic::Value::FieldFunction(..) => "FieldFunction",
        classic::Value::IfThenElse(..) => "IfThenElse",
        classic::Value::Lambda(..) => "Lambda",
        classic::Value::LetDefinition(..) => "LetDefinition",
        classic::Value::LetRecursion(..) => "LetRecursion",
        classic::Value::List(..) => "List",
        classic::Value::Literal(..) => "Literal",
        classic::Value::PatternMatch(..) => "PatternMatch",
        classic::Value::Record(..) => "Record",
        classic::Value::Tuple(..) => "Tuple",
        classic::Value::Unit(_) => "Unit",
        classic::Value::Update(..) => "UpdateRecord",
        classic::Value::Variable(..) => "Variable",
        classic::Value::Reference(..) => "Reference",
    }
}

// =============================================================================
// The decoded node
// =============================================================================

/// One decoded node of any kind the kit names.
#[derive(Debug, Clone)]
enum Node {
    Name(Name),
    Path(Path),
    FQName(FQName),
    FormatVersion(FormatVersion),
    Type(Type),
    Literal(Literal),
    Pattern(Pattern),
    Value(Value),
    TypeSpecification(TypeSpecification),
    TypeDefinition(TypeDefinition),
    ValueSpecification(ValueSpecification),
    ValueDefinition(ValueDefinition),
    AccessControlledTypeDefinition(AccessControlled<Documented<TypeDefinition>>),
    AccessControlledValueDefinition(AccessControlled<Documented<ValueDefinition>>),
    ModuleDefinition(ModuleDefinition),
    ModuleSpecification(ModuleSpecification),
    IRFile(IRFile),
    // The four files a document tree is made of. Each is a node in its own right, so it is read,
    // stripped and written here the way every other node is.
    DistributionManifestFile(DistributionManifestFile),
    ModuleManifestFile(ModuleManifestFile),
    TypeDefinitionFile(TypeDefinitionFile),
    ValueDefinitionFile(ValueDefinitionFile),
    // Version 3 stays in the classic model: it is read, stripped and written there, so a case
    // pinned to version 3 is answered in the spelling its own canonical fence uses.
    ClassicName(classic::Name),
    ClassicPath(classic::Path),
    ClassicFQName(classic::FQName),
    ClassicLiteral(classic::Literal),
    ClassicType(classic::Type<classic::Attrs>),
    ClassicTypeSpecification(classic::TypeSpecification<classic::Attrs>),
    ClassicPattern(ClassicPattern),
    ClassicValue(ClassicValue),
    ClassicValueDefinition(ClassicValueDefinition),
}

impl Node {
    /// The name a `rejected expect=<Kind>` fence means: the variant the node decoded to, or the
    /// node's own name where it has no variants.
    fn kind(&self) -> &'static str {
        match self {
            Node::Name(_) => "Name",
            Node::Path(_) => "Path",
            Node::FQName(_) => "FQName",
            Node::FormatVersion(_) => "FormatVersion",
            Node::Type(node) => type_kind(node),
            Node::Literal(node) => literal_kind(node),
            Node::Pattern(node) => pattern_kind(node),
            Node::Value(node) => value_kind(node),
            Node::TypeSpecification(node) => type_specification_kind(node),
            Node::TypeDefinition(node) => type_definition_kind(node),
            Node::ValueSpecification(_) => "ValueSpecification",
            Node::ValueDefinition(node) => value_definition_kind(node),
            Node::AccessControlledTypeDefinition(node) => type_definition_kind(&node.value.value),
            Node::AccessControlledValueDefinition(node) => value_definition_kind(&node.value.value),
            Node::ModuleDefinition(_) => "ModuleDefinition",
            Node::ModuleSpecification(_) => "ModuleSpecification",
            Node::IRFile(node) => distribution_kind(&node.distribution),
            // A manifest has no variants: it is the file kind itself.
            Node::DistributionManifestFile(_) => "DistributionManifestFile",
            Node::ModuleManifestFile(_) => "ModuleManifestFile",
            // A node file answers with what is inside it, the way a bare definition or
            // specification node does.
            Node::TypeDefinitionFile(node) => match &node.body {
                NodeFileBody::Def(definition) => type_definition_kind(&definition.value.value),
                NodeFileBody::Spec(specification) => type_specification_kind(&specification.value),
            },
            // A value specification has no variants, so a spec file answers with the name of what
            // it holds, the way a bare ValueSpecification node does.
            Node::ValueDefinitionFile(node) => match &node.body {
                NodeFileBody::Def(definition) => value_definition_kind(&definition.value.value),
                NodeFileBody::Spec(_) => "ValueSpecification",
            },
            Node::ClassicName(_) => "Name",
            Node::ClassicPath(_) => "Path",
            Node::ClassicFQName(_) => "FQName",
            Node::ClassicLiteral(node) => classic_literal_kind(node),
            Node::ClassicType(node) => classic_type_kind(node),
            Node::ClassicTypeSpecification(node) => classic_type_specification_kind(node),
            Node::ClassicPattern(node) => classic_pattern_kind(node),
            Node::ClassicValue(node) => classic_value_kind(node),
            Node::ClassicValueDefinition(_) => "ValueDefinition",
        }
    }

    /// The node with every attribute cleared, so two spellings are compared on meaning alone.
    fn stripped(self) -> Node {
        match self {
            Node::Type(node) => Node::Type(strip_type(node)),
            Node::Pattern(node) => Node::Pattern(strip_pattern(node)),
            Node::Value(node) => Node::Value(strip_value(node)),
            Node::TypeSpecification(node) => {
                Node::TypeSpecification(strip_type_specification(node))
            }
            Node::TypeDefinition(node) => Node::TypeDefinition(strip_type_definition(node)),
            Node::ValueSpecification(node) => {
                Node::ValueSpecification(strip_value_specification(node))
            }
            Node::ValueDefinition(node) => Node::ValueDefinition(strip_value_definition(node)),
            Node::AccessControlledTypeDefinition(node) => {
                Node::AccessControlledTypeDefinition(strip_access_controlled(node, |d| {
                    strip_documented(d, strip_type_definition)
                }))
            }
            Node::AccessControlledValueDefinition(node) => {
                Node::AccessControlledValueDefinition(strip_access_controlled(node, |d| {
                    strip_documented(d, strip_value_definition)
                }))
            }
            Node::ModuleDefinition(node) => Node::ModuleDefinition(strip_module_definition(node)),
            Node::ModuleSpecification(node) => {
                Node::ModuleSpecification(strip_module_specification(node))
            }
            Node::IRFile(node) => Node::IRFile(IRFile {
                format_version: node.format_version,
                distribution: strip_distribution(node.distribution),
            }),
            // A distribution manifest is names, a kind and a budget: nothing it holds carries
            // attributes, so it is returned as it came (the `other` arm below would do the same,
            // but saying so here keeps the four file kinds together).
            Node::DistributionManifestFile(node) => Node::DistributionManifestFile(node),
            Node::ModuleManifestFile(node) => Node::ModuleManifestFile(ModuleManifestFile {
                types: strip_module_entries(node.types, strip_access_controlled_type_definition, {
                    |specification| strip_documented(specification, strip_type_specification)
                }),
                values: strip_module_entries(
                    node.values,
                    strip_access_controlled_value_definition,
                    |specification| strip_documented(specification, strip_value_specification),
                ),
                ..node
            }),
            Node::TypeDefinitionFile(node) => Node::TypeDefinitionFile(TypeDefinitionFile {
                body: match node.body {
                    NodeFileBody::Def(definition) => {
                        NodeFileBody::Def(strip_access_controlled_type_definition(definition))
                    }
                    NodeFileBody::Spec(specification) => NodeFileBody::Spec(strip_documented(
                        specification,
                        strip_type_specification,
                    )),
                },
                ..node
            }),
            Node::ValueDefinitionFile(node) => Node::ValueDefinitionFile(ValueDefinitionFile {
                body: match node.body {
                    NodeFileBody::Def(definition) => {
                        NodeFileBody::Def(strip_access_controlled_value_definition(definition))
                    }
                    NodeFileBody::Spec(specification) => NodeFileBody::Spec(strip_documented(
                        specification,
                        strip_value_specification,
                    )),
                },
                ..node
            }),
            Node::ClassicPattern(node) => Node::ClassicPattern(strip_classic_pattern(node)),
            Node::ClassicValue(node) => Node::ClassicValue(strip_classic_value(node)),
            Node::ClassicValueDefinition(node) => {
                Node::ClassicValueDefinition(strip_classic_value_definition(node))
            }
            // Names, paths, literals, the format version, a classic type and a classic type
            // specification — which carries nothing but types — have no attributes a reader can
            // clear.
            other => other,
        }
    }

    /// The canonical spelling of this node in the requested profile, in the compact type
    /// encoding, with the one trailing newline a canonical fence carries.
    ///
    /// Both profiles write the same value tree — the node's compact serialisation — so a case's
    /// two canonical fences are two spellings of one answer rather than two answers.
    fn write(&self, profile: Profile) -> Result<String, Diagnostic> {
        let value = self.value()?;
        Ok(match profile {
            Profile::Json => format!("{}\n", write_canonical(&value)),
            // The YAML writer ends its output with the newline itself.
            Profile::Yaml => morphir_core::ir::yaml::write_canonical(&value),
        })
    }

    /// This node as the value tree both canonical writers spell, in the compact type encoding.
    fn value(&self) -> Result<Json, Diagnostic> {
        fn text<T: Serialize>(node: &T) -> Result<Json, Diagnostic> {
            with_type_encoding(TypeEncoding::Compact, || serde_json::to_value(node)).map_err(
                |error| {
                    Diagnostic::normalization(DiagnosticCode::InvalidType, "/", error.to_string())
                },
            )
        }

        match self {
            Node::Name(node) => text(node),
            Node::Path(node) => text(node),
            Node::FQName(node) => text(node),
            Node::FormatVersion(node) => text(node),
            Node::Type(node) => text(node),
            Node::Literal(node) => text(node),
            Node::Pattern(node) => text(node),
            Node::Value(node) => text(node),
            Node::TypeSpecification(node) => text(node),
            Node::TypeDefinition(node) => text(node),
            Node::ValueSpecification(node) => text(node),
            Node::ValueDefinition(node) => text(node),
            Node::AccessControlledTypeDefinition(node) => text(node),
            Node::AccessControlledValueDefinition(node) => text(node),
            Node::ModuleDefinition(node) => text(node),
            Node::ModuleSpecification(node) => text(node),
            // `formatVersion` first, then `distribution`: the root member order of the file.
            Node::IRFile(node) => text(node),
            Node::DistributionManifestFile(node) => text(node),
            Node::ModuleManifestFile(node) => text(node),
            Node::TypeDefinitionFile(node) => text(node),
            Node::ValueDefinitionFile(node) => text(node),
            Node::ClassicName(node) => text(node),
            Node::ClassicPath(node) => text(node),
            Node::ClassicFQName(node) => text(node),
            Node::ClassicLiteral(node) => text(node),
            Node::ClassicType(node) => text(node),
            Node::ClassicTypeSpecification(node) => text(node),
            Node::ClassicPattern(node) => text(node),
            Node::ClassicValue(node) => text(node),
            Node::ClassicValueDefinition(node) => text(node),
        }
    }
}

fn type_kind(node: &Type) -> &'static str {
    match node {
        Type::Variable(..) => "Variable",
        Type::Reference(..) => "Reference",
        Type::Tuple(..) => "Tuple",
        Type::Record(..) => "Record",
        Type::ExtensibleRecord(..) => "ExtensibleRecord",
        Type::Function(..) => "Function",
        Type::Unit(..) => "Unit",
    }
}

fn literal_kind(node: &Literal) -> &'static str {
    match node {
        Literal::Bool(_) => "BoolLiteral",
        Literal::Char(_) => "CharLiteral",
        Literal::String(_) => "StringLiteral",
        Literal::Integer(_) => "IntegerLiteral",
        Literal::Float(_) => "FloatLiteral",
        Literal::Decimal(_) => "DecimalLiteral",
        Literal::Document(_) => "DocumentLiteral",
    }
}

fn pattern_kind(node: &Pattern) -> &'static str {
    match node {
        Pattern::WildcardPattern(_) => "WildcardPattern",
        Pattern::AsPattern(..) => "AsPattern",
        Pattern::TuplePattern(..) => "TuplePattern",
        Pattern::ConstructorPattern(..) => "ConstructorPattern",
        Pattern::EmptyListPattern(_) => "EmptyListPattern",
        Pattern::HeadTailPattern(..) => "HeadTailPattern",
        Pattern::LiteralPattern(..) => "LiteralPattern",
        Pattern::UnitPattern(_) => "UnitPattern",
    }
}

fn value_kind(node: &Value) -> &'static str {
    match node {
        Value::Literal(..) => "Literal",
        Value::Constructor(..) => "Constructor",
        Value::Tuple(..) => "Tuple",
        Value::List(..) => "List",
        Value::Record(..) => "Record",
        Value::Variable(..) => "Variable",
        Value::Reference(..) => "Reference",
        Value::Field(..) => "Field",
        Value::FieldFunction(..) => "FieldFunction",
        Value::Apply(..) => "Apply",
        Value::Lambda(..) => "Lambda",
        Value::LetDefinition(..) => "LetDefinition",
        Value::LetRecursion(..) => "LetRecursion",
        Value::Destructure(..) => "Destructure",
        Value::IfThenElse(..) => "IfThenElse",
        Value::PatternMatch(..) => "PatternMatch",
        Value::UpdateRecord(..) => "UpdateRecord",
        Value::Unit(_) => "Unit",
        Value::Hole(..) => "Hole",
    }
}

fn type_specification_kind(node: &TypeSpecification) -> &'static str {
    match node {
        TypeSpecification::TypeAliasSpecification { .. } => "TypeAliasSpecification",
        TypeSpecification::OpaqueTypeSpecification { .. } => "OpaqueTypeSpecification",
        TypeSpecification::CustomTypeSpecification { .. } => "CustomTypeSpecification",
        TypeSpecification::DerivedTypeSpecification { .. } => "DerivedTypeSpecification",
    }
}

fn type_definition_kind(node: &TypeDefinition) -> &'static str {
    match node {
        TypeDefinition::TypeAliasDefinition { .. } => "TypeAliasDefinition",
        TypeDefinition::CustomTypeDefinition { .. } => "CustomTypeDefinition",
        TypeDefinition::IncompleteTypeDefinition { .. } => "IncompleteTypeDefinition",
    }
}

fn value_definition_kind(node: &ValueDefinition) -> &'static str {
    match node.body {
        ValueBody::Expression(_) => "ExpressionBody",
        ValueBody::Native { .. } => "NativeBody",
        ValueBody::External { .. } => "ExternalBody",
        ValueBody::Incomplete { .. } => "IncompleteBody",
    }
}

fn distribution_kind(node: &Distribution) -> &'static str {
    match node {
        Distribution::Library(_) => "Library",
        Distribution::Specs(_) => "Specs",
        Distribution::Application(_) => "Application",
    }
}

// =============================================================================
// Clearing attributes
// =============================================================================

fn strip_type(node: Type) -> Type {
    let attributes = TypeAttributes::default();
    match node {
        Type::Variable(_, name) => Type::Variable(attributes, name),
        Type::Reference(_, fqname, arguments) => Type::Reference(
            attributes,
            fqname,
            arguments.into_iter().map(strip_type).collect(),
        ),
        Type::Tuple(_, elements) => {
            Type::Tuple(attributes, elements.into_iter().map(strip_type).collect())
        }
        Type::Record(_, fields) => {
            Type::Record(attributes, fields.into_iter().map(strip_field).collect())
        }
        Type::ExtensibleRecord(_, name, fields) => Type::ExtensibleRecord(
            attributes,
            name,
            fields.into_iter().map(strip_field).collect(),
        ),
        Type::Function(_, parameter, result) => Type::Function(
            attributes,
            Box::new(strip_type(*parameter)),
            Box::new(strip_type(*result)),
        ),
        Type::Unit(_) => Type::Unit(attributes),
    }
}

fn strip_field(field: Field) -> Field {
    Field {
        name: field.name,
        tpe: strip_type(field.tpe),
    }
}

fn strip_pattern(node: Pattern) -> Pattern {
    let attributes = ValueAttributes::default();
    match node {
        Pattern::WildcardPattern(_) => Pattern::WildcardPattern(attributes),
        Pattern::AsPattern(_, pattern, name) => {
            Pattern::AsPattern(attributes, Box::new(strip_pattern(*pattern)), name)
        }
        Pattern::TuplePattern(_, patterns) => Pattern::TuplePattern(
            attributes,
            patterns.into_iter().map(strip_pattern).collect(),
        ),
        Pattern::ConstructorPattern(_, fqname, arguments) => Pattern::ConstructorPattern(
            attributes,
            fqname,
            arguments.into_iter().map(strip_pattern).collect(),
        ),
        Pattern::EmptyListPattern(_) => Pattern::EmptyListPattern(attributes),
        Pattern::HeadTailPattern(_, head, tail) => Pattern::HeadTailPattern(
            attributes,
            Box::new(strip_pattern(*head)),
            Box::new(strip_pattern(*tail)),
        ),
        Pattern::LiteralPattern(_, literal) => Pattern::LiteralPattern(attributes, literal),
        Pattern::UnitPattern(_) => Pattern::UnitPattern(attributes),
    }
}

fn strip_value(node: Value) -> Value {
    let attributes = ValueAttributes::default();
    match node {
        Value::Literal(_, literal) => Value::Literal(attributes, literal),
        Value::Constructor(_, fqname) => Value::Constructor(attributes, fqname),
        Value::Tuple(_, values) => {
            Value::Tuple(attributes, values.into_iter().map(strip_value).collect())
        }
        Value::List(_, values) => {
            Value::List(attributes, values.into_iter().map(strip_value).collect())
        }
        Value::Record(_, fields) => Value::Record(
            attributes,
            fields.into_iter().map(strip_record_field).collect(),
        ),
        Value::Variable(_, name) => Value::Variable(attributes, name),
        Value::Reference(_, fqname) => Value::Reference(attributes, fqname),
        Value::Field(_, target, name) => {
            Value::Field(attributes, Box::new(strip_value(*target)), name)
        }
        Value::FieldFunction(_, name) => Value::FieldFunction(attributes, name),
        Value::Apply(_, function, argument) => Value::Apply(
            attributes,
            Box::new(strip_value(*function)),
            Box::new(strip_value(*argument)),
        ),
        Value::Lambda(_, pattern, body) => Value::Lambda(
            attributes,
            strip_pattern(pattern),
            Box::new(strip_value(*body)),
        ),
        Value::LetDefinition(_, name, definition, body) => Value::LetDefinition(
            attributes,
            name,
            Box::new(strip_value_definition(*definition)),
            Box::new(strip_value(*body)),
        ),
        Value::LetRecursion(_, bindings, body) => Value::LetRecursion(
            attributes,
            bindings
                .into_iter()
                .map(|binding| LetBinding(binding.0, strip_value_definition(binding.1)))
                .collect(),
            Box::new(strip_value(*body)),
        ),
        Value::Destructure(_, pattern, subject, body) => Value::Destructure(
            attributes,
            strip_pattern(pattern),
            Box::new(strip_value(*subject)),
            Box::new(strip_value(*body)),
        ),
        Value::IfThenElse(_, condition, then_branch, else_branch) => Value::IfThenElse(
            attributes,
            Box::new(strip_value(*condition)),
            Box::new(strip_value(*then_branch)),
            Box::new(strip_value(*else_branch)),
        ),
        Value::PatternMatch(_, subject, cases) => Value::PatternMatch(
            attributes,
            Box::new(strip_value(*subject)),
            cases
                .into_iter()
                .map(|case| PatternCase(strip_pattern(case.0), strip_value(case.1)))
                .collect(),
        ),
        Value::UpdateRecord(_, target, fields) => Value::UpdateRecord(
            attributes,
            Box::new(strip_value(*target)),
            fields.into_iter().map(strip_record_field).collect(),
        ),
        Value::Unit(_) => Value::Unit(attributes),
        Value::Hole(_, reason, expected_type) => Value::Hole(
            attributes,
            reason,
            expected_type.map(|t| Box::new(strip_type(*t))),
        ),
    }
}

fn strip_record_field(field: RecordFieldEntry) -> RecordFieldEntry {
    RecordFieldEntry(field.0, strip_value(field.1))
}

/// Strips the attributes off the value expressions an annotation's arguments carry; the names and
/// the free text of an annotation carry none.
fn strip_annotations(annotations: Vec<Annotation>) -> Vec<Annotation> {
    annotations
        .into_iter()
        .map(|annotation| match annotation {
            Annotation::Compact { name, text } => Annotation::Compact { name, text },
            Annotation::Structured { name, args } => Annotation::Structured {
                name,
                args: args
                    .into_iter()
                    .map(|argument| match argument {
                        AnnotationArgument::Positional(value) => {
                            AnnotationArgument::Positional(strip_value(value))
                        }
                        AnnotationArgument::Named { name, value } => AnnotationArgument::Named {
                            name,
                            value: strip_value(value),
                        },
                    })
                    .collect(),
            },
        })
        .collect()
}

fn strip_type_specification(node: TypeSpecification) -> TypeSpecification {
    match node {
        TypeSpecification::TypeAliasSpecification {
            annotations,
            type_params,
            type_expr,
        } => TypeSpecification::TypeAliasSpecification {
            annotations: strip_annotations(annotations),
            type_params,
            type_expr: strip_type(type_expr),
        },
        TypeSpecification::OpaqueTypeSpecification {
            annotations,
            type_params,
        } => TypeSpecification::OpaqueTypeSpecification {
            annotations: strip_annotations(annotations),
            type_params,
        },
        TypeSpecification::CustomTypeSpecification {
            annotations,
            type_params,
            constructors,
        } => TypeSpecification::CustomTypeSpecification {
            annotations: strip_annotations(annotations),
            type_params,
            constructors: constructors
                .into_iter()
                .map(|constructor| ConstructorSpecification {
                    name: constructor.name,
                    args: constructor
                        .args
                        .into_iter()
                        .map(|arg| ConstructorArgSpec {
                            name: arg.name,
                            arg_type: strip_type(arg.arg_type),
                        })
                        .collect(),
                })
                .collect(),
        },
        TypeSpecification::DerivedTypeSpecification {
            annotations,
            type_params,
            base_type,
            from_base_type,
            to_base_type,
        } => TypeSpecification::DerivedTypeSpecification {
            annotations: strip_annotations(annotations),
            type_params,
            base_type: strip_type(base_type),
            from_base_type,
            to_base_type,
        },
    }
}

fn strip_type_definition(node: TypeDefinition) -> TypeDefinition {
    match node {
        TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr,
        } => TypeDefinition::TypeAliasDefinition {
            type_params,
            type_expr: strip_type(type_expr),
        },
        TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors,
        } => TypeDefinition::CustomTypeDefinition {
            type_params,
            constructors: strip_access_controlled(constructors, |definitions| {
                definitions
                    .into_iter()
                    .map(|constructor| ConstructorDefinition {
                        name: constructor.name,
                        args: constructor
                            .args
                            .into_iter()
                            .map(|arg| ConstructorArg {
                                name: arg.name,
                                arg_type: strip_type(arg.arg_type),
                            })
                            .collect(),
                    })
                    .collect()
            }),
        },
        TypeDefinition::IncompleteTypeDefinition {
            type_params,
            incompleteness,
            partial_type_expr,
        } => TypeDefinition::IncompleteTypeDefinition {
            type_params,
            incompleteness: strip_incompleteness(incompleteness),
            partial_type_expr: partial_type_expr.map(strip_type),
        },
    }
}

/// A hole's `partialBody` is a type expression, so it carries attributes the testee strips like
/// any other.
fn strip_incompleteness(node: Incompleteness) -> Incompleteness {
    match node {
        Incompleteness::Draft => Incompleteness::Draft,
        Incompleteness::Hole {
            reason,
            partial_body,
        } => Incompleteness::Hole {
            reason,
            partial_body: partial_body.map(strip_type),
        },
    }
}

fn strip_value_specification(node: ValueSpecification) -> ValueSpecification {
    ValueSpecification {
        annotations: strip_annotations(node.annotations),
        inputs: node
            .inputs
            .into_iter()
            .map(|(name, tpe)| (name, strip_type(tpe)))
            .collect(),
        output: strip_type(node.output),
    }
}

fn strip_value_definition(node: ValueDefinition) -> ValueDefinition {
    ValueDefinition {
        input_types: node
            .input_types
            .into_iter()
            .map(|(name, tpe)| (name, strip_type(tpe)))
            .collect(),
        output_type: node.output_type.map(strip_type),
        body: match node.body {
            ValueBody::Expression(value) => ValueBody::Expression(strip_value(value)),
            ValueBody::Native { native_info } => ValueBody::Native { native_info },
            ValueBody::External {
                externals,
                fallback,
            } => ValueBody::External {
                externals,
                fallback: fallback.map(|value| Box::new(strip_value(*value))),
            },
            ValueBody::Incomplete {
                incompleteness,
                partial_body,
            } => ValueBody::Incomplete {
                incompleteness: strip_incompleteness(incompleteness),
                partial_body: partial_body.map(|value| Box::new(strip_value(*value))),
            },
        },
    }
}

fn strip_access_controlled<T>(
    node: AccessControlled<T>,
    strip: impl FnOnce(T) -> T,
) -> AccessControlled<T> {
    AccessControlled {
        access: node.access,
        value: strip(node.value),
    }
}

/// A module manifest's `types` or `values` with every attribute inside it cleared.
///
/// A names-style listing holds nothing but names, so it has nothing to strip; the bodies it names
/// live in the node files beside the manifest and are stripped when those are read.
fn strip_module_entries<D, S>(
    entries: ModuleEntries<D, S>,
    strip_definition: impl Fn(D) -> D,
    strip_specification: impl Fn(S) -> S,
) -> ModuleEntries<D, S> {
    match entries {
        ModuleEntries::Names(names) => ModuleEntries::Names(names),
        ModuleEntries::Definitions(items) => ModuleEntries::Definitions(
            items
                .into_iter()
                .map(|(name, definition)| (name, strip_definition(definition)))
                .collect(),
        ),
        ModuleEntries::Specifications(items) => ModuleEntries::Specifications(
            items
                .into_iter()
                .map(|(name, specification)| (name, strip_specification(specification)))
                .collect(),
        ),
    }
}

fn strip_access_controlled_type_definition(
    node: AccessControlled<Documented<TypeDefinition>>,
) -> AccessControlled<Documented<TypeDefinition>> {
    strip_access_controlled(node, |documented| {
        strip_documented(documented, strip_type_definition)
    })
}

fn strip_access_controlled_value_definition(
    node: AccessControlled<Documented<ValueDefinition>>,
) -> AccessControlled<Documented<ValueDefinition>> {
    strip_access_controlled(node, |documented| {
        strip_documented(documented, strip_value_definition)
    })
}

fn strip_documented<T>(node: Documented<T>, strip: impl FnOnce(T) -> T) -> Documented<T> {
    Documented {
        doc: node.doc,
        value: strip(node.value),
    }
}

fn strip_module_definition(node: ModuleDefinition) -> ModuleDefinition {
    ModuleDefinition {
        types: node
            .types
            .into_iter()
            .map(|(name, definition)| {
                (
                    name,
                    strip_access_controlled(definition, |d| {
                        strip_documented(d, strip_type_definition)
                    }),
                )
            })
            .collect(),
        values: node
            .values
            .into_iter()
            .map(|(name, definition)| {
                (
                    name,
                    strip_access_controlled(definition, |d| {
                        strip_documented(d, strip_value_definition)
                    }),
                )
            })
            .collect(),
        doc: node.doc,
    }
}

fn strip_module_specification(node: ModuleSpecification) -> ModuleSpecification {
    ModuleSpecification {
        annotations: strip_annotations(node.annotations),
        types: node
            .types
            .into_iter()
            .map(|(name, specification)| {
                (
                    name,
                    strip_documented(specification, strip_type_specification),
                )
            })
            .collect(),
        values: node
            .values
            .into_iter()
            .map(|(name, specification)| {
                (
                    name,
                    strip_documented(specification, strip_value_specification),
                )
            })
            .collect(),
        doc: node.doc,
    }
}

fn strip_package_specification(node: PackageSpecification) -> PackageSpecification {
    PackageSpecification {
        modules: node
            .modules
            .into_iter()
            .map(|(name, module)| (name, strip_module_specification(module)))
            .collect(),
    }
}

fn strip_package_definition(node: PackageDefinition) -> PackageDefinition {
    PackageDefinition {
        modules: node
            .modules
            .into_iter()
            .map(|(name, module)| {
                (
                    name,
                    strip_access_controlled(module, strip_module_definition),
                )
            })
            .collect(),
    }
}

fn strip_distribution(node: Distribution) -> Distribution {
    match node {
        Distribution::Library(content) => Distribution::Library(LibraryContent {
            package_name: content.package_name,
            dependencies: strip_dependencies(content.dependencies),
            def: strip_package_definition(content.def),
        }),
        Distribution::Specs(content) => Distribution::Specs(SpecsContent {
            package_name: content.package_name,
            dependencies: strip_dependencies(content.dependencies),
            spec: strip_package_specification(content.spec),
        }),
        Distribution::Application(content) => Distribution::Application(ApplicationContent {
            package_name: content.package_name,
            dependencies: strip_definition_dependencies(content.dependencies),
            def: strip_package_definition(content.def),
            entry_points: content.entry_points,
        }),
    }
}

fn strip_dependencies(
    dependencies: morphir_core::ir::v4::Dependencies,
) -> morphir_core::ir::v4::Dependencies {
    dependencies
        .into_iter()
        .map(|(name, specification)| (name, strip_package_specification(specification)))
        .collect()
}

fn strip_definition_dependencies(
    dependencies: morphir_core::ir::v4::DefinitionDependencies,
) -> morphir_core::ir::v4::DefinitionDependencies {
    dependencies
        .into_iter()
        .map(|(name, definition)| (name, strip_package_definition(definition)))
        .collect()
}
