//! Cursor-carrying decoders for the declaration layer of a version 4 document.
//!
//! Type expressions, literals, patterns and value expressions are decoded by
//! [`super::serde_tagged`]; everything a document wraps around them — access control,
//! documentation, type and value definitions and specifications, modules, packages,
//! distributions and the file itself — is decoded here, by the same rules:
//!
//! - the node is read as a [`JsonValue`] first, because `serde_json`'s own value type is what
//!   implements the arbitrary-precision protocol a document literal depends on;
//! - every failure is a [`Diagnostic`] with the kit's code and a JSON pointer, carried through
//!   [`DiagnosticError`] so it survives serde;
//! - a wrapper's members go through [`wrapper_members`], so a spelling the window of decision
//!   0006 keeps alive warns at the member the author wrote and any other member is refused
//!   there.
//!
//! The spellings are the ones the Morphir Compatibility Kit's `definitions-*` and
//! `distributions-*` cases pin.

use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use super::access::{Access, AccessControlled};
use super::attributes::ValueAttributes;
use super::distribution::{
    ApplicationContent, Dependencies, Distribution, EntryPoint, EntryPointKind, EntryPoints,
    LibraryContent, SpecsContent,
};
use super::legacy::accept_legacy_form;
use super::module::{Documentation, Documented, ModuleDefinition, ModuleSpecification};
use super::package::{PackageDefinition, PackageSpecification};
use super::serde_tagged::{
    Members, carry, decode_fqname, decode_name, decode_type, decode_value, invalid_type,
    member_cursor, required, unknown_node_at, wrapper_members,
};
use super::types::{
    ConstructorArg, ConstructorArgSpec, ConstructorDefinition, ConstructorSpecification,
    Incompleteness, Type, TypeDefinition, TypeSpecification,
};
use super::value::{
    ExternalBinding, HoleReason, InputTypeEntry, NativeHint, NativeInfo, ValueBody,
    ValueDefinition, ValueSpecification,
};
use super::{FormatVersion, IRFile};
use crate::format_version::{NormalizedFormatVersion, ScalarValue, SupportTable};
use crate::ir::{Diagnostic, DiagnosticCode, DiagnosticError};
use crate::naming::{Name, PackageName};

/// Reads the node as JSON and hands it to a cursor-carrying decoder, starting at the root.
///
/// A `Deserialize` impl gets no way to learn its own path, so the cursor a nested node reports
/// is relative to wherever the decode was entered. A whole document entered at [`IRFile`] is
/// therefore located absolutely; a fragment decoded on its own is located within itself.
pub(super) fn deserialize_with<'de, D, T>(
    deserializer: D,
    decode: fn(&JsonValue, &str) -> Result<T, Diagnostic>,
) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = JsonValue::deserialize(deserializer)?;
    decode(&value, "").map_err(carry)
}

fn members_of<'a>(
    value: &'a JsonValue,
    cursor: &str,
    what: &str,
) -> Result<&'a serde_json::Map<String, JsonValue>, Diagnostic> {
    value
        .as_object()
        .ok_or_else(|| invalid_type(cursor, format!("{what} must be an object")))
}

fn missing(cursor: &str, member: &str) -> Diagnostic {
    Diagnostic::normalization(
        DiagnosticCode::MissingMember,
        cursor,
        format!("missing member {member}"),
    )
}

fn unknown_member(cursor: &str, member: &str) -> Diagnostic {
    Diagnostic::normalization(
        DiagnosticCode::UnknownMember,
        cursor,
        format!("unexpected member {member}"),
    )
}

/// The single member of a wrapper object, or a diagnostic saying why there is not exactly one.
fn single_member<'a>(
    value: &'a JsonValue,
    cursor: &str,
    what: &str,
) -> Result<(&'a String, &'a JsonValue), Diagnostic> {
    let members = members_of(value, cursor, what)?;
    let mut entries = members.iter();
    let entry = entries
        .next()
        .ok_or_else(|| unknown_node_at(cursor, format!("an empty object is not {what}")))?;
    if entries.next().is_some() {
        return Err(unknown_node_at(
            cursor,
            format!("an object with more than one member is not {what}"),
        ));
    }
    Ok(entry)
}

// =============================================================================
// Documentation and access control
// =============================================================================

/// Decodes documentation: one line as a string, or several as an array of strings.
pub(super) fn decode_documentation(
    value: &JsonValue,
    cursor: &str,
) -> Result<Documentation, Diagnostic> {
    match value {
        JsonValue::String(line) => Ok(Documentation::new([line.clone()])),
        JsonValue::Array(lines) => lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                line.as_str().map(str::to_owned).ok_or_else(|| {
                    invalid_type(
                        &format!("{cursor}/{index}"),
                        "a documentation line is a string",
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Documentation::new),
        _ => Err(invalid_type(
            cursor,
            "documentation is a string or an array of strings",
        )),
    }
}

/// Decodes an access-controlled node.
///
/// Canonical is the access level as the variant tag: `{ "Public": <node> }`. Accepted beside it,
/// silently: the access level flattened next to the node's own members
/// (`{ "access": "Public", … }`), the same with the node under `value`, and the `pub`/`priv`
/// shorthands (definitions-0001, 0017, 0018, 0019).
pub(super) fn decode_access_controlled<T>(
    value: &JsonValue,
    cursor: &str,
    inner: impl FnOnce(&JsonValue, &str) -> Result<T, Diagnostic>,
) -> Result<AccessControlled<T>, Diagnostic> {
    let members = members_of(value, cursor, "an access-controlled node")?;

    if let Some(written) = members.get("access") {
        let access_cursor = format!("{cursor}/access");
        let access = decode_access(written, &access_cursor)?;
        let mut rest = members.clone();
        rest.remove("access");
        let (payload, payload_cursor) = if rest.len() == 1 && rest.contains_key("value") {
            (rest["value"].clone(), format!("{cursor}/value"))
        } else {
            (JsonValue::Object(rest), cursor.to_owned())
        };
        return Ok(AccessControlled {
            access,
            value: inner(&payload, &payload_cursor)?,
        });
    }

    let (tag, payload) = single_member(value, cursor, "an access-controlled node")?;
    let access = Access::from_tag(tag).ok_or_else(|| {
        Diagnostic::normalization(
            DiagnosticCode::InvalidAccess,
            cursor,
            format!("{tag} does not name an access level"),
        )
    })?;
    Ok(AccessControlled {
        access,
        value: inner(payload, &format!("{cursor}/{tag}"))?,
    })
}

fn decode_access(value: &JsonValue, cursor: &str) -> Result<Access, Diagnostic> {
    value.as_str().and_then(Access::from_tag).ok_or_else(|| {
        Diagnostic::normalization(
            DiagnosticCode::InvalidAccess,
            cursor,
            "access is Public or Private",
        )
    })
}

/// Decodes a node that may carry documentation.
///
/// Decision 0010 flattens `doc` beside the node's own members and places it first. The nested
/// wrapper that put the node under `value` decodes for the window of decision 0006, with a
/// `legacy_spelling` warning at the `value` member (definitions-0006, 0010, 0017, 0018, 0019).
pub(super) fn decode_documented<T>(
    value: &JsonValue,
    cursor: &str,
    inner: impl FnOnce(&JsonValue, &str) -> Result<T, Diagnostic>,
) -> Result<Documented<T>, Diagnostic> {
    let members = members_of(value, cursor, "a documented node")?;

    let doc = match members.get("doc") {
        Some(written) => Some(decode_documentation(written, &format!("{cursor}/doc"))?),
        None => None,
    };
    let mut rest = members.clone();
    rest.remove("doc");

    let (payload, payload_cursor) = if rest.len() == 1 && rest.contains_key("value") {
        let at = format!("{cursor}/value");
        accept_legacy_form(&at, &at, "value")?;
        (rest["value"].clone(), at)
    } else {
        (JsonValue::Object(rest), cursor.to_owned())
    };

    Ok(Documented {
        doc,
        value: inner(&payload, &payload_cursor)?,
    })
}

// =============================================================================
// Type definitions and specifications
// =============================================================================

fn decode_type_params(members: &Members<'_>, cursor: &str) -> Result<Vec<Name>, Diagnostic> {
    let Some(member) = members.get("typeParams") else {
        return Ok(Vec::new());
    };
    let at = member_cursor(members, "typeParams", cursor);
    let items = member
        .value
        .as_array()
        .ok_or_else(|| invalid_type(&at, "typeParams is an array of type variable names"))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| decode_name(item, &format!("{at}/{index}")))
        .collect()
}

fn optional_type(
    members: &Members<'_>,
    name: &str,
    cursor: &str,
) -> Result<Option<Type>, Diagnostic> {
    match members.get(name) {
        None => Ok(None),
        Some(member) if member.value.is_null() => Ok(None),
        Some(member) => decode_type(member.value, &member_cursor(members, name, cursor)).map(Some),
    }
}

fn decode_member_type(members: &Members<'_>, name: &str, cursor: &str) -> Result<Type, Diagnostic> {
    decode_type(
        required(members, name, cursor)?,
        &member_cursor(members, name, cursor),
    )
}

fn decode_member_fqname(
    members: &Members<'_>,
    name: &str,
    cursor: &str,
) -> Result<crate::naming::FQName, Diagnostic> {
    decode_fqname(
        required(members, name, cursor)?,
        &member_cursor(members, name, cursor),
    )
}

/// Decodes a constructor map: `{ "just": [["value", "a"]], "nothing": [] }`.
fn decode_constructor_map<T>(
    value: &JsonValue,
    cursor: &str,
    build: impl Fn(Name, Vec<(Name, Type)>) -> T,
) -> Result<Vec<T>, Diagnostic> {
    let constructors = members_of(value, cursor, "constructors")?;
    constructors
        .iter()
        .map(|(name, args)| {
            let at = format!("{cursor}/{name}");
            let name = decode_name(&JsonValue::String(name.clone()), &at)?;
            let args = args
                .as_array()
                .ok_or_else(|| invalid_type(&at, "a constructor's arguments are an array"))?
                .iter()
                .enumerate()
                .map(|(index, pair)| {
                    let at = format!("{at}/{index}");
                    let pair = pair
                        .as_array()
                        .filter(|pair| pair.len() == 2)
                        .ok_or_else(|| {
                            invalid_type(&at, "a constructor argument is a [name, type] pair")
                        })?;
                    Ok((
                        decode_name(&pair[0], &format!("{at}/0"))?,
                        decode_type(&pair[1], &format!("{at}/1"))?,
                    ))
                })
                .collect::<Result<Vec<_>, Diagnostic>>()?;
            Ok(build(name, args))
        })
        .collect()
}

/// Decodes a type definition wrapper (definitions-0001, 0003, 0014, 0015).
pub(super) fn decode_type_definition(
    value: &JsonValue,
    cursor: &str,
) -> Result<TypeDefinition, Diagnostic> {
    let (tag, payload) = single_member(value, cursor, "a type definition")?;
    let at = format!("{cursor}/{tag}");
    match tag.as_str() {
        "TypeAliasDefinition" => {
            let members = wrapper_members(tag, payload, &at, &["typeParams", "typeExp"])?;
            Ok(TypeDefinition::TypeAliasDefinition {
                type_params: decode_type_params(&members, &at)?,
                type_expr: decode_member_type(&members, "typeExp", &at)?,
            })
        }
        "CustomTypeDefinition" => {
            let members =
                wrapper_members(tag, payload, &at, &["typeParams", "access", "constructors"])?;
            let type_params = decode_type_params(&members, &at)?;
            let access = decode_access(
                required(&members, "access", &at)?,
                &member_cursor(&members, "access", &at),
            )?;
            let constructors = decode_constructor_map(
                required(&members, "constructors", &at)?,
                &member_cursor(&members, "constructors", &at),
                |name, args| ConstructorDefinition {
                    name,
                    args: args
                        .into_iter()
                        .map(|(name, arg_type)| ConstructorArg { name, arg_type })
                        .collect(),
                },
            )?;
            Ok(TypeDefinition::CustomTypeDefinition {
                type_params,
                constructors: AccessControlled {
                    access,
                    value: constructors,
                },
            })
        }
        "IncompleteTypeDefinition" => {
            let members = wrapper_members(
                tag,
                payload,
                &at,
                &["typeParams", "incompleteness", "partialTypeExp"],
            )?;
            Ok(TypeDefinition::IncompleteTypeDefinition {
                type_params: decode_type_params(&members, &at)?,
                incompleteness: decode_incompleteness(
                    required(&members, "incompleteness", &at)?,
                    &member_cursor(&members, "incompleteness", &at),
                )?,
                partial_type_expr: optional_type(&members, "partialTypeExp", &at)?,
            })
        }
        other => Err(unknown_node_at(
            cursor,
            format!("{other} is not a type definition"),
        )),
    }
}

/// Decodes a type specification wrapper (definitions-0002, 0011, 0012, 0013).
pub(super) fn decode_type_specification(
    value: &JsonValue,
    cursor: &str,
) -> Result<TypeSpecification, Diagnostic> {
    // An opaque specification has no payload at all, so the pre-decision pair spelling
    // `["OpaqueTypeSpecification", []]` is unambiguous and is read as the empty wrapper.
    if let JsonValue::Array(items) = value
        && let Some(JsonValue::String(tag)) = items.first()
        && tag == "OpaqueTypeSpecification"
    {
        return Ok(TypeSpecification::OpaqueTypeSpecification {
            type_params: Vec::new(),
        });
    }

    let (tag, payload) = single_member(value, cursor, "a type specification")?;
    let at = format!("{cursor}/{tag}");
    match tag.as_str() {
        "OpaqueTypeSpecification" => {
            let members = wrapper_members(tag, payload, &at, &["typeParams"])?;
            Ok(TypeSpecification::OpaqueTypeSpecification {
                type_params: decode_type_params(&members, &at)?,
            })
        }
        "TypeAliasSpecification" => {
            let members = wrapper_members(tag, payload, &at, &["typeParams", "typeExp"])?;
            Ok(TypeSpecification::TypeAliasSpecification {
                type_params: decode_type_params(&members, &at)?,
                type_expr: decode_member_type(&members, "typeExp", &at)?,
            })
        }
        "CustomTypeSpecification" => {
            let members = wrapper_members(tag, payload, &at, &["typeParams", "constructors"])?;
            let type_params = decode_type_params(&members, &at)?;
            let constructors = decode_constructor_map(
                required(&members, "constructors", &at)?,
                &member_cursor(&members, "constructors", &at),
                |name, args| ConstructorSpecification {
                    name,
                    args: args
                        .into_iter()
                        .map(|(name, arg_type)| ConstructorArgSpec { name, arg_type })
                        .collect(),
                },
            )?;
            Ok(TypeSpecification::CustomTypeSpecification {
                type_params,
                constructors,
            })
        }
        "DerivedTypeSpecification" => {
            let members = wrapper_members(
                tag,
                payload,
                &at,
                &["typeParams", "baseType", "fromBaseType", "toBaseType"],
            )?;
            Ok(TypeSpecification::DerivedTypeSpecification {
                type_params: decode_type_params(&members, &at)?,
                base_type: decode_member_type(&members, "baseType", &at)?,
                from_base_type: decode_member_fqname(&members, "fromBaseType", &at)?,
                to_base_type: decode_member_fqname(&members, "toBaseType", &at)?,
            })
        }
        other => Err(unknown_node_at(
            cursor,
            format!("{other} is not a type specification"),
        )),
    }
}

/// Decodes an incompleteness (definitions-0014, 0015, 0016, 0024).
///
/// A hole says why it is one under `reason` and may keep the type expression the author had as
/// `partialBody`; a draft is deliberately unfinished and takes an empty payload.
pub(super) fn decode_incompleteness(
    value: &JsonValue,
    cursor: &str,
) -> Result<Incompleteness, Diagnostic> {
    let (tag, payload) = single_member(value, cursor, "an incompleteness")?;
    let at = format!("{cursor}/{tag}");
    match tag.as_str() {
        "Draft" => Ok(Incompleteness::Draft),
        "Hole" => {
            let members = wrapper_members(tag, payload, &at, &["reason", "partialBody"])?;
            Ok(Incompleteness::Hole {
                reason: decode_hole_reason(
                    required(&members, "reason", &at)?,
                    &member_cursor(&members, "reason", &at),
                )?,
                partial_body: optional_type(&members, "partialBody", &at)?,
            })
        }
        other => Err(unknown_node_at(
            cursor,
            format!("{other} is not an incompleteness"),
        )),
    }
}

/// Decodes a hole's reason (definitions-0014, 0016, 0026). `Draft` is an incompleteness kind, not
/// a reason, so it is an unknown node here.
pub(super) fn decode_hole_reason(
    value: &JsonValue,
    cursor: &str,
) -> Result<HoleReason, Diagnostic> {
    let (tag, payload) = single_member(value, cursor, "a hole reason")?;
    let at = format!("{cursor}/{tag}");
    match tag.as_str() {
        "UnresolvedReference" => {
            let members = wrapper_members(tag, payload, &at, &["target"])?;
            Ok(HoleReason::UnresolvedReference {
                target: decode_member_fqname(&members, "target", &at)?,
            })
        }
        "DeletedDuringRefactor" => {
            let members = wrapper_members(tag, payload, &at, &["tx-id"])?;
            Ok(HoleReason::DeletedDuringRefactor {
                tx_id: decode_text(&members, "tx-id", &at)?,
            })
        }
        "TypeMismatch" => {
            let members = wrapper_members(tag, payload, &at, &["expected", "found"])?;
            Ok(HoleReason::TypeMismatch {
                expected: decode_text(&members, "expected", &at)?,
                found: decode_text(&members, "found", &at)?,
            })
        }
        other => Err(unknown_node_at(
            cursor,
            format!("{other} is not a hole reason"),
        )),
    }
}

/// Decodes a native definition's info (definitions-0009, 0030).
pub(super) fn decode_native_info(
    value: &JsonValue,
    cursor: &str,
) -> Result<NativeInfo, Diagnostic> {
    let members = wrapper_members("NativeInfo", value, cursor, &["hint", "description"])?;
    let hint = decode_native_hint(
        required(&members, "hint", cursor)?,
        &member_cursor(&members, "hint", cursor),
    )?;
    let description = match members.get("description") {
        None => None,
        Some(member) => Some(text_of(
            member.value,
            &member_cursor(&members, "description", cursor),
        )?),
    };
    Ok(NativeInfo { hint, description })
}

/// Decodes a native operation's category hint (definitions-0009, 0030).
pub(super) fn decode_native_hint(
    value: &JsonValue,
    cursor: &str,
) -> Result<NativeHint, Diagnostic> {
    let (tag, payload) = single_member(value, cursor, "a native hint")?;
    let at = format!("{cursor}/{tag}");
    let hint = match tag.as_str() {
        "Arithmetic" => NativeHint::Arithmetic,
        "Comparison" => NativeHint::Comparison,
        "StringOp" => NativeHint::StringOp,
        "CollectionOp" => NativeHint::CollectionOp,
        "PlatformSpecific" => {
            let members = wrapper_members(tag, payload, &at, &["platform"])?;
            return Ok(NativeHint::PlatformSpecific {
                platform: decode_text(&members, "platform", &at)?,
            });
        }
        other => {
            return Err(unknown_node_at(
                cursor,
                format!("{other} is not a native hint"),
            ));
        }
    };
    // The four nullary hints take an empty payload; anything inside it is unknown.
    wrapper_members(tag, payload, &at, &[])?;
    Ok(hint)
}

// =============================================================================
// Value specifications and definitions
// =============================================================================

/// Decodes a value specification (definitions-0004, 0010).
///
/// `inputs` is an object keyed by parameter name; an array of `[name, type]` pairs is accepted
/// beside it, which is what a reader meets from a writer that kept the parameters ordered as a
/// list.
pub(super) fn decode_value_specification(
    value: &JsonValue,
    cursor: &str,
) -> Result<ValueSpecification, Diagnostic> {
    let members = wrapper_members("ValueSpecification", value, cursor, &["inputs", "output"])?;
    let inputs = match members.get("inputs") {
        None => IndexMap::new(),
        Some(member) => decode_input_map(
            member.value,
            &member_cursor(&members, "inputs", cursor),
            decode_type,
        )?,
    };
    Ok(ValueSpecification {
        inputs,
        output: decode_member_type(&members, "output", cursor)?,
    })
}

/// Decodes a map of named types: an object keyed by name, or an array of `[name, type]` pairs.
fn decode_input_map<T>(
    value: &JsonValue,
    cursor: &str,
    decode_entry: fn(&JsonValue, &str) -> Result<T, Diagnostic>,
) -> Result<IndexMap<String, T>, Diagnostic> {
    match value {
        JsonValue::Object(entries) => entries
            .iter()
            .map(|(name, written)| {
                let at = format!("{cursor}/{name}");
                let name = decode_name(&JsonValue::String(name.clone()), &at)?;
                Ok((name.to_canonical_string(), decode_entry(written, &at)?))
            })
            .collect(),
        JsonValue::Array(pairs) => pairs
            .iter()
            .enumerate()
            .map(|(index, pair)| {
                let at = format!("{cursor}/{index}");
                let pair = pair
                    .as_array()
                    .filter(|pair| pair.len() == 2)
                    .ok_or_else(|| invalid_type(&at, "an input is a [name, type] pair"))?;
                let name = decode_name(&pair[0], &format!("{at}/0"))?;
                Ok((
                    name.to_canonical_string(),
                    decode_entry(&pair[1], &format!("{at}/1"))?,
                ))
            })
            .collect(),
        _ => Err(invalid_type(
            cursor,
            "inputs is an object keyed by name, or an array of [name, type] pairs",
        )),
    }
}

/// One entry of a definition's `inputTypes`: a bare type, or the expanded spelling carrying the
/// parameter's own attributes beside it.
fn decode_input_type_entry(value: &JsonValue, cursor: &str) -> Result<InputTypeEntry, Diagnostic> {
    if let JsonValue::Object(members) = value
        && members.contains_key("type")
    {
        let type_attributes = match members.get("typeAttributes") {
            None => None,
            Some(written) => Some(
                serde_json::from_value::<ValueAttributes>(written.clone()).map_err(|error| {
                    invalid_type(&format!("{cursor}/typeAttributes"), error.to_string())
                })?,
            ),
        };
        return Ok(InputTypeEntry {
            type_attributes,
            input_type: decode_type(&members["type"], &format!("{cursor}/type"))?,
        });
    }
    Ok(InputTypeEntry {
        type_attributes: None,
        input_type: decode_type(value, cursor)?,
    })
}

/// The four value definition bodies (definitions-0005, 0007, 0008, 0009, 0016).
const BODY_TAGS: &[&str] = &[
    "ExpressionBody",
    "NativeBody",
    "ExternalBody",
    "IncompleteBody",
];

/// Decodes a whole value definition: a body wrapper carrying the definition's signature.
pub(super) fn decode_value_definition(
    value: &JsonValue,
    cursor: &str,
) -> Result<ValueDefinition, Diagnostic> {
    let (input_types, output_type, body) = decode_definition_parts(value, cursor, true)?;
    Ok(ValueDefinition {
        input_types,
        output_type,
        body,
    })
}

/// Decodes a body wrapper on its own, without requiring the signature a definition carries.
pub(super) fn decode_value_body(value: &JsonValue, cursor: &str) -> Result<ValueBody, Diagnostic> {
    Ok(decode_definition_parts(value, cursor, false)?.2)
}

type DefinitionParts = (IndexMap<String, InputTypeEntry>, Option<Type>, ValueBody);

fn decode_definition_parts(
    value: &JsonValue,
    cursor: &str,
    signature_required: bool,
) -> Result<DefinitionParts, Diagnostic> {
    let (tag, payload) = single_member(value, cursor, "a value definition")?;
    if !BODY_TAGS.contains(&tag.as_str()) {
        return Err(unknown_node_at(
            cursor,
            format!("{tag} is not a value definition body"),
        ));
    }
    let at = format!("{cursor}/{tag}");
    let canonical: &[&'static str] = match tag.as_str() {
        "ExpressionBody" => &["inputTypes", "outputType", "body"],
        "NativeBody" => &["inputTypes", "outputType", "nativeInfo"],
        // `externalName` and `targetPlatform` are listed so the window spelling reaches
        // `decode_externals`, which is what records its `legacy_spelling` warnings; mapping them
        // onto `externals` here would make the pair look like one member written twice.
        "ExternalBody" => &[
            "inputTypes",
            "outputType",
            "externals",
            "body",
            "externalName",
            "targetPlatform",
        ],
        _ => &["inputTypes", "outputType", "incompleteness", "partialBody"],
    };
    let members = wrapper_members(tag, payload, &at, canonical)?;

    let input_types = match members.get("inputTypes") {
        None => IndexMap::new(),
        Some(member) => decode_input_map(
            member.value,
            &member_cursor(&members, "inputTypes", &at),
            decode_input_type_entry,
        )?,
    };
    let output_type = optional_type(&members, "outputType", &at)?;
    // An incomplete definition may not have an output type yet (definitions-0016); the three
    // complete bodies always do (definitions-0005, 0007, 0009).
    if signature_required && output_type.is_none() && tag != "IncompleteBody" {
        return Err(missing(&at, "outputType"));
    }

    let body = match tag.as_str() {
        "ExpressionBody" => ValueBody::Expression(decode_value(
            required(&members, "body", &at)?,
            &member_cursor(&members, "body", &at),
        )?),
        "NativeBody" => ValueBody::Native {
            native_info: decode_native_info(
                required(&members, "nativeInfo", &at)?,
                &member_cursor(&members, "nativeInfo", &at),
            )?,
        },
        "ExternalBody" => ValueBody::External {
            externals: decode_externals(&members, &at)?,
            fallback: match members.get("body") {
                None => None,
                Some(member) => Some(Box::new(decode_value(
                    member.value,
                    &member_cursor(&members, "body", &at),
                )?)),
            },
        },
        _ => ValueBody::Incomplete {
            incompleteness: decode_incompleteness(
                required(&members, "incompleteness", &at)?,
                &member_cursor(&members, "incompleteness", &at),
            )?,
            partial_body: match members.get("partialBody") {
                None => None,
                Some(member) => Some(Box::new(decode_value(
                    member.value,
                    &member_cursor(&members, "partialBody", &at),
                )?)),
            },
        },
    };

    Ok((input_types, output_type, body))
}

/// Reads an external body's bindings.
///
/// Decision 0008 spells them as a list, one entry per target platform, and makes a target
/// platform unique within the list. The single-binding spelling that carried `externalName` and
/// `targetPlatform` beside the body's own members decodes as a one-entry list for the window of
/// decision 0006, warning at each member the author wrote.
fn decode_externals(
    members: &Members<'_>,
    cursor: &str,
) -> Result<Vec<ExternalBinding>, Diagnostic> {
    if let Some(member) = members.get("externals") {
        // `externalName` and `targetPlatform` are members of this body only as the window
        // spelling of `externals` itself. Beside a list they name nothing, and dropping them
        // silently would lose whatever the author meant by writing them.
        for stray in ["externalName", "targetPlatform"] {
            if members.contains_key(stray) {
                return Err(unknown_member(&format!("{cursor}/{stray}"), stray));
            }
        }

        let at = member_cursor(members, "externals", cursor);
        let entries = member
            .value
            .as_array()
            .ok_or_else(|| invalid_type(&at, "externals is an array of per-platform bindings"))?;
        let mut bindings: Vec<ExternalBinding> = Vec::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            let at = format!("{at}/{index}");
            let entry = wrapper_members(
                "ExternalBinding",
                entry,
                &at,
                &["targetPlatform", "externalName"],
            )?;
            let target_platform = decode_text(&entry, "targetPlatform", &at)?;
            if bindings
                .iter()
                .any(|binding| binding.target_platform == target_platform)
            {
                return Err(Diagnostic::normalization(
                    DiagnosticCode::DuplicateMember,
                    member_cursor(&entry, "targetPlatform", &at),
                    format!("duplicate target platform {target_platform}"),
                ));
            }
            bindings.push(ExternalBinding {
                external_name: decode_text(&entry, "externalName", &at)?,
                target_platform,
            });
        }
        return Ok(bindings);
    }

    let (Some(external_name), Some(target_platform)) =
        (members.get("externalName"), members.get("targetPlatform"))
    else {
        return Err(missing(cursor, "externals"));
    };
    for member in ["externalName", "targetPlatform"] {
        super::legacy::accept_member("ExternalBody", member, &format!("{cursor}/{member}"))?;
    }
    Ok(vec![ExternalBinding {
        target_platform: text_of(target_platform.value, &format!("{cursor}/targetPlatform"))?,
        external_name: text_of(external_name.value, &format!("{cursor}/externalName"))?,
    }])
}

fn decode_text(members: &Members<'_>, name: &str, cursor: &str) -> Result<String, Diagnostic> {
    text_of(
        required(members, name, cursor)?,
        &member_cursor(members, name, cursor),
    )
}

fn text_of(value: &JsonValue, cursor: &str) -> Result<String, Diagnostic> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| invalid_type(cursor, "expected a string"))
}

// =============================================================================
// Modules and packages
// =============================================================================

/// Decodes a module specification: the public face of a module (definitions-0010).
pub(super) fn decode_module_specification(
    value: &JsonValue,
    cursor: &str,
) -> Result<ModuleSpecification, Diagnostic> {
    let members = wrapper_members(
        "ModuleSpecification",
        value,
        cursor,
        &["types", "values", "doc"],
    )?;
    Ok(ModuleSpecification {
        types: decode_keyed(&members, "types", cursor, |value, cursor| {
            decode_documented(value, cursor, decode_type_specification)
        })?,
        values: decode_keyed(&members, "values", cursor, |value, cursor| {
            decode_documented(value, cursor, decode_value_specification)
        })?,
        doc: decode_optional_doc(&members, cursor)?,
    })
}

/// Decodes a module definition: its types and values, each access-controlled and each able to
/// carry documentation (distributions-0004, 0007).
pub(super) fn decode_module_definition(
    value: &JsonValue,
    cursor: &str,
) -> Result<ModuleDefinition, Diagnostic> {
    let members = wrapper_members(
        "ModuleDefinition",
        value,
        cursor,
        &["types", "values", "doc"],
    )?;
    Ok(ModuleDefinition {
        types: decode_keyed(&members, "types", cursor, |value, cursor| {
            decode_access_controlled(value, cursor, |value, cursor| {
                decode_documented(value, cursor, decode_type_definition)
            })
        })?,
        values: decode_keyed(&members, "values", cursor, |value, cursor| {
            decode_access_controlled(value, cursor, |value, cursor| {
                decode_documented(value, cursor, decode_value_definition)
            })
        })?,
        doc: decode_optional_doc(&members, cursor)?,
    })
}

fn decode_optional_doc(
    members: &Members<'_>,
    cursor: &str,
) -> Result<Option<Documentation>, Diagnostic> {
    match members.get("doc") {
        None => Ok(None),
        Some(member) if member.value.is_null() => Ok(None),
        Some(member) => {
            decode_documentation(member.value, &member_cursor(members, "doc", cursor)).map(Some)
        }
    }
}

/// Decodes an optional member holding an object keyed by name, whose order is the declaration
/// order; an absent member is the empty map.
fn decode_keyed<T>(
    members: &Members<'_>,
    name: &str,
    cursor: &str,
    decode_entry: impl Fn(&JsonValue, &str) -> Result<T, Diagnostic>,
) -> Result<IndexMap<String, T>, Diagnostic> {
    let Some(member) = members.get(name) else {
        return Ok(IndexMap::new());
    };
    let at = member_cursor(members, name, cursor);
    decode_map(member.value, &at, decode_entry)
}

fn decode_map<T>(
    value: &JsonValue,
    cursor: &str,
    decode_entry: impl Fn(&JsonValue, &str) -> Result<T, Diagnostic>,
) -> Result<IndexMap<String, T>, Diagnostic> {
    let entries = members_of(value, cursor, "a map keyed by name")?;
    entries
        .iter()
        .map(|(name, written)| {
            let at = format!("{cursor}/{name}");
            Ok((name.clone(), decode_entry(written, &at)?))
        })
        .collect()
}

/// Decodes a package specification: its modules' public faces.
pub(super) fn decode_package_specification(
    value: &JsonValue,
    cursor: &str,
) -> Result<PackageSpecification, Diagnostic> {
    let members = wrapper_members("PackageSpecification", value, cursor, &["modules"])?;
    Ok(PackageSpecification {
        modules: decode_keyed(&members, "modules", cursor, decode_module_specification)?,
    })
}

/// Decodes a package definition: its modules, each access-controlled.
pub(super) fn decode_package_definition(
    value: &JsonValue,
    cursor: &str,
) -> Result<PackageDefinition, Diagnostic> {
    let members = wrapper_members("PackageDefinition", value, cursor, &["modules"])?;
    Ok(PackageDefinition {
        modules: decode_keyed(&members, "modules", cursor, |value, cursor| {
            decode_access_controlled(value, cursor, decode_module_definition)
        })?,
    })
}

// =============================================================================
// Distributions and the document root
// =============================================================================

fn invalid_distribution_shape(cursor: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::normalization(DiagnosticCode::InvalidDistributionShape, cursor, message)
}

/// Decodes a distribution (distributions-0002 to 0007).
pub(super) fn decode_distribution(
    value: &JsonValue,
    cursor: &str,
) -> Result<Distribution, Diagnostic> {
    let members = value.as_object().ok_or_else(|| {
        invalid_distribution_shape(
            cursor,
            "a distribution is a wrapper object such as { \"Library\": { … } }",
        )
    })?;
    let mut entries = members.iter();
    let (tag, payload) = entries.next().ok_or_else(|| {
        invalid_distribution_shape(cursor, "an empty object names no distribution kind")
    })?;
    if entries.next().is_some() {
        return Err(invalid_distribution_shape(
            cursor,
            "a distribution names exactly one kind",
        ));
    }

    let at = format!("{cursor}/{tag}");
    match tag.as_str() {
        "Library" => {
            let members =
                wrapper_members(tag, payload, &at, &["packageName", "dependencies", "def"])?;
            Ok(Distribution::Library(LibraryContent {
                package_name: decode_package_name(&members, &at)?,
                dependencies: decode_dependencies(&members, &at)?,
                def: decode_optional_definition(&members, "def", &at)?,
            }))
        }
        "Specs" => {
            let members =
                wrapper_members(tag, payload, &at, &["packageName", "dependencies", "spec"])?;
            Ok(Distribution::Specs(SpecsContent {
                package_name: decode_package_name(&members, &at)?,
                dependencies: decode_dependencies(&members, &at)?,
                spec: match members.get("spec") {
                    None => PackageSpecification {
                        modules: IndexMap::new(),
                    },
                    Some(member) => decode_package_specification(
                        member.value,
                        &member_cursor(&members, "spec", &at),
                    )?,
                },
            }))
        }
        "Application" => {
            let members = wrapper_members(
                tag,
                payload,
                &at,
                &["packageName", "dependencies", "def", "entryPoints"],
            )?;
            Ok(Distribution::Application(ApplicationContent {
                package_name: decode_package_name(&members, &at)?,
                dependencies: decode_dependencies(&members, &at)?,
                def: decode_optional_definition(&members, "def", &at)?,
                entry_points: decode_entry_points(
                    required(&members, "entryPoints", &at)?,
                    &member_cursor(&members, "entryPoints", &at),
                )?,
            }))
        }
        other => Err(invalid_distribution_shape(
            cursor,
            format!("{other} does not name a distribution kind"),
        )),
    }
}

fn decode_optional_definition(
    members: &Members<'_>,
    name: &str,
    cursor: &str,
) -> Result<PackageDefinition, Diagnostic> {
    match members.get(name) {
        None => Ok(PackageDefinition {
            modules: IndexMap::new(),
        }),
        Some(member) => {
            decode_package_definition(member.value, &member_cursor(members, name, cursor))
        }
    }
}

fn decode_package_name(members: &Members<'_>, cursor: &str) -> Result<PackageName, Diagnostic> {
    let at = member_cursor(members, "packageName", cursor);
    let written = required(members, "packageName", cursor)?;
    parse_package_name(&text_of(written, &at)?, &at)
}

fn parse_package_name(text: &str, cursor: &str) -> Result<PackageName, Diagnostic> {
    crate::naming::Path::from_canonical_string(text)
        .map(PackageName::new)
        .map_err(|error| Diagnostic::normalization(DiagnosticCode::InvalidPath, cursor, error))
}

/// Decodes the dependency map, checking every key is a canonical package name (decision 0011).
fn decode_dependencies(members: &Members<'_>, cursor: &str) -> Result<Dependencies, Diagnostic> {
    let Some(member) = members.get("dependencies") else {
        return Ok(Dependencies::new());
    };
    let at = member_cursor(members, "dependencies", cursor);
    let entries = members_of(member.value, &at, "dependencies")?;
    entries
        .iter()
        .map(|(name, written)| {
            let at = format!("{at}/{name}");
            parse_package_name(name, &at)?;
            Ok((name.clone(), decode_package_specification(written, &at)?))
        })
        .collect()
}

/// Decodes an application's entry points (distributions-0007).
fn decode_entry_points(value: &JsonValue, cursor: &str) -> Result<EntryPoints, Diagnostic> {
    decode_map(value, cursor, |value, cursor| {
        let members = wrapper_members("EntryPoint", value, cursor, &["target", "kind", "doc"])?;
        let target_at = member_cursor(&members, "target", cursor);
        let target = decode_fqname(required(&members, "target", cursor)?, &target_at)?;
        let kind_at = member_cursor(&members, "kind", cursor);
        let kind = required(&members, "kind", cursor)?
            .as_str()
            .and_then(EntryPointKind::parse)
            .ok_or_else(|| {
                invalid_type(
                    &kind_at,
                    "an entry point's kind is main, command, handler, job or policy",
                )
            })?;
        Ok(EntryPoint {
            target: target.to_canonical_string(),
            kind,
            doc: match members.get("doc") {
                None => None,
                Some(member) => Some(text_of(
                    member.value,
                    &member_cursor(&members, "doc", cursor),
                )?),
            },
        })
    })
}

/// Decodes a whole version 4 document.
///
/// `formatVersion` comes first and `distribution` second; a document that writes them the other
/// way round is the same document. A top-level `$meta` member is reserved for a tool's own
/// bookkeeping and is ignored rather than refused.
pub(super) fn decode_ir_file(value: &JsonValue, cursor: &str) -> Result<IRFile, Diagnostic> {
    let members = members_of(value, cursor, "a version 4 document")?;
    for member in members.keys() {
        if !matches!(member.as_str(), "formatVersion" | "distribution" | "$meta") {
            return Err(unknown_member(&format!("{cursor}/{member}"), member));
        }
    }
    let format_version = members.get("formatVersion").ok_or_else(|| {
        Diagnostic::normalization(
            DiagnosticCode::MissingFormatVersion,
            cursor,
            "the root object is missing formatVersion",
        )
    })?;
    let distribution = members
        .get("distribution")
        .ok_or_else(|| missing(cursor, "distribution"))?;
    Ok(IRFile {
        format_version: decode_format_version(format_version, &format!("{cursor}/formatVersion"))?,
        distribution: decode_distribution(distribution, &format!("{cursor}/distribution"))?,
    })
}

/// Decodes `formatVersion` through the shared format-version contract, reporting its stable
/// category as one of the kit's diagnostic codes (distributions-0001).
pub(super) fn decode_format_version(
    value: &JsonValue,
    cursor: &str,
) -> Result<FormatVersion, Diagnostic> {
    let scalar =
        ScalarValue::from_json(value).map_err(|error| format_version_diagnostic(error, cursor))?;
    let support = SupportTable::reference();
    let normalized = NormalizedFormatVersion::from_scalar(&scalar, &support)
        .map_err(|error| format_version_diagnostic(error, cursor))?;
    // The contract records compatibility rather than failing on it, because a tool may want to
    // report an unsupported release rather than refuse the scalar. A version 4 reader refuses it.
    if let Some(error) =
        support.unsupported_diagnostic(&normalized.release, normalized.compatibility)
    {
        return Err(format_version_diagnostic(error, cursor));
    }
    Ok(normalized.canonical.into())
}

fn format_version_diagnostic(
    error: crate::format_version::FormatVersionDiagnostic,
    cursor: &str,
) -> Diagnostic {
    // The shared format-version contract's categories and the kit's diagnostic codes are the
    // same vocabulary, listed here rather than mapped by name so that renaming a category on
    // either side is a compile error or a failing assertion rather than a silent downgrade.
    let code = match error.code() {
        "missing_format_version" => DiagnosticCode::MissingFormatVersion,
        "duplicate_format_version" => DiagnosticCode::DuplicateFormatVersion,
        "invalid_format_version_type" => DiagnosticCode::InvalidFormatVersionType,
        "invalid_format_version_syntax" => DiagnosticCode::InvalidFormatVersionSyntax,
        "format_version_out_of_range" => DiagnosticCode::FormatVersionOutOfRange,
        "unsupported_format_version_major" => DiagnosticCode::UnsupportedFormatVersionMajor,
        "unsupported_format_version_minor" => DiagnosticCode::UnsupportedFormatVersionMinor,
        other => {
            debug_assert!(
                false,
                "the format-version contract reported the category {other}, which is not one of \
                 the kit's diagnostic codes"
            );
            DiagnosticCode::InvalidFormatVersionType
        }
    };
    Diagnostic::normalization(code, cursor, error.message())
}

/// Recovers a [`Diagnostic`] a nested derived decode smuggled through serde, or builds one at
/// `cursor` when there is none to recover.
pub(super) fn recover(error: &serde_json::Error, cursor: &str) -> Diagnostic {
    Diagnostic::from_serde_error(error).unwrap_or_else(|| invalid_type(cursor, error.to_string()))
}

/// Carries a [`Diagnostic`] out through a serde error, for the derived impls that still wrap one.
pub(super) fn carried<E: serde::de::Error>(diagnostic: Diagnostic) -> E {
    E::custom(DiagnosticError(diagnostic))
}
