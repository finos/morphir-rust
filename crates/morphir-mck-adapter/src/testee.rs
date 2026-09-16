//! The operations that answer each request in the kit's adapter protocol.
//!
//! `decode` reads one node in one profile at one IR version and answers with the canonical
//! spelling of what it read, the node kind a `rejected expect=<Kind>` fence names, and the
//! legacy spellings accepted on the way (`protocol.schema.json`'s `DecodeSuccess`). The
//! spellings themselves are morphir-core's; nothing here decides what a member is called.

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use morphir_core::ir::classic;
use morphir_core::ir::v4::{
    AccessControlled, ApplicationContent, ConstructorArg, ConstructorArgSpec,
    ConstructorDefinition, ConstructorSpecification, Distribution, Documented, Field,
    FormatVersion, IRFile, InputTypeEntry, LetBinding, LibraryContent, Literal, ModuleDefinition,
    ModuleSpecification, PackageDefinition, PackageSpecification, Pattern, PatternCase,
    RecordFieldEntry, SpecsContent, SpellingMode, Type, TypeAttributes, TypeDefinition,
    TypeEncoding, TypeSpecification, Value, ValueAttributes, ValueBody, ValueDefinition,
    ValueSpecification, with_spelling_mode, with_type_encoding,
};
use morphir_core::ir::{Diagnostic, DiagnosticCode, DiagnosticError, Warning};
use morphir_core::migration::{self, MigrationContext, MigrationDiagnostic};
use morphir_core::naming::{FQName, Name, Path};

use crate::protocol::{DecodeRequest, DecodeResponse, NodeKind, PathMode, Profile};

/// How many nested containers a document may carry.
///
/// A reader that follows arbitrary nesting turns a small input into a deep recursion, so the
/// profile puts a ceiling on it and reports `nesting_too_deep` rather than failing some other
/// way at some other depth.
const MAX_DEPTH: usize = 512;

/// Reads one node and answers with its canonical spelling or the diagnostic that refused it.
pub fn decode(req: &DecodeRequest) -> DecodeResponse {
    match read(req) {
        Ok((node, warnings)) => {
            let node = if req.strip { node.stripped() } else { node };
            match node.write() {
                Ok(text) => DecodeResponse::Ok {
                    kind: node.kind().to_string(),
                    canonical: BTreeMap::from([("json".to_string(), format!("{text}\n"))]),
                    warnings,
                },
                Err(diagnostic) => DecodeResponse::Err { diagnostic },
            }
        }
        Err(diagnostic) => DecodeResponse::Err { diagnostic },
    }
}

fn read(req: &DecodeRequest) -> Result<(Node, Vec<Warning>), Diagnostic> {
    if req.profile != Profile::Json {
        return Err(Diagnostic::syntax(
            DiagnosticCode::InvalidYaml,
            "/",
            "this binding reads the json profile only",
        ));
    }

    // A repeated member and a document nested past the ceiling are properties of the text, not
    // of any node, so they are settled before the text becomes a value: `serde_json::Value`
    // folds a repeated member onto the last one written and would hide it.
    if let Some(diagnostic) = check_duplicates(&req.input) {
        return Err(diagnostic);
    }

    match req.version {
        4 => read_v4(req),
        3 => read_v3(req).map(|node| (node, Vec::new())),
        other => Err(Diagnostic::normalization(
            DiagnosticCode::InvalidType,
            "/",
            format!("this binding reads IR versions 3 and 4, not {other}"),
        )),
    }
}

// =============================================================================
// Version 4
// =============================================================================

fn read_v4(req: &DecodeRequest) -> Result<(Node, Vec<Warning>), Diagnostic> {
    let value = parse_json(&req.input)?;
    let mode = match req.path {
        PathMode::Current => SpellingMode::Current,
        PathMode::Pinned => SpellingMode::Pinned,
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

/// Parses the input as JSON, with the ceiling this reader states rather than serde_json's own.
///
/// `disable_recursion_limit` needs the `unbounded_depth` feature; without it serde_json stops at
/// its own default of 128, which would report `invalid_json` for a document the profile admits.
/// The depth that matters is [`MAX_DEPTH`], and [`check_duplicates`] has already enforced it.
fn parse_json(text: &str) -> Result<Json, Diagnostic> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    deserializer.disable_recursion_limit();
    let value = Json::deserialize(&mut deserializer).map_err(invalid_json)?;
    deserializer.end().map_err(invalid_json)?;
    Ok(value)
}

fn invalid_json(error: serde_json::Error) -> Diagnostic {
    let mut diagnostic = Diagnostic::syntax(DiagnosticCode::InvalidJson, "/", error.to_string());
    diagnostic.line = u32::try_from(error.line()).ok();
    diagnostic.column = u32::try_from(error.column()).ok();
    diagnostic
}

// =============================================================================
// Duplicate members and nesting
// =============================================================================

/// Reports the second occurrence of a repeated object member, or a document nested past
/// [`MAX_DEPTH`].
///
/// `serde_json::Value` cannot answer either question: it keeps one entry per key, so the second
/// `"a"` in `{"a":1,"a":2}` is gone by the time a value exists, and its own recursion limit
/// fails before this reader's ceiling is reached. The probe below walks the token stream
/// instead, carrying the JSON pointer of where it is, and stops at the first thing it finds.
pub fn check_duplicates(text: &str) -> Option<Diagnostic> {
    let mut deserializer = serde_json::Deserializer::from_str(text);
    deserializer.disable_recursion_limit();
    match (Probe {
        cursor: String::new(),
        depth: 0,
    })
    .deserialize(&mut deserializer)
    {
        Ok(()) => None,
        // A syntax error is not this probe's to report: `parse_json` reports it with the line
        // and column serde_json gives, as `invalid_json`.
        Err(error) => Diagnostic::from_serde_error(&error),
    }
}

/// One position in the token stream: the JSON pointer of the value about to be read and how
/// many containers are already open around it.
///
/// This is a [`DeserializeSeed`] rather than a [`Deserialize`] because the cursor and the depth
/// have to travel *into* each member, and a `Deserialize` impl is handed nothing but the
/// deserializer.
struct Probe {
    cursor: String,
    depth: usize,
}

/// serde_json's `arbitrary_precision` feature carries a number through `deserialize_any` as a
/// one-member map under this reserved key, so the probe would otherwise count every number as a
/// container and read its lexeme as a member name.
const NUMBER_TOKEN: &str = "$serde_json::private::Number";

impl<'de> DeserializeSeed<'de> for Probe {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<(), D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Probe {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value")
    }

    fn visit_map<A>(self, mut map: A) -> Result<(), A::Error>
    where
        A: MapAccess<'de>,
    {
        let depth = self.enter::<A::Error>()?;
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if key == NUMBER_TOKEN {
                map.next_value::<serde::de::IgnoredAny>()?;
                continue;
            }
            let cursor = format!("{}/{}", self.cursor, escape(&key));
            if !seen.insert(key.clone()) {
                return Err(carry(Diagnostic::syntax(
                    DiagnosticCode::DuplicateMember,
                    cursor,
                    format!("member {key} is written more than once"),
                )));
            }
            map.next_value_seed(Probe { cursor, depth })?;
        }
        Ok(())
    }

    fn visit_seq<A>(self, mut seq: A) -> Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        let depth = self.enter::<A::Error>()?;
        let mut index = 0usize;
        while seq
            .next_element_seed(Probe {
                cursor: format!("{}/{index}", self.cursor),
                depth,
            })?
            .is_some()
        {
            index += 1;
        }
        Ok(())
    }

    fn visit_bool<E: serde::de::Error>(self, _value: bool) -> Result<(), E> {
        Ok(())
    }

    fn visit_i64<E: serde::de::Error>(self, _value: i64) -> Result<(), E> {
        Ok(())
    }

    fn visit_u64<E: serde::de::Error>(self, _value: u64) -> Result<(), E> {
        Ok(())
    }

    fn visit_f64<E: serde::de::Error>(self, _value: f64) -> Result<(), E> {
        Ok(())
    }

    fn visit_str<E: serde::de::Error>(self, _value: &str) -> Result<(), E> {
        Ok(())
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
        Ok(())
    }
}

impl Probe {
    /// Opens the container at this position, refusing the one that crosses the ceiling.
    fn enter<E: serde::de::Error>(&self) -> Result<usize, E> {
        let depth = self.depth + 1;
        if depth > MAX_DEPTH {
            return Err(carry(Diagnostic::syntax(
                DiagnosticCode::NestingTooDeep,
                &self.cursor,
                format!("more than {MAX_DEPTH} nested containers"),
            )));
        }
        Ok(depth)
    }
}

fn carry<E: serde::de::Error>(diagnostic: Diagnostic) -> E {
    E::custom(DiagnosticError(diagnostic))
}

/// A member name as a JSON pointer reference token (RFC 6901).
fn escape(member: &str) -> String {
    member.replace('~', "~0").replace('/', "~1")
}

// =============================================================================
// Version 3
// =============================================================================

/// The classic value attribute: a `{}` before type inference has run, an inferred type after.
type ClassicAnnotation = classic::Attrs<classic::Type<classic::Attrs>>;

fn read_v3(req: &DecodeRequest) -> Result<Node, Diagnostic> {
    fn of<T: for<'de> Deserialize<'de>, U>(
        text: &str,
        migrate: impl FnOnce(&T, &mut MigrationContext) -> Result<U, MigrationDiagnostic>,
        wrap: fn(U) -> Node,
    ) -> Result<Node, Diagnostic> {
        let classic: T = serde_json::from_str(text).map_err(|error| recover(&error))?;
        let mut context = MigrationContext::default();
        migrate(&classic, &mut context)
            .map(wrap)
            .map_err(migration_failed)
    }

    fn pure<T: for<'de> Deserialize<'de>, U>(
        text: &str,
        migrate: impl FnOnce(&T) -> U,
        wrap: fn(U) -> Node,
    ) -> Result<Node, Diagnostic> {
        let classic: T = serde_json::from_str(text).map_err(|error| recover(&error))?;
        Ok(wrap(migrate(&classic)))
    }

    let text = &req.input;
    match req.node {
        NodeKind::Name => pure::<classic::Name, _>(text, migration::migrate_name, Node::Name),
        NodeKind::Path => pure::<classic::Path, _>(text, migration::migrate_path, Node::Path),
        NodeKind::FQName => {
            pure::<classic::FQName, _>(text, migration::migrate_fqname, Node::FQName)
        }
        NodeKind::Literal => {
            pure::<classic::Literal, _>(text, migration::migrate_literal, Node::Literal)
        }
        NodeKind::Type => {
            of::<classic::Type<classic::Attrs>, _>(text, migration::migrate_type, Node::Type)
        }
        NodeKind::Pattern => of::<classic::Pattern<ClassicAnnotation>, _>(
            text,
            migration::migrate_pattern,
            Node::Pattern,
        ),
        NodeKind::Value => of::<classic::Value<classic::Attrs, ClassicAnnotation>, _>(
            text,
            migration::migrate_value,
            Node::Value,
        ),
        NodeKind::ValueDefinition => {
            of::<classic::ValueDefinition<classic::Attrs, ClassicAnnotation>, _>(
                text,
                migration::migrate_value_definition,
                Node::ValueDefinition,
            )
        }
        NodeKind::ModuleDefinition => {
            of::<classic::ModuleDefinition<classic::Attrs, ClassicAnnotation>, _>(
                text,
                migration::migrate_module_definition,
                Node::ModuleDefinition,
            )
        }
        NodeKind::IRFile | NodeKind::Distribution => {
            let classic: classic::Distribution =
                serde_json::from_str(text).map_err(|error| recover(&error))?;
            migration::migrate_distribution(&classic, Default::default())
                .map(|migrated| Node::IRFile(migrated.value))
                .map_err(migration_failed)
        }
        // Classic has no spelling for these on their own, so there is nothing to migrate from.
        NodeKind::FormatVersion
        | NodeKind::TypeSpecification
        | NodeKind::TypeDefinition
        | NodeKind::ValueSpecification
        | NodeKind::AccessControlledTypeDefinition
        | NodeKind::AccessControlledValueDefinition
        | NodeKind::ModuleSpecification => Err(Diagnostic::normalization(
            DiagnosticCode::UnknownNode,
            "/",
            format!(
                "{:?} is not a node a version 3 document spells on its own",
                req.node
            ),
        )),
    }
}

fn migration_failed(diagnostic: MigrationDiagnostic) -> Diagnostic {
    Diagnostic::normalization(DiagnosticCode::InvalidType, "/", diagnostic.message)
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
            // Names, paths and the format version carry no attributes at all.
            other => other,
        }
    }

    /// The canonical JSON spelling of this node, in the compact type encoding.
    fn write(&self) -> Result<String, Diagnostic> {
        fn text<T: Serialize>(node: &T) -> Result<String, Diagnostic> {
            let value = with_type_encoding(TypeEncoding::Compact, || serde_json::to_value(node))
                .map_err(|error| {
                    Diagnostic::normalization(DiagnosticCode::InvalidType, "/", error.to_string())
                })?;
            Ok(write_canonical(&value))
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
        }
    }
}

/// Writes a value in the JSON profile's canonical text form.
///
/// The profile's writer is not `serde_json::to_string`: a non-empty object is padded inside its
/// braces and its members separated by `, `, while an array is not padded, and a number keeps
/// the lexeme it was written with. The driver compares canonicals as strings (kit README, "What
/// the driver does with a case"), so this is part of the contract rather than a style.
fn write_canonical(value: &Json) -> String {
    match value {
        Json::Null => "null".to_string(),
        Json::Bool(true) => "true".to_string(),
        Json::Bool(false) => "false".to_string(),
        // `arbitrary_precision` keeps the lexeme, so `Display` writes the number back as it came.
        Json::Number(number) => number.to_string(),
        Json::String(_) => serde_json::to_string(value).expect("a string always serializes"),
        Json::Array(elements) if elements.is_empty() => "[]".to_string(),
        Json::Array(elements) => format!(
            "[{}]",
            elements
                .iter()
                .map(write_canonical)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Json::Object(members) if members.is_empty() => "{}".to_string(),
        Json::Object(members) => format!(
            "{{ {} }}",
            members
                .iter()
                .map(|(key, member)| format!(
                    "{}: {}",
                    serde_json::to_string(key).expect("a member name always serializes"),
                    write_canonical(member)
                ))
                .collect::<Vec<_>>()
                .join(", ")
        ),
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

fn strip_type_specification(node: TypeSpecification) -> TypeSpecification {
    match node {
        TypeSpecification::TypeAliasSpecification {
            type_params,
            type_expr,
        } => TypeSpecification::TypeAliasSpecification {
            type_params,
            type_expr: strip_type(type_expr),
        },
        TypeSpecification::OpaqueTypeSpecification { type_params } => {
            TypeSpecification::OpaqueTypeSpecification { type_params }
        }
        TypeSpecification::CustomTypeSpecification {
            type_params,
            constructors,
        } => TypeSpecification::CustomTypeSpecification {
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
            type_params,
            base_type,
            from_base_type,
            to_base_type,
        } => TypeSpecification::DerivedTypeSpecification {
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
            incompleteness,
            partial_type_expr: partial_type_expr.map(strip_type),
        },
    }
}

fn strip_value_specification(node: ValueSpecification) -> ValueSpecification {
    ValueSpecification {
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
            .map(|(name, entry)| {
                (
                    name,
                    InputTypeEntry {
                        type_attributes: None,
                        input_type: strip_type(entry.input_type),
                    },
                )
            })
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
            ValueBody::Incomplete { incompleteness } => ValueBody::Incomplete { incompleteness },
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
            dependencies: strip_dependencies(content.dependencies),
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
