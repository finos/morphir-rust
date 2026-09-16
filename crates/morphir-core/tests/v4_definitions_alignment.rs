//! The declaration layer of a version 4 document: access control, documentation, type and value
//! definitions and specifications, modules, packages, distributions and the file itself.
//!
//! Every fixture here is this crate's own. The Morphir Compatibility Kit is the oracle for these
//! rules, and a test that reproduced a kit fence would drift with it rather than check it: a
//! fence edited in the kit would still find a green suite. So the package is `acme/shop`, the
//! modules are `orders` and `pricing`, and the names, literals and platforms are this file's.
//! Only nullary wrappers — `{ "Draft": {} }`, `{ "Unit": {} }` — are written the one way they can
//! be written.

use morphir_core::ir::v4::{
    Access, AccessControlled, Distribution, Documented, FormatVersion, IRFile, Incompleteness,
    ModuleDefinition, ModuleSpecification, SpellingMode, TypeDefinition, TypeEncoding,
    TypeSpecification, ValueBody, ValueDefinition, ValueSpecification, with_spelling_mode,
    with_type_encoding,
};
use morphir_core::ir::{Diagnostic, DiagnosticCode, Warning};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const MONEY: &str = "acme/shop:pricing#money";
const QUANTITY: &str = "acme/shop:pricing#quantity";
const TEXT: &str = "morphir/SDK:string#string";

/// The access-controlled, documented type definition a module's `types` entry holds.
type TypeEntry = AccessControlled<Documented<TypeDefinition>>;
/// The access-controlled, documented value definition a module's `values` entry holds.
type ValueEntry = AccessControlled<Documented<ValueDefinition>>;

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, Diagnostic> {
    serde_json::from_value::<T>(value).map_err(|error| {
        Diagnostic::from_serde_error(&error)
            .unwrap_or_else(|| panic!("a v4 refusal carries a diagnostic, got: {error}"))
    })
}

/// Decodes under decision 0006's open window, returning whatever warnings it recorded.
fn decode_watching<T: DeserializeOwned>(value: Value) -> (Result<T, Diagnostic>, Vec<Warning>) {
    with_spelling_mode(SpellingMode::Current, || decode::<T>(value))
}

/// Writes a node in its canonical spelling.
fn encode<T: Serialize>(node: &T) -> Value {
    with_type_encoding(TypeEncoding::Compact, || {
        serde_json::to_value(node).unwrap()
    })
}

/// Decodes, re-encodes, and asserts the result is the canonical spelling given.
fn normalizes_to<T: DeserializeOwned + Serialize>(
    written: Value,
    canonical: &Value,
) -> Vec<Warning> {
    let (decoded, warnings) = decode_watching::<T>(written);
    assert_eq!(&encode(&decoded.unwrap()), canonical);
    warnings
}

fn cursors(warnings: &[Warning]) -> Vec<&str> {
    warnings
        .iter()
        .map(|warning| {
            assert_eq!(warning.code, DiagnosticCode::LegacySpelling);
            warning.cursor.as_str()
        })
        .collect()
}

// =============================================================================
// Access control and documentation
// =============================================================================

#[test]
fn access_control_is_the_variant_wrapper_and_the_other_spellings_decode_silently() {
    let definition = json!({ "TypeAliasDefinition": { "typeParams": [], "typeExp": TEXT } });
    let canonical = json!({ "Public": definition });

    let decoded: TypeEntry = decode(canonical.clone()).unwrap();
    assert_eq!(decoded.access, Access::Public);
    assert_eq!(encode(&decoded), canonical);

    for accepted in [
        json!({ "access": "Public", "TypeAliasDefinition": { "typeParams": [], "typeExp": TEXT } }),
        json!({ "access": "Public", "value": definition }),
        json!({ "pub": definition }),
    ] {
        let (other, warnings) = decode_watching::<TypeEntry>(accepted);
        assert_eq!(other.unwrap(), decoded);
        assert!(
            warnings.is_empty(),
            "these spellings are accepted, not legacy"
        );
    }
}

#[test]
fn a_private_definition_takes_the_same_spellings() {
    let definition = json!({ "TypeAliasDefinition": { "typeParams": [], "typeExp": QUANTITY } });
    let canonical = json!({ "Private": definition });

    let decoded: TypeEntry = decode(canonical.clone()).unwrap();
    assert_eq!(decoded.access, Access::Private);
    assert_eq!(encode(&decoded), canonical);

    for accepted in [
        json!({ "access": "Private", "TypeAliasDefinition": { "typeParams": [], "typeExp": QUANTITY } }),
        json!({ "priv": definition }),
    ] {
        assert_eq!(decode::<TypeEntry>(accepted).unwrap(), decoded);
    }
}

#[test]
fn an_access_tag_this_reader_does_not_know_is_invalid_access() {
    let refused = decode::<TypeEntry>(
        json!({ "Internal": { "TypeAliasDefinition": { "typeParams": [], "typeExp": TEXT } } }),
    )
    .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::InvalidAccess);
}

#[test]
fn documentation_is_flattened_first_and_the_nested_wrapper_warns_at_its_member() {
    let canonical = json!({ "Public": {
        "doc": "The money a line of an order is worth.",
        "TypeAliasDefinition": { "typeParams": [], "typeExp": MONEY }
    } });

    let warnings = normalizes_to::<TypeEntry>(canonical.clone(), &canonical);
    assert!(warnings.is_empty());

    // The access level flattened beside the documentation is accepted, silently.
    let warnings = normalizes_to::<TypeEntry>(
        json!({
            "access": "Public",
            "doc": "The money a line of an order is worth.",
            "TypeAliasDefinition": { "typeParams": [], "typeExp": MONEY }
        }),
        &canonical,
    );
    assert!(warnings.is_empty());

    // The definition nested under `value` inside the wrapper is the window spelling.
    let warnings = normalizes_to::<TypeEntry>(
        json!({ "Public": {
            "doc": "The money a line of an order is worth.",
            "value": { "TypeAliasDefinition": { "typeParams": [], "typeExp": MONEY } }
        } }),
        &canonical,
    );
    // The cursor is relative to the documented node, because an `AccessControlled<T>` read on
    // its own hands the payload to a derived decode that starts its own path again. Read inside
    // a module — which is where a document puts one — the whole path is reported; see
    // `a_module_reports_the_window_spelling_at_its_whole_path`.
    assert_eq!(cursors(&warnings), vec!["/value"]);
}

#[test]
fn a_module_reports_the_window_spelling_at_its_whole_path() {
    let canonical = json!({
        "types": { "money": { "Public": {
            "doc": "The money a line of an order is worth.",
            "TypeAliasDefinition": { "typeParams": [], "typeExp": MONEY }
        } } },
        "values": {}
    });
    let warnings = normalizes_to::<ModuleDefinition>(
        json!({
            "types": { "money": { "Public": {
                "doc": "The money a line of an order is worth.",
                "value": { "TypeAliasDefinition": { "typeParams": [], "typeExp": MONEY } }
            } } },
            "values": {}
        }),
        &canonical,
    );
    assert_eq!(cursors(&warnings), vec!["/types/money/Public/value"]);
}

#[test]
fn the_nested_value_wrapper_closes_with_the_window() {
    let (refused, warnings) = with_spelling_mode(SpellingMode::Pinned, || {
        decode::<TypeEntry>(json!({ "Private": {
            "value": { "TypeAliasDefinition": { "typeParams": [], "typeExp": TEXT } }
        } }))
    });
    let refused = refused.unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::UnknownMember);
    assert_eq!(refused.cursor, "/value");
    assert!(warnings.is_empty());
}

#[test]
fn a_documented_value_definition_writes_its_doc_before_the_body() {
    let canonical = json!({ "Public": {
        "doc": "Total the lines of an order.",
        "ExpressionBody": {
            "inputTypes": { "lines": QUANTITY },
            "outputType": MONEY,
            "body": { "Variable": "lines" }
        }
    } });
    assert!(normalizes_to::<ValueEntry>(canonical.clone(), &canonical).is_empty());

    let warnings = normalizes_to::<ValueEntry>(
        json!({ "Public": {
            "doc": "Total the lines of an order.",
            "value": { "ExpressionBody": {
                "inputTypes": { "lines": QUANTITY },
                "outputType": MONEY,
                "body": { "Variable": "lines" }
            } }
        } }),
        &canonical,
    );
    assert_eq!(cursors(&warnings), vec!["/value"]);
}

// =============================================================================
// Type definitions and specifications
// =============================================================================

#[test]
fn a_custom_type_definition_carries_its_access_and_its_constructor_pairs() {
    let canonical = json!({ "CustomTypeDefinition": {
        "typeParams": ["a"],
        "access": "Public",
        "constructors": {
            "shipped": [["at", TEXT]],
            "pending": []
        }
    } });
    assert!(normalizes_to::<TypeDefinition>(canonical.clone(), &canonical).is_empty());
}

#[test]
fn an_incomplete_type_definition_says_why_under_reason() {
    let hole = json!({ "IncompleteTypeDefinition": {
        "typeParams": ["a"],
        "incompleteness": { "Hole": { "reason": { "TypeMismatch": {
            "expected": MONEY,
            "found": TEXT
        } } } }
    } });
    assert!(normalizes_to::<TypeDefinition>(hole.clone(), &hole).is_empty());

    // A draft is deliberately unfinished rather than broken, so it has no reason at all and
    // keeps whatever type expression the author had written.
    let draft = json!({ "IncompleteTypeDefinition": {
        "typeParams": [],
        "incompleteness": { "Draft": {} },
        "partialTypeExp": QUANTITY
    } });
    assert!(normalizes_to::<TypeDefinition>(draft.clone(), &draft).is_empty());
}

#[test]
fn a_hole_without_a_reason_is_a_missing_member() {
    let refused =
        decode::<Incompleteness>(json!({ "Hole": { "UnresolvedReference": { "target": MONEY } } }))
            .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::UnknownMember);
    assert_eq!(refused.cursor, "/Hole/UnresolvedReference");
}

#[test]
fn an_opaque_specification_is_the_empty_wrapper_and_the_pair_spelling_decodes() {
    let canonical = json!({ "OpaqueTypeSpecification": {} });
    assert!(normalizes_to::<TypeSpecification>(canonical.clone(), &canonical).is_empty());
    assert!(
        normalizes_to::<TypeSpecification>(json!(["OpaqueTypeSpecification", []]), &canonical)
            .is_empty()
    );
}

#[test]
fn an_alias_a_custom_and_a_derived_specification_keep_their_members() {
    for canonical in [
        json!({ "TypeAliasSpecification": {
            "typeParams": ["a"],
            "typeExp": { "Reference": ["morphir/SDK:list#list", "a"] }
        } }),
        json!({ "CustomTypeSpecification": {
            "typeParams": ["a"],
            "constructors": { "shipped": [["at", TEXT]], "pending": [] }
        } }),
        json!({ "DerivedTypeSpecification": {
            "typeParams": [],
            "baseType": TEXT,
            "fromBaseType": "acme/shop:pricing#from-text",
            "toBaseType": "acme/shop:pricing#to-text"
        } }),
    ] {
        assert!(normalizes_to::<TypeSpecification>(canonical.clone(), &canonical).is_empty());
    }
}

#[test]
fn a_specification_has_no_access_level() {
    let refused = decode::<TypeSpecification>(json!({ "CustomTypeSpecification": {
        "typeParams": [],
        "access": "Public",
        "constructors": {}
    } }))
    .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::UnknownMember);
    assert_eq!(refused.cursor, "/CustomTypeSpecification/access");
}

// =============================================================================
// Value specifications and definition bodies
// =============================================================================

#[test]
fn a_value_specification_spells_inputs_and_output() {
    let canonical = json!({
        "inputs": { "quantity": QUANTITY, "unit-price": MONEY },
        "output": MONEY
    });
    assert!(normalizes_to::<ValueSpecification>(canonical.clone(), &canonical).is_empty());

    // The parameters may arrive ordered as a list of pairs; the object is what a writer emits.
    assert!(
        normalizes_to::<ValueSpecification>(
            json!({
                "inputs": [["quantity", QUANTITY], ["unit-price", MONEY]],
                "output": MONEY
            }),
            &canonical
        )
        .is_empty()
    );
}

#[test]
fn input_types_and_output_type_belong_to_a_definition_body_not_a_specification() {
    let refused = decode::<ValueSpecification>(json!({
        "inputTypes": { "quantity": QUANTITY },
        "outputType": MONEY
    }))
    .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::UnknownMember);
    assert_eq!(refused.cursor, "/inputTypes");
}

#[test]
fn the_four_definition_bodies_keep_their_members() {
    for canonical in [
        json!({ "ExpressionBody": {
            "inputTypes": { "quantity": QUANTITY },
            "outputType": MONEY,
            "body": { "Variable": "quantity" }
        } }),
        json!({ "NativeBody": {
            "inputTypes": { "left": MONEY, "right": MONEY },
            "outputType": MONEY,
            "nativeInfo": { "hint": { "Arithmetic": {} } }
        } }),
        json!({ "ExternalBody": {
            "inputTypes": { "note": TEXT },
            "outputType": "morphir/SDK:basics#unit",
            "externals": [
                { "targetPlatform": "elixir", "externalName": "Logger.info" },
                { "targetPlatform": "erlang", "externalName": "logger:info" }
            ],
            "body": { "Unit": {} }
        } }),
        json!({ "IncompleteBody": {
            "inputTypes": { "quantity": QUANTITY },
            "outputType": MONEY,
            "incompleteness": { "Hole": { "reason": { "DeletedDuringRefactor": {
                "tx-id": "shop-2026-03-04"
            } } } }
        } }),
    ] {
        assert!(normalizes_to::<ValueDefinition>(canonical.clone(), &canonical).is_empty());
    }
}

#[test]
fn a_target_platform_is_unique_within_a_definitions_bindings() {
    let refused = decode::<ValueDefinition>(json!({ "ExternalBody": {
        "inputTypes": {},
        "outputType": MONEY,
        "externals": [
            { "targetPlatform": "elixir", "externalName": "Shop.total" },
            { "targetPlatform": "elixir", "externalName": "Shop.sum" }
        ]
    } }))
    .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::DuplicateMember);
    assert_eq!(refused.cursor, "/ExternalBody/externals/1/targetPlatform");
}

#[test]
fn the_single_binding_members_name_nothing_beside_a_list_of_bindings() {
    // `externalName` and `targetPlatform` are members of an external body only as the window
    // spelling of `externals` itself. Written beside a list they are not a second binding and
    // not a legacy spelling of anything, so they are refused where the author wrote them rather
    // than dropped.
    for stray in ["externalName", "targetPlatform"] {
        let mut body = json!({ "ExternalBody": {
            "inputTypes": {},
            "outputType": MONEY,
            "externals": [{ "targetPlatform": "elixir", "externalName": "Shop.total" }]
        } });
        body["ExternalBody"][stray] = json!("Shop.stray");

        let (refused, warnings) = decode_watching::<ValueDefinition>(body);
        let refused = refused.unwrap_err();
        assert_eq!(refused.code, DiagnosticCode::UnknownMember);
        assert_eq!(refused.cursor, format!("/ExternalBody/{stray}"));
        assert!(
            warnings.is_empty(),
            "a stray member is not the window spelling"
        );
    }
}

#[test]
fn a_complete_body_states_its_output_type_and_an_incomplete_one_may_not_have_one_yet() {
    let refused = decode::<ValueDefinition>(json!({ "ExpressionBody": {
        "inputTypes": {},
        "body": { "Unit": {} }
    } }))
    .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::MissingMember);
    assert_eq!(refused.cursor, "/ExpressionBody");

    let open = json!({ "IncompleteBody": {
        "inputTypes": {},
        "incompleteness": { "Draft": {} }
    } });
    let decoded: ValueDefinition = decode(open.clone()).unwrap();
    assert!(decoded.output_type.is_none());
    assert!(matches!(decoded.body, ValueBody::Incomplete { .. }));
    assert_eq!(encode(&decoded), open);
}

#[test]
fn a_diagnostic_inside_a_nested_definition_carries_its_whole_path() {
    let refused = decode::<ValueDefinition>(json!({ "ExpressionBody": {
        "inputTypes": {},
        "outputType": MONEY,
        "body": { "LetDefinition": {
            "name": "subtotal",
            "definition": { "ExpressionBody": {
                "inputTypes": {},
                "outputType": MONEY,
                "body": { "Sorcery": {} }
            } },
            "in": { "Variable": "subtotal" }
        } }
    } }))
    .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::UnknownNode);
    assert_eq!(
        refused.cursor,
        "/ExpressionBody/body/LetDefinition/definition/ExpressionBody/body"
    );
}

// =============================================================================
// Modules
// =============================================================================

#[test]
fn a_module_specification_documents_its_values_in_place() {
    let canonical = json!({
        "types": {},
        "values": { "line-total": {
            "doc": "The money a line is worth.",
            "inputs": { "quantity": QUANTITY, "unit-price": MONEY },
            "output": MONEY
        } }
    });
    assert!(normalizes_to::<ModuleSpecification>(canonical.clone(), &canonical).is_empty());

    let warnings = normalizes_to::<ModuleSpecification>(
        json!({
            "types": {},
            "values": { "line-total": {
                "doc": "The money a line is worth.",
                "value": {
                    "inputs": { "quantity": QUANTITY, "unit-price": MONEY },
                    "output": MONEY
                }
            } }
        }),
        &canonical,
    );
    assert_eq!(cursors(&warnings), vec!["/values/line-total/value"]);
}

#[test]
fn a_module_level_access_level_is_the_variant_wrapper_and_the_flattened_pair_decodes() {
    let canonical = json!({ "Public": {
        "types": { "money": { "Public": {
            "TypeAliasDefinition": { "typeParams": [], "typeExp": TEXT }
        } } },
        "values": {},
        "doc": "How an order is priced."
    } });

    let decoded: AccessControlled<ModuleDefinition> = decode(canonical.clone()).unwrap();
    assert_eq!(encode(&decoded), canonical);

    let (flattened, warnings) = decode_watching::<AccessControlled<ModuleDefinition>>(json!({
        "access": "Public",
        "value": {
            "types": { "money": { "access": "Public", "TypeAliasDefinition": {
                "typeParams": [], "typeExp": TEXT
            } } },
            "values": {},
            "doc": "How an order is priced."
        }
    }));
    assert_eq!(flattened.unwrap(), decoded);
    assert!(warnings.is_empty());
}

#[test]
fn a_member_a_module_does_not_have_is_an_unknown_member() {
    let refused = decode::<ModuleSpecification>(json!({
        "types": {},
        "values": {},
        "entryPoints": {}
    }))
    .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::UnknownMember);
    assert_eq!(refused.cursor, "/entryPoints");
}

// =============================================================================
// Distributions and the document root
// =============================================================================

#[test]
fn a_library_a_specs_and_an_application_name_their_own_members() {
    for canonical in [
        json!({ "Library": {
            "packageName": "acme/shop",
            "dependencies": {},
            "def": { "modules": {} }
        } }),
        json!({ "Specs": {
            "packageName": "acme/shop",
            "dependencies": {},
            "spec": { "modules": { "pricing": {
                "types": {},
                "values": { "money": { "inputs": {}, "output": TEXT } }
            } } }
        } }),
        json!({ "Application": {
            "packageName": "acme/shop",
            "dependencies": {},
            "def": { "modules": {} },
            "entryPoints": { "checkout": {
                "target": "acme/shop:orders#checkout",
                "kind": "job",
                "doc": "Run the nightly checkout."
            } }
        } }),
    ] {
        assert!(normalizes_to::<Distribution>(canonical.clone(), &canonical).is_empty());
    }
}

#[test]
fn a_specs_distribution_carries_no_implementation() {
    let refused = decode::<Distribution>(json!({ "Specs": {
        "packageName": "acme/shop",
        "dependencies": {},
        "def": { "modules": {} }
    } }))
    .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::UnknownMember);
    assert_eq!(refused.cursor, "/Specs/def");
}

#[test]
fn an_application_documents_its_entry_points_rather_than_itself() {
    let refused = decode::<Distribution>(json!({ "Application": {
        "packageName": "acme/shop",
        "dependencies": {},
        "def": { "modules": {} },
        "entryPoints": {},
        "doc": "Not where a package's documentation goes."
    } }))
    .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::UnknownMember);
    assert_eq!(refused.cursor, "/Application/doc");
}

#[test]
fn an_entry_point_kind_is_drawn_from_a_fixed_set() {
    for kind in ["main", "command", "handler", "job", "policy"] {
        let written = json!({ "Application": {
            "packageName": "acme/shop",
            "dependencies": {},
            "def": { "modules": {} },
            "entryPoints": { "run": { "target": "acme/shop:orders#checkout", "kind": kind } }
        } });
        assert!(normalizes_to::<Distribution>(written.clone(), &written).is_empty());
    }

    let refused = decode::<Distribution>(json!({ "Application": {
        "packageName": "acme/shop",
        "dependencies": {},
        "def": { "modules": {} },
        "entryPoints": { "run": { "target": "acme/shop:orders#checkout", "kind": "cron" } }
    } }))
    .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::InvalidType);
    assert_eq!(refused.cursor, "/Application/entryPoints/run/kind");
}

#[test]
fn a_v3_tagged_array_is_not_a_v4_document() {
    let refused = decode::<IRFile>(json!({
        "formatVersion": 4,
        "distribution": ["Library", "acme/shop", {}, { "modules": [] }]
    }))
    .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::InvalidDistributionShape);
    assert_eq!(refused.cursor, "/distribution");
}

#[test]
fn the_root_members_may_be_written_in_either_order_and_meta_is_ignored() {
    let canonical = json!({
        "formatVersion": 4,
        "distribution": { "Library": {
            "packageName": "acme/shop",
            "dependencies": {},
            "def": { "modules": {} }
        } }
    });
    assert!(normalizes_to::<IRFile>(canonical.clone(), &canonical).is_empty());

    assert!(
        normalizes_to::<IRFile>(
            json!({
                "distribution": { "Library": {
                    "packageName": "acme/shop",
                    "dependencies": {},
                    "def": { "modules": {} }
                } },
                "formatVersion": 4
            }),
            &canonical
        )
        .is_empty()
    );

    // `$meta` is reserved for a tool's own bookkeeping: a reader passes over it.
    let with_meta: IRFile = decode(json!({
        "formatVersion": 4,
        "$meta": { "writtenBy": "the shop's build" },
        "distribution": { "Library": {
            "packageName": "acme/shop",
            "dependencies": {},
            "def": { "modules": {} }
        } }
    }))
    .unwrap();
    assert_eq!(
        with_meta.distribution.package_name().to_string(),
        "acme/shop"
    );
    assert_eq!(encode(&with_meta), canonical);
}

#[test]
fn a_root_member_that_is_neither_is_an_unknown_member() {
    let refused = decode::<IRFile>(json!({
        "formatVersion": 4,
        "meta": { "writtenBy": "the shop's build" },
        "distribution": { "Library": { "packageName": "acme/shop" } }
    }))
    .unwrap_err();
    assert_eq!(refused.code, DiagnosticCode::UnknownMember);
    assert_eq!(refused.cursor, "/meta");
}

#[test]
fn a_dependency_is_keyed_by_its_canonical_package_name() {
    let written = json!({
        "formatVersion": 4,
        "distribution": { "Library": {
            "packageName": "acme/shop",
            "dependencies": { "morphir/SDK": { "modules": {} } },
            "def": { "modules": {} }
        } }
    });
    let decoded: IRFile = decode(written.clone()).unwrap();
    let encoded = encode(&decoded);
    assert!(
        encoded["distribution"]["Library"]["dependencies"]
            .get("morphir/SDK")
            .is_some(),
        "the SDK keeps its canonical spelling: {encoded}"
    );

    // `morphir/sdk` is a valid name for some other package, so it is read as one.
    let other: IRFile = decode(json!({
        "formatVersion": 4,
        "distribution": { "Library": {
            "packageName": "acme/shop",
            "dependencies": { "morphir/sdk": { "modules": {} } },
            "def": { "modules": {} }
        } }
    }))
    .unwrap();
    assert_ne!(other, decoded);
}

#[test]
fn the_format_version_spellings_are_the_shared_contracts() {
    assert_eq!(
        decode::<FormatVersion>(json!(4)).unwrap(),
        FormatVersion::Integer(4)
    );
    assert_eq!(
        decode::<FormatVersion>(json!("4.0.0")).unwrap(),
        FormatVersion::Integer(4)
    );
    assert_eq!(
        decode::<FormatVersion>(json!("4.0.0-rc1"))
            .unwrap_err()
            .code,
        DiagnosticCode::InvalidFormatVersionSyntax
    );
    assert_eq!(
        decode::<FormatVersion>(json!("4.2.0")).unwrap_err().code,
        DiagnosticCode::UnsupportedFormatVersionRevision
    );
}

// =============================================================================
// The checked-in examples
// =============================================================================

#[test]
fn the_published_complete_example_decodes_and_normalizes() {
    let published: Value =
        serde_json::from_str(include_str!("fixtures/ir/v4/complete-example.json")).unwrap();
    let (decoded, warnings) = decode_watching::<IRFile>(published);
    let decoded = decoded.unwrap();
    assert!(
        warnings.is_empty(),
        "the published example uses accepted spellings, not legacy ones: {warnings:?}"
    );

    // It spells its format version `"4.0.0"` and its access levels flattened, both of which a
    // reader accepts and a writer never emits.
    let written = encode(&decoded);
    assert_eq!(written["formatVersion"], json!(4));
    let modules = &written["distribution"]["Library"]["def"]["modules"];
    let module = modules
        .as_object()
        .and_then(|modules| modules.values().next())
        .expect("the example defines a module");
    assert!(
        module.get("Public").is_some(),
        "a module's access level is the variant wrapper: {module}"
    );
    assert!(module["Public"].get("doc").is_some());
    assert!(
        written["distribution"]["Library"]["dependencies"]
            .get("morphir/SDK")
            .is_some()
    );

    // Writing what was read is a fixed point.
    assert_eq!(encode(&decode::<IRFile>(written.clone()).unwrap()), written);
}

#[test]
fn the_checked_in_library_distribution_decodes_and_normalizes() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/ir/v4/v4-library-distribution.json")).unwrap();
    let (decoded, warnings) = decode_watching::<IRFile>(fixture);
    let decoded = decoded.unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let written = encode(&decoded);
    assert_eq!(encode(&decode::<IRFile>(written.clone()).unwrap()), written);
}

#[test]
fn the_example_fragments_are_written_in_the_decided_spellings() {
    fn examples(source: &str) -> Vec<Value> {
        let document: Value = serde_json::from_str(source).unwrap();
        document["examples"]
            .as_object()
            .expect("a fragment file holds its examples in an object")
            .values()
            .cloned()
            .collect()
    }

    for example in examples(include_str!("fixtures/ir/v4/incompleteness-examples.json")) {
        assert!(normalizes_to::<Incompleteness>(example.clone(), &example).is_empty());
    }
    for example in examples(include_str!(
        "fixtures/ir/v4/incomplete-type-definition-example.json"
    )) {
        assert!(normalizes_to::<TypeDefinition>(example.clone(), &example).is_empty());
    }
    for example in examples(include_str!("fixtures/ir/v4/hole-reason-examples.json")) {
        let wrapped = json!({ "Hole": { "reason": example } });
        assert!(normalizes_to::<Incompleteness>(wrapped.clone(), &wrapped).is_empty());
    }
    for example in examples(include_str!("fixtures/ir/v4/native-hint-examples.json")) {
        let body = json!({ "NativeBody": {
            "inputTypes": {},
            "outputType": MONEY,
            "nativeInfo": { "hint": example }
        } });
        assert!(normalizes_to::<ValueDefinition>(body.clone(), &body).is_empty());
    }
}
