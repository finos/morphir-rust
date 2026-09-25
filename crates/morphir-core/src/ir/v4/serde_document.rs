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
use super::annotation::{Annotation, AnnotationArgument, Annotations};
use super::distribution::{
    ApplicationContent, Distribution, EntryPoint, EntryPointKind, EntryPoints, LibraryContent,
    SpecsContent,
};
use super::legacy::accept_legacy_form;
use super::linked_metadata::{DocumentMeta, MetadataScope};
use super::linked_metadata_project::expand_v4_single_file_graph;
use super::linked_metadata_scan::validate_document_scopes;
use super::linked_metadata_scan::{LinkedMetadataCarrier, StandaloneMetadata};
use super::module::{Documentation, Documented, ModuleDefinition, ModuleSpecification};
use super::package::{PackageDefinition, PackageSpecification};
use super::serde_tagged::{
    Members, carry, decode_fqname, decode_name, decode_type, decode_value, invalid_type,
    member_cursor, required, unknown_node_at, wrapper_members, wrapper_members_of,
};
use super::tree_files::{
    DistributionKind, DistributionManifestFile, ExpectedEntries, MIN_PATH_BUDGET, ModuleEntries,
    ModuleManifestFile, NodeFileBody, TypeDefinitionFile, ValueDefinitionFile, is_escaped_stem,
};
use super::types::{
    ConstructorArg, ConstructorArgSpec, ConstructorDefinition, ConstructorSpecification,
    Incompleteness, Type, TypeDefinition, TypeSpecification,
};
use super::value::{
    ExternalBinding, HoleReason, NativeHint, NativeInfo, ValueBody, ValueDefinition,
    ValueSpecification,
};
use super::{FormatVersion, IRFile};
use crate::format_version::{NormalizedFormatVersion, ScalarValue, SupportTable};
use crate::ir::{Diagnostic, DiagnosticCode, DiagnosticError};
use crate::metadata::{ContextResources, DocumentId};
use crate::naming::{ModuleName, Name, PackageName};

/// Reads the node as JSON and hands it to a cursor-carrying decoder, starting at the root.
///
/// A `Deserialize` impl gets no way to learn its own path, so the cursor a nested node reports
/// is relative to wherever the decode was entered. A whole document entered at [`IRFile`] is
/// therefore located absolutely; a fragment decoded on its own is located within itself.
pub(in crate::ir) fn deserialize_with<'de, D, T>(
    deserializer: D,
    decode: fn(&JsonValue, &str) -> Result<T, Diagnostic>,
) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = JsonValue::deserialize(deserializer)?;
    decode(&value, "").map_err(carry)
}

/// Decode a public fragment, then close its metadata over an empty document context.
/// Whole-file decoding calls the internal decoders directly and closes over `$meta` instead.
pub(super) fn deserialize_standalone_with<'de, D, T>(
    deserializer: D,
    decode: fn(&JsonValue, &str) -> Result<T, Diagnostic>,
) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: StandaloneMetadata,
{
    let value = JsonValue::deserialize(deserializer)?;
    let mut decoded = decode(&value, "").map_err(carry)?;
    decoded
        .validate_standalone()
        .map_err(|error| carry(invalid_type("", error)))?;
    Ok(decoded)
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

/// Decodes documentation: one string (definitions-0028; decision 0010). An array of lines is
/// tolerated only inside a module manifest file of a document tree, never here.
pub(in crate::ir) fn decode_documentation(
    value: &JsonValue,
    cursor: &str,
) -> Result<Documentation, Diagnostic> {
    match value {
        JsonValue::String(text) => Ok(Documentation::new(text.clone())),
        _ => Err(invalid_type(cursor, "documentation is a string")),
    }
}

/// Decodes an access-controlled node.
///
/// Canonical is the access level as the variant tag: `{ "Public": <node> }`. Accepted beside it,
/// silently: the access level flattened next to the node's own members
/// (`{ "access": "Public", … }`), the same with the node under `value`, and the `pub`/`private`
/// shorthands (definitions-0001, 0017, 0018, 0019).
pub(in crate::ir) fn decode_access_controlled<T>(
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
pub(in crate::ir) fn decode_documented<T>(
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

/// Decodes a specification's `annotations` member; absent means none.
pub(super) fn decode_annotations(
    members: &Members<'_>,
    cursor: &str,
) -> Result<Annotations, Diagnostic> {
    let Some(member) = members.get("annotations") else {
        return Ok(Annotations::default());
    };
    let at = member_cursor(members, "annotations", cursor);
    decode_annotations_value(member.value, &at)
}

pub(super) fn decode_annotations_value(
    value: &JsonValue,
    cursor: &str,
) -> Result<Annotations, Diagnostic> {
    let (entries, metadata) = if let Some(object) = value.as_object() {
        for key in object.keys() {
            if !matches!(key.as_str(), "entries" | "@context" | "facts") {
                return Err(unknown_member(&format!("{cursor}/{key}"), key));
            }
        }
        let metadata = MetadataScope::parse_unresolved(object.get("@context"), object.get("facts"))
            .map_err(|error| invalid_type(cursor, error))?;
        (object.get("entries"), metadata)
    } else {
        (Some(value), MetadataScope::default())
    };
    let items = match entries {
        Some(entries) => entries
            .as_array()
            .ok_or_else(|| invalid_type(cursor, "annotation entries must be an array"))?,
        None => {
            return Ok(Annotations {
                entries: Vec::new(),
                metadata: (!metadata.is_empty()).then(|| Box::new(metadata)),
            });
        }
    };
    let entries = items
        .iter()
        .enumerate()
        .map(|(index, item)| decode_annotation(item, &format!("{cursor}/{index}"), &metadata))
        .collect::<Result<_, _>>()?;
    Ok(Annotations {
        entries,
        metadata: (!metadata.is_empty()).then(|| Box::new(metadata)),
    })
}

fn decode_annotation(
    value: &JsonValue,
    cursor: &str,
    scope: &MetadataScope,
) -> Result<Annotation, Diagnostic> {
    match value {
        JsonValue::String(text) => {
            // The separator is the first colon after the local-name hash; the FQName's own
            // colon comes before the hash.
            let split = text
                .find('#')
                .and_then(|hash| text[hash + 1..].find(':').map(|colon| hash + 1 + colon));
            let (name_text, free_text) = match split {
                Some(at) => (&text[..at], Some(text[at + 1..].to_owned())),
                None => (text.as_str(), None),
            };
            match decode_fqname(&JsonValue::String(name_text.to_owned()), cursor) {
                Ok(name) => Ok(Annotation::Compact {
                    name,
                    text: free_text,
                }),
                Err(_) if free_text.is_none() => match scope.expand_key(name_text) {
                    Ok(declaration) => Ok(Annotation::LinkedCompact {
                        authored_name: name_text.to_owned(),
                        declaration,
                    }),
                    Err(_) => Ok(Annotation::PendingCompact {
                        authored_name: name_text.to_owned(),
                    }),
                },
                Err(diagnostic) => Err(diagnostic),
            }
        }
        JsonValue::Object(_) => {
            let members = wrapper_members("Annotation", value, cursor, &["name", "arguments"])?;
            let name_value = required(&members, "name", cursor)?;
            let name_cursor = member_cursor(&members, "name", cursor);
            let args = match members.get("arguments") {
                None => Vec::new(),
                Some(member) => {
                    let at = member_cursor(&members, "arguments", cursor);
                    member
                        .value
                        .as_array()
                        .ok_or_else(|| invalid_type(&at, "arguments is an array"))?
                        .iter()
                        .enumerate()
                        .map(|(index, item)| {
                            decode_annotation_argument(item, &format!("{at}/{index}"))
                        })
                        .collect::<Result<_, _>>()?
                }
            };
            match decode_fqname(name_value, &name_cursor) {
                Ok(name) => Ok(Annotation::Structured { name, args }),
                Err(diagnostic) => {
                    let authored_name = name_value.as_str().ok_or(diagnostic)?.to_owned();
                    match scope.expand_key(&authored_name) {
                        Ok(declaration) => Ok(Annotation::LinkedStructured {
                            authored_name,
                            declaration,
                            args,
                        }),
                        Err(_) => Ok(Annotation::PendingStructured {
                            authored_name,
                            args,
                        }),
                    }
                }
            }
        }
        _ => Err(invalid_type(
            cursor,
            "an annotation is a string or an object",
        )),
    }
}

fn decode_annotation_argument(
    value: &JsonValue,
    cursor: &str,
) -> Result<AnnotationArgument, Diagnostic> {
    // A named argument is exactly { name, value }; no value wrapper has that member set, so the
    // shape alone decides.
    if let JsonValue::Object(members) = value
        && members.len() == 2
        && members.contains_key("name")
        && members.contains_key("value")
    {
        return Ok(AnnotationArgument::Named {
            name: decode_name(&members["name"], &format!("{cursor}/name"))?,
            value: decode_value(&members["value"], &format!("{cursor}/value"))?,
        });
    }
    Ok(AnnotationArgument::Positional(decode_value(value, cursor)?))
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
pub(in crate::ir) fn decode_type_definition(
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
pub(in crate::ir) fn decode_type_specification(
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
            annotations: Vec::new().into(),
            type_params: Vec::new(),
        });
    }

    let (tag, payload) = single_member(value, cursor, "a type specification")?;
    let at = format!("{cursor}/{tag}");
    match tag.as_str() {
        "OpaqueTypeSpecification" => {
            let members = wrapper_members(tag, payload, &at, &["annotations", "typeParams"])?;
            Ok(TypeSpecification::OpaqueTypeSpecification {
                annotations: decode_annotations(&members, &at)?,
                type_params: decode_type_params(&members, &at)?,
            })
        }
        "TypeAliasSpecification" => {
            let members =
                wrapper_members(tag, payload, &at, &["annotations", "typeParams", "typeExp"])?;
            Ok(TypeSpecification::TypeAliasSpecification {
                annotations: decode_annotations(&members, &at)?,
                type_params: decode_type_params(&members, &at)?,
                type_expr: decode_member_type(&members, "typeExp", &at)?,
            })
        }
        "CustomTypeSpecification" => {
            let members = wrapper_members(
                tag,
                payload,
                &at,
                &["annotations", "typeParams", "constructors"],
            )?;
            let annotations = decode_annotations(&members, &at)?;
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
                annotations,
                type_params,
                constructors,
            })
        }
        "DerivedTypeSpecification" => {
            let members = wrapper_members(
                tag,
                payload,
                &at,
                &[
                    "annotations",
                    "typeParams",
                    "baseType",
                    "fromBaseType",
                    "toBaseType",
                ],
            )?;
            Ok(TypeSpecification::DerivedTypeSpecification {
                annotations: decode_annotations(&members, &at)?,
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
pub(in crate::ir) fn decode_value_specification(
    value: &JsonValue,
    cursor: &str,
) -> Result<ValueSpecification, Diagnostic> {
    let members = wrapper_members(
        "ValueSpecification",
        value,
        cursor,
        &["annotations", "inputs", "output"],
    )?;
    let annotations = decode_annotations(&members, cursor)?;
    let inputs = match members.get("inputs") {
        None => IndexMap::new(),
        Some(member) => decode_input_map(
            member.value,
            &member_cursor(&members, "inputs", cursor),
            decode_type,
        )?,
    };
    Ok(ValueSpecification {
        annotations,
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

/// The four value definition bodies (definitions-0005, 0007, 0008, 0009, 0016).
const BODY_TAGS: &[&str] = &[
    "ExpressionBody",
    "NativeBody",
    "ExternalBody",
    "IncompleteBody",
];

/// Decodes a whole value definition: a body wrapper carrying the definition's signature.
pub(in crate::ir) fn decode_value_definition(
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

type DefinitionParts = (IndexMap<String, Type>, Option<Type>, ValueBody);

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
            decode_type,
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
pub(in crate::ir) fn decode_module_specification(
    value: &JsonValue,
    cursor: &str,
) -> Result<ModuleSpecification, Diagnostic> {
    let members = wrapper_members(
        "ModuleSpecification",
        value,
        cursor,
        &["annotations", "types", "values", "doc"],
    )?;
    Ok(ModuleSpecification {
        annotations: decode_annotations(&members, cursor)?,
        types: decode_keyed_names(&members, "types", cursor, |value, cursor| {
            decode_documented(value, cursor, decode_type_specification)
        })?,
        values: decode_keyed_names(&members, "values", cursor, |value, cursor| {
            decode_documented(value, cursor, decode_value_specification)
        })?,
        doc: decode_optional_doc(&members, cursor)?,
    })
}

/// Decodes a module definition: its types and values, each access-controlled and each able to
/// carry documentation (distributions-0004, 0007).
pub(in crate::ir) fn decode_module_definition(
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
        types: decode_keyed_names(&members, "types", cursor, |value, cursor| {
            decode_access_controlled(value, cursor, |value, cursor| {
                decode_documented(value, cursor, decode_type_definition)
            })
        })?,
        values: decode_keyed_names(&members, "values", cursor, |value, cursor| {
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

/// [`decode_keyed`] for a member whose keys are names rather than paths.
fn decode_keyed_names<T>(
    members: &Members<'_>,
    name: &str,
    cursor: &str,
    decode_entry: impl Fn(&JsonValue, &str) -> Result<T, Diagnostic>,
) -> Result<IndexMap<String, T>, Diagnostic> {
    let Some(member) = members.get(name) else {
        return Ok(IndexMap::new());
    };
    let at = member_cursor(members, name, cursor);
    decode_named_map(member.value, &at, decode_entry)
}

/// [`decode_map`] where every key is a name.
///
/// The key is read as a [`Name`] before its entry is decoded, so a listing keyed by something that
/// cannot be a name is refused at the key rather than carried into the model — which is what a
/// module's `types` and `values` need, and what a package's `modules` does not: a module key is a
/// *path*, which a name is not.
///
/// The key is kept as it was written rather than re-spelled from the parsed name. The two are the
/// same text for every key this accepts, and keeping the written one means the reader moves no
/// bytes a writer did not ask it to.
fn decode_named_map<T>(
    value: &JsonValue,
    cursor: &str,
    decode_entry: impl Fn(&JsonValue, &str) -> Result<T, Diagnostic>,
) -> Result<IndexMap<String, T>, Diagnostic> {
    let entries = members_of(value, cursor, "a map keyed by name")?;
    entries
        .iter()
        .map(|(name, written)| {
            let at = format!("{cursor}/{name}");
            decode_name(&JsonValue::String(name.clone()), &at)?;
            Ok((name.clone(), decode_entry(written, &at)?))
        })
        .collect()
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
pub(in crate::ir) fn decode_package_specification(
    value: &JsonValue,
    cursor: &str,
) -> Result<PackageSpecification, Diagnostic> {
    let members = wrapper_members("PackageSpecification", value, cursor, &["modules"])?;
    Ok(PackageSpecification {
        modules: decode_keyed(&members, "modules", cursor, decode_module_specification)?,
    })
}

/// Decodes a package definition: its modules, each access-controlled.
pub(in crate::ir) fn decode_package_definition(
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
pub(in crate::ir) fn decode_distribution(
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
                dependencies: decode_dependencies(&members, &at, decode_package_specification)?,
                def: decode_optional_definition(&members, "def", &at)?,
            }))
        }
        "Specs" => {
            let members =
                wrapper_members(tag, payload, &at, &["packageName", "dependencies", "spec"])?;
            Ok(Distribution::Specs(SpecsContent {
                package_name: decode_package_name(&members, &at)?,
                dependencies: decode_dependencies(&members, &at, decode_package_specification)?,
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
                dependencies: decode_dependencies(&members, &at, decode_package_definition)?,
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
///
/// What an entry holds depends on the distribution: a `Library` or `Specs` depends on a package's
/// public face, an `Application` on its definitions (distributions-0010), so the caller passes the
/// decoder for the entries its kind carries.
fn decode_dependencies<T>(
    members: &Members<'_>,
    cursor: &str,
    decode_entry: fn(&JsonValue, &str) -> Result<T, Diagnostic>,
) -> Result<IndexMap<String, T>, Diagnostic> {
    let Some(member) = members.get("dependencies") else {
        return Ok(IndexMap::new());
    };
    let at = member_cursor(members, "dependencies", cursor);
    let entries = members_of(member.value, &at, "dependencies")?;
    entries
        .iter()
        .map(|(name, written)| {
            let at = format!("{at}/{name}");
            parse_package_name(name, &at)?;
            Ok((name.clone(), decode_entry(written, &at)?))
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

/// The root of a whole single-file document: an object whose only members are `formatVersion`
/// and `distribution`, both present. Answers the two members' values.
///
/// This is the check [`decode_ir_file`] makes of a version 4 root, in its order and with its
/// codes: an unknown member first (`unknown_member` at the member), then a missing
/// `formatVersion` (`missing_format_version`) and a missing `distribution` (`missing_member`),
/// both at the root. This legacy root check rejects `$meta`; the proposed 4.1.0 decoder
/// handles it separately. `what` names the document in the `invalid_type` a root that is not an
/// object earns.
///
/// Public so a classic (version 3) reader, whose derived decoder ignores unknown members, can
/// hold its root to the same rule before deserializing it.
pub fn document_root<'a>(
    value: &'a JsonValue,
    cursor: &str,
    what: &str,
) -> Result<(&'a JsonValue, &'a JsonValue), Diagnostic> {
    let members = members_of(value, cursor, what)?;
    for member in members.keys() {
        if !matches!(member.as_str(), "formatVersion" | "distribution") {
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
    Ok((format_version, distribution))
}

/// Decodes a whole version 4 document.
///
/// `formatVersion` comes first and `distribution` second; member order is irrelevant.
/// The proposed 4.1.0 revision admits document-owned `$meta` at this root.
pub(in crate::ir) fn decode_ir_file(value: &JsonValue, cursor: &str) -> Result<IRFile, Diagnostic> {
    let root = members_of(value, cursor, "a version 4 document")?;
    let (written_version, distribution) = if root.contains_key("$meta") {
        let version = root.get("formatVersion").ok_or_else(|| {
            Diagnostic::normalization(
                DiagnosticCode::MissingFormatVersion,
                cursor,
                "the root object is missing formatVersion",
            )
        })?;
        let decoded = decode_format_version(version, &format!("{cursor}/formatVersion"))?;
        if decoded != FormatVersion::String("4.1.0".to_owned()) {
            return Err(unknown_member(&format!("{cursor}/$meta"), "$meta"));
        }
        for key in root.keys() {
            if !matches!(key.as_str(), "formatVersion" | "distribution" | "$meta") {
                return Err(unknown_member(&format!("{cursor}/{key}"), key));
            }
        }
        (
            version,
            root.get("distribution")
                .ok_or_else(|| missing(cursor, "distribution"))?,
        )
    } else {
        document_root(value, cursor, "a version 4 document")?
    };
    let format_version =
        decode_format_version(written_version, &format!("{cursor}/formatVersion"))?;
    let metadata = root
        .get("$meta")
        .map(|value| {
            DocumentMeta::parse(value)
                .map_err(|error| invalid_type(&format!("{cursor}/$meta"), error))
        })
        .transpose()?
        .map(Box::new);
    let mut distribution = decode_distribution(distribution, &format!("{cursor}/distribution"))?;
    if format_version != FormatVersion::String("4.1.0".to_owned())
        && distribution.contains_linked_metadata()
    {
        return Err(invalid_type(
            cursor,
            "linked metadata requires formatVersion 4.1.0",
        ));
    }
    validate_document_scopes(&mut distribution, metadata.as_deref())
        .map_err(|error| invalid_type(cursor, error))?;
    let file = IRFile {
        format_version,
        distribution,
        metadata,
    };
    if file
        .metadata
        .as_ref()
        .is_some_and(|meta| !meta.assertion_sources.is_empty())
    {
        // Source selectors need every carrier and the containing file before
        // they can be matched. The datatype placeholder gives typed @json
        // objects a stable identity here; declaration validation follows at
        // the acquired-provider boundary.
        let owner = DocumentId::new("$document").expect("fixed nonempty identity");
        expand_v4_single_file_graph(&file, &owner, &ContextResources::new("."), |predicate| {
            Some(predicate.clone())
        })
        .map_err(|error| invalid_type(cursor, error.to_string()))?;
    }
    Ok(file)
}

/// Decodes `formatVersion` through the shared format-version contract, reporting its stable
/// category as one of the kit's diagnostic codes (distributions-0001).
pub(in crate::ir) fn decode_format_version(
    value: &JsonValue,
    cursor: &str,
) -> Result<FormatVersion, Diagnostic> {
    let scalar =
        ScalarValue::from_json(value).map_err(|error| format_version_diagnostic(error, cursor))?;
    let support = SupportTable::linked_metadata();
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

// =============================================================================
// The four files of a document tree
// =============================================================================

/// The root object of a tree file, with the reserved top-level `$meta` taken off.
///
/// `$meta` is stripped here rather than listed as an optional member of all four kinds: a member
/// check that never sees it can never report it, and a model that never holds it can never write
/// it back (decision 0014). Only the *top-level* member is reserved; a nested one is an unknown
/// member wherever it sits.
pub(crate) fn root_without_meta<'a>(
    value: &'a JsonValue,
    cursor: &str,
    what: &str,
) -> Result<std::borrow::Cow<'a, serde_json::Map<String, JsonValue>>, Diagnostic> {
    let members = members_of(value, cursor, what)?;
    if !members.contains_key("$meta") {
        return Ok(std::borrow::Cow::Borrowed(members));
    }
    let mut rest = members.clone();
    rest.remove("$meta");
    Ok(std::borrow::Cow::Owned(rest))
}

/// Every file of a tree repeats the format version at its root and is checked for support against
/// the same table a whole document is checked against, so the two cannot drift.
///
/// Read before the member check and off the raw root, so a file with no `formatVersion` answers
/// `missing_format_version` — what a single document answers — rather than a plain
/// `missing_member`.
fn decode_file_format_version(
    root: &serde_json::Map<String, JsonValue>,
    cursor: &str,
) -> Result<FormatVersion, Diagnostic> {
    let written = root.get("formatVersion").ok_or_else(|| {
        Diagnostic::normalization(
            DiagnosticCode::MissingFormatVersion,
            cursor,
            "the root has no formatVersion member",
        )
    })?;
    decode_format_version(written, &format!("{cursor}/formatVersion"))
}

/// What to call a JSON value in a message, the way every reader that has to say what it found
/// instead calls it.
fn describe_json(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "boolean",
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}

/// Decodes a distribution manifest file: the root of a document tree.
pub(in crate::ir) fn decode_distribution_manifest_file(
    value: &JsonValue,
    cursor: &str,
) -> Result<DistributionManifestFile, Diagnostic> {
    let root = root_without_meta(value, cursor, "a distribution manifest")?;
    let format_version = decode_file_format_version(&root, cursor)?;
    let members = wrapper_members_of(
        "DistributionManifestFile",
        &root,
        cursor,
        &[
            "formatVersion",
            "distribution",
            "package",
            "pathBudget",
            "dependencies",
            "entryPoints",
            "version",
            "created",
            "layout",
        ],
    )?;

    // Every required member is fetched before any of them is read, so a file missing one answers
    // `missing_member` rather than whatever the first member that *is* present happens to say.
    let written_distribution = required(&members, "distribution", cursor)?;
    let written_package = required(&members, "package", cursor)?;
    let written_path_budget = required(&members, "pathBudget", cursor)?;

    let kind_at = member_cursor(&members, "distribution", cursor);
    let kind = text_of(written_distribution, &kind_at)?;
    let distribution = DistributionKind::parse(&kind).ok_or_else(|| {
        invalid_distribution_shape(&kind_at, format!("unknown distribution \"{kind}\""))
    })?;

    let package_at = member_cursor(&members, "package", cursor);
    let package = parse_package_name(&text_of(written_package, &package_at)?, &package_at)?;

    let budget_at = member_cursor(&members, "pathBudget", cursor);
    let path_budget = decode_path_budget(written_path_budget, &budget_at)?;

    let dependencies = match members.get("dependencies") {
        None => Vec::new(),
        Some(member) => decode_dependency_names(
            member.value,
            &member_cursor(&members, "dependencies", cursor),
        )?,
    };

    // Only an Application has entry points, so the member is unknown on the other two kinds rather
    // than merely ignored.
    let entry_points = match members.get("entryPoints") {
        None => EntryPoints::new(),
        Some(member) => {
            let at = member_cursor(&members, "entryPoints", cursor);
            if distribution != DistributionKind::Application {
                return Err(Diagnostic::normalization(
                    DiagnosticCode::UnknownMember,
                    &at,
                    format!(
                        "unknown member \"entryPoints\" on a {} manifest",
                        distribution.as_str()
                    ),
                ));
            }
            decode_entry_points(member.value, &at)?
        }
    };

    // `version`, `created` and `layout` are recorded by whatever wrote the tree and mean nothing
    // to a reader; they are still type-checked, so a mistyped one is caught here rather than
    // carried through unseen.
    for key in ["version", "created", "layout"] {
        if let Some(member) = members.get(key) {
            text_of(member.value, &member_cursor(&members, key, cursor))?;
        }
    }

    Ok(DistributionManifestFile {
        format_version,
        distribution,
        package,
        path_budget,
        dependencies,
        entry_points,
    })
}

/// Decodes `pathBudget`: a number first, then an integer of at least [`MIN_PATH_BUDGET`].
///
/// The two are separate answers. Something that is not a number at all is a type error about the
/// member, the way any other mistyped member is; only a number gets to be measured against the
/// floor. The v3 document tree reads its manifest's budget here too, so the two cannot drift.
pub(crate) fn decode_path_budget(value: &JsonValue, cursor: &str) -> Result<u32, Diagnostic> {
    let JsonValue::Number(number) = value else {
        return Err(expected_string_like(cursor, "a number", value));
    };
    let refuse = || {
        invalid_type(
            cursor,
            format!("pathBudget must be an integer of at least {MIN_PATH_BUDGET}"),
        )
    };
    let budget = number.as_u64().ok_or_else(refuse)?;
    let budget = u32::try_from(budget).map_err(|_| refuse())?;
    if budget < MIN_PATH_BUDGET {
        return Err(refuse());
    }
    Ok(budget)
}

/// `expected <what>, found <kind>`: the wording every tree-file reader uses when a member is the
/// wrong sort of JSON value.
fn expected_string_like(cursor: &str, what: &str, found: &JsonValue) -> Diagnostic {
    invalid_type(
        cursor,
        format!("expected {what}, found {}", describe_json(found)),
    )
}

/// Decodes the manifest's `dependencies`: the package names whose bodies live under `deps/`.
///
/// A manifest that lists the same dependency twice would give the layout two package roots with
/// the identical directory prefix, so it is reported here, at the second occurrence.
/// `duplicate_member` is the closest code the kit has — the array plays the role a JSON object's
/// members would.
fn decode_dependency_names(
    value: &JsonValue,
    cursor: &str,
) -> Result<Vec<PackageName>, Diagnostic> {
    let items = value
        .as_array()
        .ok_or_else(|| invalid_type(cursor, "dependencies is an array of package names"))?;
    let mut names: Vec<PackageName> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let at = format!("{cursor}/{index}");
        let name = parse_package_name(&text_of(item, &at)?, &at)?;
        let canonical = name.to_canonical_string();
        if names
            .iter()
            .any(|seen| seen.to_canonical_string() == canonical)
        {
            return Err(Diagnostic::normalization(
                DiagnosticCode::DuplicateMember,
                &at,
                format!("duplicate dependency \"{canonical}\""),
            ));
        }
        names.push(name);
    }
    Ok(names)
}

/// Decodes a module manifest file: what a module is, and what it holds.
///
/// `expect` says how an inline `types` or `values` object is read. The layout knows it from the
/// distribution kind and the root the module sits under; it is never guessed from the shape,
/// because a specification that happens to look access-controlled would then read as a definition.
pub(in crate::ir) fn decode_module_manifest_file(
    value: &JsonValue,
    cursor: &str,
    expect: ExpectedEntries,
) -> Result<ModuleManifestFile, Diagnostic> {
    let root = root_without_meta(value, cursor, "a module manifest")?;
    let format_version = decode_file_format_version(&root, cursor)?;
    let members = wrapper_members_of(
        "ModuleManifestFile",
        &root,
        cursor,
        &[
            "formatVersion",
            "path",
            "module",
            "access",
            "doc",
            "types",
            "values",
            "fileNames",
        ],
    )?;

    // `module` is an accepted spelling of `path` rather than a legacy one in decision 0006's
    // window, so it is listed as a member of its own and read silently; the writer still only ever
    // emits `path`.
    let (spelled, written_path) = match (members.get("path"), members.get("module")) {
        (Some(_), Some(_)) => {
            return Err(Diagnostic::normalization(
                DiagnosticCode::UnknownMember,
                format!("{cursor}/module"),
                "module is the legacy spelling of path; write only one",
            ));
        }
        (Some(member), None) => ("path", member.value),
        (None, Some(member)) => ("module", member.value),
        (None, None) => {
            return Err(Diagnostic::normalization(
                DiagnosticCode::MissingMember,
                cursor,
                "missing member \"path\"",
            ));
        }
    };
    let path = decode_module_name(written_path, &format!("{cursor}/{spelled}"))?;

    let access = match members.get("access") {
        None => Access::Public,
        Some(member) => decode_access(member.value, &member_cursor(&members, "access", cursor))?,
    };

    let doc = decode_manifest_doc(&members, cursor)?;

    let types = decode_module_entries(
        &members,
        "types",
        cursor,
        expect,
        |value, cursor| {
            decode_access_controlled(value, cursor, |value, cursor| {
                decode_documented(value, cursor, decode_type_definition)
            })
        },
        |value, cursor| decode_documented(value, cursor, decode_type_specification),
    )?;
    let values = decode_module_entries(
        &members,
        "values",
        cursor,
        expect,
        |value, cursor| {
            decode_access_controlled(value, cursor, |value, cursor| {
                decode_documented(value, cursor, decode_value_definition)
            })
        },
        |value, cursor| decode_documented(value, cursor, decode_value_specification),
    )?;

    let mut listed = types.listed_names();
    listed.extend(values.listed_names());
    let file_names = decode_file_names(&members, cursor, &listed)?;

    Ok(ModuleManifestFile {
        format_version,
        path,
        access,
        doc,
        types,
        values,
        file_names,
    })
}

fn decode_module_name(value: &JsonValue, cursor: &str) -> Result<ModuleName, Diagnostic> {
    let text = value
        .as_str()
        .ok_or_else(|| invalid_type(cursor, "a module name must be a canonical string"))?;
    ModuleName::from_canonical_string(text)
        .map_err(|error| Diagnostic::normalization(DiagnosticCode::InvalidPath, cursor, error))
}

/// Decodes a module manifest's `doc`: one string, or — accepted only here — an array of lines,
/// joined the way the text would have read.
///
/// Only an absent member is no documentation. A `doc` that is written and is not an array goes
/// through the string check, so a `null` is a type error rather than a quiet nothing: a manifest
/// that says `doc: null` is saying something the model has no way to keep.
fn decode_manifest_doc(
    members: &Members<'_>,
    cursor: &str,
) -> Result<Option<Documentation>, Diagnostic> {
    let Some(member) = members.get("doc") else {
        return Ok(None);
    };
    let at = member_cursor(members, "doc", cursor);
    match member.value {
        JsonValue::Array(items) => {
            let mut lines = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                lines.push(expect_string(item, &format!("{at}/{index}"))?);
            }
            Ok(Some(Documentation::new(lines.join("\n"))))
        }
        other => expect_string(other, &at).map(|text| Some(Documentation::new(text))),
    }
}

/// A string, or a type error naming what was found instead.
fn expect_string(value: &JsonValue, cursor: &str) -> Result<String, Diagnostic> {
    match value {
        JsonValue::String(text) => Ok(text.clone()),
        other => Err(expected_string_like(cursor, "a string", other)),
    }
}

/// Decodes a module manifest's `types` or `values`: absent, an array of names, or an object of
/// entries read as whatever the caller expected.
fn decode_module_entries<D, S>(
    members: &Members<'_>,
    name: &str,
    cursor: &str,
    expect: ExpectedEntries,
    decode_definition: impl Fn(&JsonValue, &str) -> Result<D, Diagnostic>,
    decode_specification: impl Fn(&JsonValue, &str) -> Result<S, Diagnostic>,
) -> Result<ModuleEntries<D, S>, Diagnostic> {
    let Some(member) = members.get(name) else {
        return Ok(ModuleEntries::Names(Vec::new()));
    };
    let at = member_cursor(members, name, cursor);
    match member.value {
        JsonValue::Array(items) => items
            .iter()
            .enumerate()
            .map(|(index, item)| decode_name(item, &format!("{at}/{index}")))
            .collect::<Result<Vec<_>, _>>()
            .map(ModuleEntries::Names),
        JsonValue::Object(_) => match expect {
            ExpectedEntries::Definitions => decode_named_map(member.value, &at, decode_definition)
                .map(ModuleEntries::Definitions),
            // A definition where a specification was expected is a mistake about what the tree
            // holds, so it is reported as one here rather than reaching the specification reader
            // and coming back as an unknown variant wrapper.
            ExpectedEntries::Specifications => {
                decode_named_map(member.value, &at, |value, cursor| {
                    if looks_access_controlled(value) {
                        return Err(invalid_distribution_shape(
                            cursor,
                            "expected a specification, found an access-controlled definition",
                        ));
                    }
                    decode_specification(value, cursor)
                })
                .map(ModuleEntries::Specifications)
            }
        },
        other => Err(invalid_type(
            &at,
            format!(
                "expected an array of names or an object of entries, found {}",
                describe_json(other)
            ),
        )),
    }
}

/// The two shapes [`decode_access_controlled`] recognizes, used only to tell a definition from a
/// specification where the caller said which it expected.
fn looks_access_controlled(value: &JsonValue) -> bool {
    let Some(members) = value.as_object() else {
        return false;
    };
    if members.contains_key("access") {
        return true;
    }
    members.len() == 1
        && members
            .keys()
            .next()
            .is_some_and(|key| key == "Public" || key == "Private")
}

/// Decodes `fileNames`: the names whose stem was truncated for the path budget, each with the stem
/// its file is under.
///
/// A reader trusts what it finds here — it never recomputes the truncation — so the checks are
/// that the key is a name, that the name is one the module lists, and that the stem is a stem.
fn decode_file_names(
    members: &Members<'_>,
    cursor: &str,
    listed: &[String],
) -> Result<Vec<(Name, String)>, Diagnostic> {
    let Some(member) = members.get("fileNames") else {
        return Ok(Vec::new());
    };
    decode_file_names_member(
        member.value,
        &member_cursor(members, "fileNames", cursor),
        listed,
    )
}

/// Decodes a `fileNames` member that is present, at `at`, against the canonical names the module
/// lists. The v3 document tree reads its module manifests' `fileNames` here too, so the two
/// cannot drift.
pub(crate) fn decode_file_names_member(
    value: &JsonValue,
    at: &str,
    listed: &[String],
) -> Result<Vec<(Name, String)>, Diagnostic> {
    let entries = members_of(value, at, "fileNames")?;
    let mut recorded = Vec::with_capacity(entries.len());
    for (key, written) in entries {
        let key_at = format!("{at}/{key}");
        let name = decode_name(&JsonValue::String(key.clone()), &key_at)?;
        if !listed.contains(&name.to_canonical_string()) {
            return Err(invalid_distribution_shape(
                &key_at,
                "fileNames key not listed in types or values",
            ));
        }
        let stem = text_of(written, &key_at)?;
        if !is_escaped_stem(&stem) {
            return Err(Diagnostic::normalization(
                DiagnosticCode::InvalidName,
                &key_at,
                format!("\"{stem}\" is not an escaped stem"),
            ));
        }
        recorded.push((name, stem));
    }
    Ok(recorded)
}

/// Decodes a `<stem>.type` file.
pub(in crate::ir) fn decode_type_definition_file(
    value: &JsonValue,
    cursor: &str,
) -> Result<TypeDefinitionFile, Diagnostic> {
    let (format_version, name, body) = decode_node_file(
        value,
        cursor,
        "TypeDefinitionFile",
        |value, cursor| {
            decode_access_controlled(value, cursor, |value, cursor| {
                decode_documented(value, cursor, decode_type_definition)
            })
        },
        |value, cursor| decode_documented(value, cursor, decode_type_specification),
    )?;
    Ok(TypeDefinitionFile {
        format_version,
        name,
        body,
    })
}

/// Decodes a `<stem>.value` file.
pub(in crate::ir) fn decode_value_definition_file(
    value: &JsonValue,
    cursor: &str,
) -> Result<ValueDefinitionFile, Diagnostic> {
    let (format_version, name, body) = decode_node_file(
        value,
        cursor,
        "ValueDefinitionFile",
        |value, cursor| {
            decode_access_controlled(value, cursor, |value, cursor| {
                decode_documented(value, cursor, decode_value_definition)
            })
        },
        |value, cursor| decode_documented(value, cursor, decode_value_specification),
    )?;
    Ok(ValueDefinitionFile {
        format_version,
        name,
        body,
    })
}

/// The shape both node files share: a format version, a name, and exactly one of `def` and `spec`.
///
/// Which of the two a file carries decides what it means, so neither and both are shape errors at
/// the file's root rather than a missing or an unknown member.
fn decode_node_file<D, S>(
    value: &JsonValue,
    cursor: &str,
    node: &str,
    decode_definition: impl Fn(&JsonValue, &str) -> Result<D, Diagnostic>,
    decode_specification: impl Fn(&JsonValue, &str) -> Result<S, Diagnostic>,
) -> Result<(FormatVersion, Name, NodeFileBody<D, S>), Diagnostic> {
    let root = root_without_meta(value, cursor, "a node file")?;
    let format_version = decode_file_format_version(&root, cursor)?;
    let members = wrapper_members_of(
        node,
        &root,
        cursor,
        &["formatVersion", "name", "def", "spec"],
    )?;
    // `name` is fetched before anything else is read, so a file missing it answers `missing_member`
    // rather than whatever its body happens to say.
    let written_name = required(&members, "name", cursor)?;
    let name = decode_name(written_name, &member_cursor(&members, "name", cursor))?;

    let body = match (members.get("def"), members.get("spec")) {
        (Some(member), None) => NodeFileBody::Def(decode_definition(
            member.value,
            &member_cursor(&members, "def", cursor),
        )?),
        (None, Some(member)) => NodeFileBody::Spec(decode_specification(
            member.value,
            &member_cursor(&members, "spec", cursor),
        )?),
        _ => {
            return Err(invalid_distribution_shape(
                cursor,
                "exactly one of def or spec",
            ));
        }
    };
    Ok((format_version, name, body))
}

/// Recovers a [`Diagnostic`] a nested derived decode smuggled through serde, or builds one at
/// `cursor` when there is none to recover.
pub(in crate::ir) fn recover(error: &serde_json::Error, cursor: &str) -> Diagnostic {
    Diagnostic::from_serde_error(error).unwrap_or_else(|| invalid_type(cursor, error.to_string()))
}

/// Carries a [`Diagnostic`] out through a serde error, for the derived impls that still wrap one.
pub(in crate::ir) fn carried<E: serde::de::Error>(diagnostic: Diagnostic) -> E {
    E::custom(DiagnosticError(diagnostic))
}
