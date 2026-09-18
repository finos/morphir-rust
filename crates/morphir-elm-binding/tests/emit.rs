//! The two emitters: the resolved model lowered to classic (v3) and to v4.
//!
//! One hand-built module is emitted both ways. The classic and v4 groups pin the
//! JSON each emitter writes; the third group checks that the two agree, by
//! migrating the classic distribution in the test (`src/` never migrates) and
//! comparing it with the natively emitted v4 one.

use morphir_elm_binding::frontend::emit::{PackageInput, emitter_for};
use morphir_elm_binding::resolved::{
    Access, FqName, RConstructor, RType, ResolvedBody, ResolvedModule, ResolvedType,
};
use serde_json::{Value, json};

/// A reference to a Morphir SDK type, the way the resolver writes one.
fn sdk(module: &str, name: &str) -> FqName {
    FqName {
        package: vec!["Morphir".to_string(), "SDK".to_string()],
        module: vec![module.to_string()],
        name: name.to_string(),
    }
}

fn int() -> RType {
    RType::Ref(sdk("Basics", "Int"), vec![])
}

fn string() -> RType {
    RType::Ref(sdk("String", "String"), vec![])
}

fn float() -> RType {
    RType::Ref(sdk("Basics", "Float"), vec![])
}

/// `My.Types`: a public alias, a public custom type with public constructors, and
/// a private alias.
fn sample_module() -> ResolvedModule {
    ResolvedModule {
        name: vec!["My".to_string(), "Types".to_string()],
        access: Access::Public,
        doc: None,
        types: vec![
            ResolvedType {
                name: "Id".to_string(),
                access: Access::Public,
                doc: None,
                params: vec![],
                body: ResolvedBody::Alias(int()),
            },
            ResolvedType {
                name: "Status".to_string(),
                access: Access::Public,
                doc: None,
                params: vec![],
                body: ResolvedBody::Custom {
                    constructor_access: Access::Public,
                    constructors: vec![
                        RConstructor {
                            name: "Active".to_string(),
                            args: vec![],
                        },
                        RConstructor {
                            name: "Closed".to_string(),
                            args: vec![string(), int()],
                        },
                    ],
                },
            },
            ResolvedType {
                name: "Hidden".to_string(),
                access: Access::Private,
                doc: None,
                params: vec![],
                body: ResolvedBody::Alias(float()),
            },
        ],
        depends_on: vec![],
        skipped_values: vec![],
    }
}

fn package() -> Vec<String> {
    vec!["Local".to_string(), "Example".to_string()]
}

/// Emits the sample package the way a caller does: one module at a time, then the
/// distribution assembled from those per-module values.
fn emit(ir_version: &str) -> Value {
    let emitter = emitter_for(ir_version).expect("an emitter for a supported IR version");
    let modules = vec![sample_module()];
    let package = package();
    let module_irs: Vec<(Vec<String>, Access, Value)> = modules
        .iter()
        .map(|module| {
            (
                module.name.clone(),
                module.access,
                emitter.emit_module(module),
            )
        })
        .collect();
    let input = PackageInput {
        package: &package,
        modules: &modules,
        dependencies: &[],
    };
    emitter.emit_distribution(&input, &module_irs)
}

// ----------------------------------------------------------------------------
// Emitter selection
// ----------------------------------------------------------------------------

#[test]
fn an_emitter_is_selected_by_ir_version() {
    assert_eq!(
        emitter_for("3").expect("classic emitter").format_version(),
        "3"
    );
    assert_eq!(emitter_for("4").expect("v4 emitter").format_version(), "4");
    assert!(emitter_for("5").is_none());
    assert!(emitter_for("").is_none());
}

// ----------------------------------------------------------------------------
// Classic (v3) shape
// ----------------------------------------------------------------------------

#[test]
fn classic_writes_a_library_distribution_for_the_package() {
    let ir = emit("3");
    assert_eq!(ir["formatVersion"], json!(3));
    assert_eq!(ir["distribution"][0], json!("Library"));
    assert_eq!(ir["distribution"][1], json!([["local"], ["example"]]));
    // No dependencies were supplied, so the dependency list is empty.
    assert_eq!(ir["distribution"][2], json!([]));
}

#[test]
fn classic_writes_one_entry_per_module() {
    let ir = emit("3");
    let modules = &ir["distribution"][3]["modules"];
    assert_eq!(modules.as_array().expect("a module list").len(), 1);
    assert_eq!(modules[0][0], json!([["my"], ["types"]]));
    assert_eq!(modules[0][1]["access"], json!("Public"));
    // Values are skipped by this frontend, and an undocumented module has no doc.
    assert_eq!(modules[0][1]["value"]["values"], json!([]));
    assert_eq!(modules[0][1]["value"]["doc"], json!(null));
}

#[test]
fn classic_writes_an_alias_as_a_type_alias_definition() {
    let ir = emit("3");
    let types = &ir["distribution"][3]["modules"][0][1]["value"]["types"];
    assert_eq!(types[0][0], json!(["id"]));
    assert_eq!(types[0][1]["access"], json!("Public"));
    // `Documented.doc` is a string in classic, so an undocumented type carries "".
    assert_eq!(types[0][1]["value"]["doc"], json!(""));
    assert_eq!(
        types[0][1]["value"]["value"],
        json!([
            "TypeAliasDefinition",
            [],
            [
                "Reference",
                {},
                [[["morphir"], ["s", "d", "k"]], [["basics"]], ["int"]],
                []
            ]
        ])
    );
}

#[test]
fn classic_names_positional_constructor_arguments_the_way_morphir_elm_does() {
    let ir = emit("3");
    let types = &ir["distribution"][3]["modules"][0][1]["value"]["types"];
    assert_eq!(types[1][0], json!(["status"]));
    let definition = &types[1][1]["value"]["value"];
    assert_eq!(definition[0], json!("CustomTypeDefinition"));
    assert_eq!(definition[1], json!([]));
    assert_eq!(definition[2]["access"], json!("Public"));
    assert_eq!(
        definition[2]["value"],
        json!([
            [["active"], []],
            [
                ["closed"],
                [
                    [
                        ["arg", "1"],
                        [
                            "Reference",
                            {},
                            [[["morphir"], ["s", "d", "k"]], [["string"]], ["string"]],
                            []
                        ]
                    ],
                    [
                        ["arg", "2"],
                        [
                            "Reference",
                            {},
                            [[["morphir"], ["s", "d", "k"]], [["basics"]], ["int"]],
                            []
                        ]
                    ]
                ]
            ]
        ])
    );
}

#[test]
fn classic_keeps_a_private_type_private() {
    let ir = emit("3");
    let types = &ir["distribution"][3]["modules"][0][1]["value"]["types"];
    assert_eq!(types[2][0], json!(["hidden"]));
    assert_eq!(types[2][1]["access"], json!("Private"));
}

#[test]
fn classic_emit_module_returns_the_access_controlled_module_definition() {
    let emitter = emitter_for("3").expect("classic emitter");
    let module = emitter.emit_module(&sample_module());
    assert_eq!(module["access"], json!("Public"));
    assert_eq!(module["value"]["types"][0][0], json!(["id"]));
}

// ----------------------------------------------------------------------------
// v4 shape
// ----------------------------------------------------------------------------

#[test]
fn v4_writes_a_library_distribution_keyed_by_canonical_names() {
    let ir = emit("4");
    assert_eq!(ir["formatVersion"], json!(4));
    let library = &ir["distribution"]["Library"];
    assert_eq!(library["packageName"], json!("local/example"));
    // Task 6 does not supply dependency specifications yet, so they stay empty.
    assert_eq!(library["dependencies"], json!({}));
    let modules = &library["def"]["modules"];
    assert_eq!(modules.as_object().expect("a module map").len(), 1);
    assert!(modules.get("my/types").is_some(), "modules: {modules}");
}

#[test]
fn v4_wraps_each_type_in_its_access_tag() {
    let ir = emit("4");
    let module = &ir["distribution"]["Library"]["def"]["modules"]["my/types"];
    // v4 writes access as the variant tag, not as an `access` member.
    let public = serde_json::to_value(morphir_core::ir::v4::Access::Public).unwrap();
    let private = serde_json::to_value(morphir_core::ir::v4::Access::Private).unwrap();
    assert_eq!(module.as_object().expect("a module").keys().len(), 1);
    assert_eq!(
        module
            .as_object()
            .and_then(|entry| entry.keys().next())
            .map(String::as_str),
        public.as_str()
    );
    let types = &module[public.as_str().unwrap()]["types"];
    assert_eq!(
        types["id"]
            .as_object()
            .and_then(|entry| entry.keys().next())
            .map(String::as_str),
        public.as_str()
    );
    assert_eq!(
        types["hidden"]
            .as_object()
            .and_then(|entry| entry.keys().next())
            .map(String::as_str),
        private.as_str()
    );
}

#[test]
fn v4_writes_an_alias_as_a_type_alias_definition() {
    let ir = emit("4");
    let types = &ir["distribution"]["Library"]["def"]["modules"]["my/types"]["Public"]["types"];
    let alias = &types["id"]["Public"];
    assert_eq!(
        alias["TypeAliasDefinition"],
        json!({
            "typeParams": [],
            "typeExp": { "Reference": { "fqname": "morphir/SDK:basics#int" } }
        })
    );
}

#[test]
fn v4_writes_constructors_keyed_by_canonical_name() {
    let ir = emit("4");
    let types = &ir["distribution"]["Library"]["def"]["modules"]["my/types"]["Public"]["types"];
    assert_eq!(
        types["status"]["Public"]["CustomTypeDefinition"],
        json!({
            "typeParams": [],
            "access": "Public",
            "constructors": {
                "active": [],
                "closed": [
                    ["arg-1", { "Reference": { "fqname": "morphir/SDK:string#string" } }],
                    ["arg-2", { "Reference": { "fqname": "morphir/SDK:basics#int" } }]
                ]
            }
        })
    );
}

#[test]
fn v4_emit_module_returns_the_access_controlled_module_definition() {
    let emitter = emitter_for("4").expect("v4 emitter");
    let module = emitter.emit_module(&sample_module());
    assert!(module["Public"]["types"]["id"].is_object(), "{module}");
    assert_eq!(module["Public"]["values"], json!({}));
}

// ----------------------------------------------------------------------------
// v3 and v4 agree
// ----------------------------------------------------------------------------

/// Drops the one member the two writers spell differently: an absent `doc`.
///
/// A classic `Documented` carries a `String`, so an undocumented classic node
/// carries `""` and the migration turns that into a `doc: ""` member. The native
/// v4 emitter has an `Option<Documentation>` and writes no `doc` member at all
/// when the resolved model has none. That is the whole difference — comparing the
/// two distributions without this filter fails on `doc: ""` alone — so nothing
/// else is normalised away: the module set, the type names and their access tags,
/// the alias bodies, the constructor lists, the argument names, the empty
/// `values` maps and every fully qualified reference are compared as written.
fn normalise(value: &Value) -> Value {
    match value {
        Value::Object(members) => Value::Object(
            members
                .iter()
                .filter(|(key, member)| {
                    !(key.as_str() == "doc" && (*member == &json!("") || member.is_null()))
                })
                .map(|(key, member)| (key.clone(), normalise(member)))
                .collect(),
        ),
        Value::Array(elements) => Value::Array(elements.iter().map(normalise).collect()),
        other => other.clone(),
    }
}

#[test]
fn the_native_v4_distribution_is_what_migrating_the_classic_one_gives() {
    let classic = emit("3");
    let native = emit("4");

    let distribution: morphir_core::ir::classic::Distribution =
        serde_json::from_value(classic).expect("the emitted classic distribution decodes");
    let migrated = morphir_core::migration::migrate_distribution(&distribution, Default::default())
        .expect("the classic distribution migrates")
        .value;
    let migrated = serde_json::to_value(&migrated).expect("the migrated distribution serializes");

    assert_eq!(
        normalise(&migrated["distribution"]),
        normalise(&native["distribution"])
    );
}

#[test]
fn agreement_is_checked_on_something() {
    // Guards the normalisation above: if it ever emptied the comparison, the
    // agreement test would pass on two empty values.
    let native = normalise(&emit("4")["distribution"]);
    let types = &native["Library"]["def"]["modules"]["my/types"]["Public"]["types"];
    assert!(types["id"]["Public"]["TypeAliasDefinition"]["typeExp"].is_object());
    assert!(
        types["status"]["Public"]["CustomTypeDefinition"]["constructors"]["closed"]
            .as_array()
            .is_some_and(|args| args.len() == 2)
    );
}
