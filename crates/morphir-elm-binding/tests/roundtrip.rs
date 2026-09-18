//! Elm in, Elm out: source → Morphir IR → source.
//!
//! The fixture is compiled to both IR versions, generated back to Elm, and the
//! generated Elm is read again. Three things are then true of it, and each one
//! catches a different kind of mistake:
//!
//! 1. The re-read AST equals the original's, once everything the IR does not
//!    keep is stripped: spans, value declarations, and the *style* in which a
//!    reference is written. A Morphir document records that `pair` holds a
//!    `My.Other.Thing`; it does not record that the author wrote `Other.Thing`
//!    under `import My.Other as Other`. So a reference's qualifier is compared
//!    by the import it implies — the import list, as a set of module paths —
//!    rather than character by character.
//! 2. Compiling the generated Elm yields the very same distribution. This is
//!    the check that a reference still names the module it named before: the
//!    IR holds fully qualified names, so a mis-qualified reference either fails
//!    to resolve or resolves somewhere else, and either way the documents
//!    differ.
//! 3. Printing the re-read module reproduces the generated text exactly, so the
//!    printer has one normal form and generation is idempotent.

use morphir_elm_binding::ElmExtension;
use morphir_elm_binding::ast::{Constructor, Field, Import, Module, TypeDecl, TypeExpr};
use morphir_elm_binding::backend::print::print;
use morphir_elm_binding::frontend::{cst_to_ast, parse};
use morphir_elm_binding::span::Span;
use morphir_extension_sdk::prelude::*;
use serde_json::Value;

const TYPES: &str = include_str!("fixtures/Types.elm");
const OTHER: &str = "module My.Other exposing (Thing)\n\ntype Thing = Thing\n";

const TYPES_URI: &str = "file:///work/My/Domain/Types.elm";
const OTHER_URI: &str = "file:///work/My/Other.elm";

// ----------------------------------------------------------------------------
// Driving the frontend and the backend
// ----------------------------------------------------------------------------

fn document(uri: &str, text: &str) -> SourceDocument {
    SourceDocument {
        uri: uri.into(),
        language_id: "elm".into(),
        version: 1,
        text: text.into(),
    }
}

/// The fixture package is called `My`, which is the prefix of both fixture
/// module names, so the frontend strips it and the backend writes it back.
fn compile(documents: Vec<SourceDocument>, ir_version: &str) -> CompileResult {
    compile_as("My", documents, ir_version)
}

fn compile_as(
    package_name: &str,
    documents: Vec<SourceDocument>,
    ir_version: &str,
) -> CompileResult {
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    let result = extension
        .frontend()
        .unwrap()
        .compile(CompileRequest {
            language_id: "elm".into(),
            documents,
            package: CompilePackage {
                name: package_name.into(),
                exposed_modules: None,
            },
            dependencies: vec![],
            options: CompileOptions {
                types_only: false,
                ir_version: ir_version.into(),
                extra: Default::default(),
            },
            baseline: None,
        })
        .unwrap();
    assert!(result.success, "v{ir_version}: {:?}", result.diagnostics);
    result
}

fn generate(ir: Value) -> GenerateResult {
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    let result = extension
        .backend()
        .unwrap()
        .generate(GenerateRequest {
            ir,
            target: "elm".into(),
            options: Default::default(),
        })
        .unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    result
}

fn artifact(result: &GenerateResult, path: &str) -> String {
    result
        .artifacts
        .iter()
        .find(|artifact| artifact.path == path)
        .unwrap_or_else(|| panic!("no artifact at `{path}`"))
        .content
        .clone()
}

fn to_ast(text: &str) -> Module {
    let parsed = parse::parse(text);
    let errors = parse::syntax_errors(&parsed, text);
    assert!(
        errors.is_empty(),
        "the Elm does not parse:\n{text}\n{errors:?}"
    );
    cst_to_ast::to_ast(&parsed, text).unwrap_or_else(|error| panic!("{error:?}\n{text}"))
}

/// Source → IR → source, for one IR version: the generated `My.Domain.Types`
/// and `My.Other`.
fn regenerate(ir_version: &str) -> (String, String) {
    let compiled = compile(
        vec![document(TYPES_URI, TYPES), document(OTHER_URI, OTHER)],
        ir_version,
    );
    let generated = generate(compiled.ir.expect("a distribution"));
    (
        artifact(&generated, "src/My/Domain/Types.elm"),
        artifact(&generated, "src/My/Other.elm"),
    )
}

// ----------------------------------------------------------------------------
// Stripping what a Morphir document does not keep
// ----------------------------------------------------------------------------

const ZERO: Span = Span { start: 0, end: 0 };

/// A module reduced to what its Morphir document actually records.
///
/// Spans go (a document holds no source positions), value declarations go (this
/// frontend skips them), and a reference keeps its name but not its qualifier:
/// how a name is reached is the writer's choice, and it is checked instead by
/// comparing the imports it implies, as a set of module paths.
fn strip(module: Module) -> Module {
    let mut imports: Vec<Import> = module
        .imports
        .into_iter()
        .map(|import| Import {
            module: import.module,
            alias: None,
            exposing: None,
            span: ZERO,
        })
        .collect();
    imports.sort_by(|left, right| left.module.cmp(&right.module));
    imports.dedup_by(|left, right| left.module == right.module);

    Module {
        name: module.name,
        exposing: module.exposing,
        imports,
        doc: module.doc,
        types: module.types.into_iter().map(strip_decl).collect(),
        skipped_values: Vec::new(),
        span: ZERO,
    }
}

fn strip_decl(decl: TypeDecl) -> TypeDecl {
    match decl {
        TypeDecl::Alias {
            name,
            params,
            body,
            doc,
            ..
        } => TypeDecl::Alias {
            name,
            params,
            body: strip_type(body),
            doc,
            span: ZERO,
        },
        TypeDecl::Custom {
            name,
            params,
            constructors,
            doc,
            ..
        } => TypeDecl::Custom {
            name,
            params,
            constructors: constructors
                .into_iter()
                .map(|constructor| Constructor {
                    name: constructor.name,
                    args: constructor.args.into_iter().map(strip_type).collect(),
                    span: ZERO,
                })
                .collect(),
            doc,
            span: ZERO,
        },
    }
}

fn strip_type(ty: TypeExpr) -> TypeExpr {
    match ty {
        TypeExpr::Var { name, .. } => TypeExpr::Var { name, span: ZERO },
        TypeExpr::Ref { name, args, .. } => TypeExpr::Ref {
            module: Vec::new(),
            name,
            args: args.into_iter().map(strip_type).collect(),
            span: ZERO,
        },
        TypeExpr::Record { fields, .. } => TypeExpr::Record {
            fields: fields.into_iter().map(strip_field).collect(),
            span: ZERO,
        },
        TypeExpr::ExtensibleRecord { base, fields, .. } => TypeExpr::ExtensibleRecord {
            base,
            fields: fields.into_iter().map(strip_field).collect(),
            span: ZERO,
        },
        TypeExpr::Tuple { items, .. } => TypeExpr::Tuple {
            items: items.into_iter().map(strip_type).collect(),
            span: ZERO,
        },
        TypeExpr::Function { arg, result, .. } => TypeExpr::Function {
            arg: Box::new(strip_type(*arg)),
            result: Box::new(strip_type(*result)),
            span: ZERO,
        },
        TypeExpr::Unit { .. } => TypeExpr::Unit { span: ZERO },
    }
}

fn strip_field(field: Field) -> Field {
    Field {
        name: field.name,
        ty: strip_type(field.ty),
    }
}

// ----------------------------------------------------------------------------
// The round trip
// ----------------------------------------------------------------------------

#[test]
fn a_module_survives_the_round_trip_through_both_ir_versions() {
    for version in ["3", "4"] {
        let (types, other) = regenerate(version);

        assert_eq!(
            strip(to_ast(&types)),
            strip(to_ast(TYPES)),
            "v{version}, generated:\n{types}"
        );
        assert_eq!(
            strip(to_ast(&other)),
            strip(to_ast(OTHER)),
            "v{version}, generated:\n{other}"
        );
    }
}

#[test]
fn the_two_ir_versions_generate_identical_elm() {
    assert_eq!(regenerate("3"), regenerate("4"));
}

/// The strongest check: a reference in the regenerated source resolves to the
/// same fully qualified name it had, because the whole document comes back.
#[test]
fn compiling_the_generated_elm_yields_the_same_distribution() {
    for version in ["3", "4"] {
        let original = compile(
            vec![document(TYPES_URI, TYPES), document(OTHER_URI, OTHER)],
            version,
        );
        let (types, other) = regenerate(version);
        let again = compile(
            vec![document(TYPES_URI, &types), document(OTHER_URI, &other)],
            version,
        );

        assert_eq!(again.ir, original.ir, "v{version}, generated:\n{types}");
    }
}

#[test]
fn printing_the_generated_elm_again_changes_nothing() {
    for version in ["3", "4"] {
        let (types, other) = regenerate(version);
        for generated in [types, other] {
            assert_eq!(
                print(&to_ast(&generated)),
                generated,
                "v{version}, generated:\n{generated}"
            );
        }
    }
}

/// A package whose modules are *not* named after it, which is what a package
/// called `local/example` holding a module `Example` is.
///
/// The frontend strips nothing, because `Example` does not start with
/// `Local.Example`, so the IR module path is `Example`. The backend always
/// writes the package path back on, so the generated module is
/// `Local.Example.Example` — a different name from the one that was compiled.
/// That is the price of a symmetric rule, and it is only paid once: compiling
/// the generated source under the same package strips the prefix again and
/// yields the very same distribution.
#[test]
fn a_package_that_does_not_prefix_its_modules_still_round_trips() {
    const EXAMPLE: &str = "module Example exposing (Id)\n\ntype alias Id = String\n";

    for version in ["3", "4"] {
        let original = compile_as(
            "local/example",
            vec![document("file:///work/Example.elm", EXAMPLE)],
            version,
        );
        let generated = generate(original.ir.clone().expect("a distribution"));

        let paths: Vec<&str> = generated
            .artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect();
        assert_eq!(paths, ["src/Local/Example/Example.elm"], "v{version}");
        let source = artifact(&generated, "src/Local/Example/Example.elm");
        assert!(
            source.starts_with("module Local.Example.Example exposing (Id)\n"),
            "v{version}, generated:\n{source}"
        );

        let again = compile_as(
            "local/example",
            vec![document("file:///work/Example.elm", &source)],
            version,
        );
        assert_eq!(again.ir, original.ir, "v{version}, generated:\n{source}");
    }
}

/// The doc comments a Morphir document does keep come back — on the module, on
/// an alias, and on a custom type alike.
#[test]
fn doc_comments_survive() {
    for version in ["3", "4"] {
        let (types, _) = regenerate(version);

        for doc in [
            "{-| Module docs. -}",
            "{-| An account. -}",
            "{-| Account lifecycle. -}",
        ] {
            assert!(types.contains(doc), "v{version} lost {doc}:\n{types}");
        }
    }
}

/// The shape of the generated source, pinned: this is the import and
/// qualification policy the backend commits to. A module that is not reachable
/// through the prelude's implicit imports is imported by its full path and its
/// types are written qualified by that same full path — never aliased, never
/// exposed — so that no two imports can make one bare name ambiguous.
#[test]
fn the_generated_source_is_elm_format_shaped() {
    let (types, other) = regenerate("3");

    assert_eq!(
        types,
        "module My.Domain.Types exposing (Account, Status(..), Id)\n\
         \n\
         {-| Module docs. -}\n\
         \n\
         import Dict\n\
         import My.Other\n\
         \n\
         \n\
         {-| An account. -}\n\
         type alias Account a =\n\
         \x20   { id : Id\n\
         \x20   , tags : List String\n\
         \x20   , extra : Dict.Dict String (Maybe a)\n\
         \x20   , pair : ( Int, My.Other.Thing )\n\
         \x20   , f : Int -> { r | name : String } -> ()\n\
         \x20   }\n\
         \n\
         \n\
         {-| Account lifecycle. -}\n\
         type Status\n\
         \x20   = Active\n\
         \x20   | Closed String Int\n\
         \x20   | Pending { reason : String }\n\
         \n\
         \n\
         type alias Id =\n\
         \x20   String\n",
        "generated:\n{types}"
    );

    assert_eq!(
        other,
        "module My.Other exposing (Thing)\n\
         \n\
         \n\
         type Thing\n\
         \x20   = Thing\n",
        "generated:\n{other}"
    );
}
