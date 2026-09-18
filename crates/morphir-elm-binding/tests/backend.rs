//! The Elm backend: a Morphir IR distribution in, Elm source out.
//!
//! The same model is generated from twice — once from a hand-written v3
//! document and once from the v4 document this crate's own emitter writes — and
//! both have to produce byte-identical Elm. That is the whole point of decoding
//! into the version-neutral resolved model: the generated source cannot depend
//! on which version the document was written in.

use morphir_elm_binding::ElmExtension;
use morphir_elm_binding::frontend::emit::{PackageInput, emitter_for};
use morphir_elm_binding::resolved::{
    Access, FqName, RConstructor, RType, ResolvedBody, ResolvedModule, ResolvedType,
};
use morphir_extension_sdk::prelude::*;
use serde_json::{Value, json};

// ----------------------------------------------------------------------------
// The one model, written two ways
// ----------------------------------------------------------------------------

fn sdk(module: &str, name: &str) -> FqName {
    FqName {
        package: vec!["Morphir".to_string(), "SDK".to_string()],
        module: vec![module.to_string()],
        name: name.to_string(),
    }
}

/// `My.Types`: `type alias Id = Int` and `type Status = Active | Closed String Int`.
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
                body: ResolvedBody::Alias(RType::Ref(sdk("Basics", "Int"), vec![])),
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
                            args: vec![
                                RType::Ref(sdk("String", "String"), vec![]),
                                RType::Ref(sdk("Basics", "Int"), vec![]),
                            ],
                        },
                    ],
                },
            },
        ],
        depends_on: vec![],
        skipped_values: vec![],
    }
}

/// The classic reference to `Morphir.SDK.<module>#<name>`, as a v3 document
/// spells one.
fn classic_reference(module: &str, name: &str) -> Value {
    json!([
        "Reference",
        {},
        [[["morphir"], ["s", "d", "k"]], [[module]], [name]],
        []
    ])
}

/// The v3 document for [`sample_module`], written by hand rather than by this
/// crate's emitter, so the decoder is checked against the format and not just
/// against its own inverse.
fn classic_ir() -> Value {
    json!({
        "formatVersion": 3,
        "distribution": ["Library", [["local"], ["example"]], [], {
            "modules": [[
                [["my"], ["types"]],
                {"access": "Public", "value": {
                    "types": [
                        [["id"], {"access": "Public", "value": {
                            "doc": "",
                            "value": ["TypeAliasDefinition", [], classic_reference("basics", "int")]
                        }}],
                        [["status"], {"access": "Public", "value": {
                            "doc": "",
                            "value": ["CustomTypeDefinition", [], {"access": "Public", "value": [
                                [["active"], []],
                                [["closed"], [
                                    [["arg", "1"], classic_reference("string", "string")],
                                    [["arg", "2"], classic_reference("basics", "int")]
                                ]]
                            ]}]
                        }}]
                    ],
                    "values": [],
                    "doc": null
                }}
            ]]
        }]
    })
}

/// The v4 document for [`sample_module`], written natively by this crate's own
/// v4 emitter.
fn v4_ir() -> Value {
    let emitter = emitter_for("4").expect("a v4 emitter");
    let modules = vec![sample_module()];
    let package = vec!["Local".to_string(), "Example".to_string()];
    let module_irs = vec![(
        modules[0].name.clone(),
        modules[0].access,
        emitter
            .emit_module(&modules[0])
            .expect("the module is emitted"),
    )];
    emitter
        .emit_distribution(
            &PackageInput {
                package: &package,
                modules: &modules,
                dependencies: &[],
            },
            &module_irs,
        )
        .expect("the distribution is assembled")
}

// ----------------------------------------------------------------------------
// Generating
// ----------------------------------------------------------------------------

fn generate(ir: Value, target: &str) -> GenerateResult {
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    extension
        .backend()
        .unwrap()
        .generate(GenerateRequest {
            ir,
            target: target.into(),
            options: Default::default(),
        })
        .unwrap()
}

fn artifact(result: &GenerateResult, path: &str) -> String {
    result
        .artifacts
        .iter()
        .find(|artifact| artifact.path == path)
        .unwrap_or_else(|| {
            panic!(
                "no artifact at `{path}`; generated {:?}",
                result
                    .artifacts
                    .iter()
                    .map(|artifact| artifact.path.as_str())
                    .collect::<Vec<_>>()
            )
        })
        .content
        .clone()
}

fn codes(result: &GenerateResult, code: &str) -> Vec<Diagnostic> {
    result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.as_deref() == Some(code))
        .cloned()
        .collect()
}

#[test]
fn both_ir_versions_generate_the_same_elm() {
    let from_v3 = generate(classic_ir(), "elm");
    let from_v4 = generate(v4_ir(), "elm");

    assert!(from_v3.success, "{:?}", from_v3.diagnostics);
    assert!(from_v4.success, "{:?}", from_v4.diagnostics);
    assert_eq!(from_v3.artifacts.len(), 1);
    assert_eq!(from_v4.artifacts.len(), 1);

    let v3 = artifact(&from_v3, "src/My/Types.elm");
    let v4 = artifact(&from_v4, "src/My/Types.elm");
    assert_eq!(v3, v4, "the two versions describe the same module");
}

#[test]
fn a_module_is_written_as_elm_format_would_write_it() {
    let elm = artifact(&generate(classic_ir(), "elm"), "src/My/Types.elm");

    assert_eq!(
        elm,
        "module My.Types exposing (Id, Status(..))\n\
         \n\
         \n\
         type alias Id =\n\
         \x20   Int\n\
         \n\
         \n\
         type Status\n\
         \x20   = Active\n\
         \x20   | Closed String Int\n",
        "generated:\n{elm}"
    );
}

/// `Int` and `String` come from the prelude's implicit imports, so nothing has
/// to be imported to name them.
#[test]
fn a_type_an_implicit_import_exposes_needs_no_import() {
    let elm = artifact(&generate(classic_ir(), "elm"), "src/My/Types.elm");

    assert!(!elm.contains("import"), "generated:\n{elm}");
}

#[test]
fn rejects_non_elm_target() {
    let result = generate(classic_ir(), "gleam");

    assert!(!result.success);
    assert!(result.artifacts.is_empty());
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].code.as_deref(), Some("ELM_REQUEST"));
    assert!(
        result.diagnostics[0].message.contains("gleam"),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn rejects_an_ir_version_it_cannot_read() {
    let result = generate(json!({"formatVersion": 2, "distribution": []}), "elm");

    assert!(!result.success);
    assert!(result.artifacts.is_empty());
    assert_eq!(result.diagnostics[0].code.as_deref(), Some("ELM_IR"));
}

#[test]
fn rejects_a_document_that_states_no_format_version() {
    let result = generate(json!({"distribution": []}), "elm");

    assert!(!result.success);
    assert_eq!(result.diagnostics[0].code.as_deref(), Some("ELM_IR"));
}

/// A document may spell its version as the release string `"3.0.0"` as well as
/// the integer `3`; the version is read through
/// [`morphir_core::format_version`], the same way every other backend reads one.
#[test]
fn accepts_a_format_version_written_as_a_release_string() {
    let mut ir = classic_ir();
    ir["formatVersion"] = json!("3.0.0");
    let result = generate(ir, "elm");

    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.artifacts.len(), 1);
}

/// The release grammar wants a whole triplet from a string, so a bare `"3"` is
/// not a release this or any other backend reads. It is refused, not guessed at.
#[test]
fn rejects_a_bare_string_format_version() {
    let mut ir = classic_ir();
    ir["formatVersion"] = json!("3");
    let result = generate(ir, "elm");

    assert!(!result.success);
    assert_eq!(result.diagnostics[0].code.as_deref(), Some("ELM_IR"));
}

/// A v4 type definition with no Elm form is reported and left out, and the rest
/// of the module is still written.
#[test]
fn v4_unsupported_construct_is_reported() {
    use morphir_core::ir::v4::{
        Access as V4Access, AccessControlled, Documented, Incompleteness, TypeDefinition,
    };

    let incomplete = serde_json::to_value(AccessControlled {
        access: V4Access::Public,
        value: Documented::new(
            None,
            TypeDefinition::IncompleteTypeDefinition {
                type_params: vec![],
                incompleteness: Incompleteness::Draft,
                partial_type_expr: None,
            },
        ),
    })
    .expect("an incomplete type definition serializes");

    let mut ir = v4_ir();
    ir["distribution"]["Library"]["def"]["modules"]["my/types"]["Public"]["types"]["draft"] =
        incomplete;

    let result = generate(ir, "elm");

    let unsupported = codes(&result, "ELM_UNSUPPORTED");
    assert_eq!(unsupported.len(), 1, "{:?}", result.diagnostics);
    assert!(
        unsupported[0].message.contains("IncompleteTypeDefinition"),
        "{:?}",
        unsupported[0]
    );
    assert!(
        unsupported[0].message.contains("My.Types") && unsupported[0].message.contains("Draft"),
        "the diagnostic names the type: {:?}",
        unsupported[0]
    );

    // The rest of the module is still written, and the construct with no Elm
    // form is left out rather than written as something it is not.
    let elm = artifact(&result, "src/My/Types.elm");
    assert!(elm.contains("type alias Id ="), "generated:\n{elm}");
    assert!(!elm.contains("Draft"), "generated:\n{elm}");
}

/// Values are counted, never written: this backend generates types only.
#[test]
fn value_definitions_are_counted_and_skipped() {
    use morphir_core::ir::classic::{
        Access as CAccess, AccessControlled, Attrs, Documented, Name, Type, Value as CValue,
        ValueDefinition,
    };

    let greet = serde_json::to_value(vec![(
        Name::from_str("greet"),
        AccessControlled {
            access: CAccess::Public,
            value: Documented {
                doc: String::new(),
                value: ValueDefinition::<Attrs, Type<Attrs>> {
                    input_types: vec![],
                    output_type: Type::Unit(Attrs::None),
                    body: CValue::Unit(Type::Unit(Attrs::None)),
                },
            },
        },
    )])
    .expect("a value definition serializes");

    let mut ir = classic_ir();
    ir["distribution"][3]["modules"][0][1]["value"]["values"] = greet;

    let result = generate(ir, "elm");

    assert!(result.success, "{:?}", result.diagnostics);
    let skipped = codes(&result, "ELM_VALUE_SKIPPED");
    assert_eq!(skipped.len(), 1, "{:?}", result.diagnostics);
    assert_eq!(skipped[0].severity, DiagnosticSeverity::Warning);
    assert!(
        skipped[0].message.contains("omitted 1 value definitions"),
        "{:?}",
        skipped[0]
    );
    let elm = artifact(&result, "src/My/Types.elm");
    assert!(!elm.contains("greet"), "generated:\n{elm}");
}

#[test]
fn no_value_definitions_means_no_warning() {
    let result = generate(classic_ir(), "elm");

    assert!(codes(&result, "ELM_VALUE_SKIPPED").is_empty());
}

/// A private type is still written — Elm has no way to say "declared but not
/// exposed" other than leaving it out of the exposing list.
#[test]
fn a_private_type_is_written_but_not_exposed() {
    let mut ir = classic_ir();
    ir["distribution"][3]["modules"][0][1]["value"]["types"][0][1]["access"] = json!("Private");

    let elm = artifact(&generate(ir, "elm"), "src/My/Types.elm");

    assert!(
        elm.starts_with("module My.Types exposing (Status(..))\n"),
        "generated:\n{elm}"
    );
    assert!(elm.contains("type alias Id ="), "generated:\n{elm}");
}

/// A custom type whose constructors are private is exposed without `(..)`.
#[test]
fn an_opaque_type_is_exposed_without_its_constructors() {
    let mut ir = classic_ir();
    ir["distribution"][3]["modules"][0][1]["value"]["types"][1][1]["value"]["value"][2]["access"] =
        json!("Private");

    let elm = artifact(&generate(ir, "elm"), "src/My/Types.elm");

    assert!(
        elm.starts_with("module My.Types exposing (Id, Status)\n"),
        "generated:\n{elm}"
    );
}

/// Elm forbids an empty exposing list, so a module with nothing public says
/// `exposing (..)`.
#[test]
fn a_module_with_no_public_types_exposes_everything() {
    let mut ir = classic_ir();
    for index in 0..2 {
        ir["distribution"][3]["modules"][0][1]["value"]["types"][index][1]["access"] =
            json!("Private");
    }

    let elm = artifact(&generate(ir, "elm"), "src/My/Types.elm");

    assert!(
        elm.starts_with("module My.Types exposing (..)\n"),
        "generated:\n{elm}"
    );
}

/// An unknown prelude is a bad request, not a generation failure.
#[test]
fn rejects_an_unknown_prelude() {
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    let result = extension
        .backend()
        .unwrap()
        .generate(GenerateRequest {
            ir: classic_ir(),
            target: "elm".into(),
            options: [("elmPrelude".to_string(), json!("nope"))]
                .into_iter()
                .collect(),
        })
        .unwrap();

    assert!(!result.success);
    assert_eq!(result.diagnostics[0].code.as_deref(), Some("ELM_REQUEST"));
}
