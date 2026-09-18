//! Cucumber acceptance for the native Elm binding.
//!
//! Four feature files drive the same `NativeExtension::frontend_backend`
//! handle the daemon holds: the type declarations a module yields in both IR
//! versions, the diagnostics a bad module yields, the Elm-to-Elm round trip,
//! and the seven incremental cases.

use cucumber::{World, given, then, when};
use morphir_elm_binding::ElmExtension;
use morphir_extension_sdk::prelude::*;
use serde_json::Value;

const TYPES: &str = include_str!("fixtures/Types.elm");
const OTHER: &str = "module My.Other exposing (Thing)\n\ntype Thing = Thing\n";

const TYPES_URI: &str = "file:///work/My/Domain/Types.elm";
const OTHER_URI: &str = "file:///work/My/Other.elm";
const INVALID_URI: &str = "file:///work/Invalid.elm";

const INVALID: &str =
    include_str!("../../morphir-daemon/tests/fixtures/morphir-elm-extension/Invalid.elm");

// The two-module package the incremental cases run on: `A` imports `B` and
// aliases one of its types.
const A_URI: &str = "file:///work/A.elm";
const B_URI: &str = "file:///work/B.elm";

const A: &str = "module A exposing (T)\n\nimport B\n\ntype alias T = B.U\n";
const A_CHANGED: &str =
    "module A exposing (T, S)\n\nimport B\n\ntype alias T = B.U\n\n\ntype alias S = Int\n";
const B: &str = "module B exposing (U)\n\ntype alias U = Int\n";
const B_BROKEN: &str = "module B exposing (U)\n\ntype alias U =\n";
const B_DOC: &str = "module B exposing (U)\n\n{-| A whole number. -}\ntype alias U = Int\n";
const B_FLOAT: &str = "module B exposing (U)\n\ntype alias U = Float\n";
const B_WIDER: &str = "module B exposing (U, V)\n\ntype alias U = Int\n\n\ntype alias V = String\n";

fn a_source(name: &str) -> &'static str {
    match name {
        "original" => A,
        "changed" => A_CHANGED,
        other => panic!("no source named `{other}` for module A"),
    }
}

fn b_source(name: &str) -> &'static str {
    match name {
        "original" => B,
        "broken" => B_BROKEN,
        "documented" => B_DOC,
        "retyped" => B_FLOAT,
        "widened" => B_WIDER,
        other => panic!("no source named `{other}` for module B"),
    }
}

fn document(uri: &str, text: &str) -> SourceDocument {
    SourceDocument {
        uri: uri.into(),
        language_id: "elm".into(),
        version: 1,
        text: text.into(),
    }
}

// ----------------------------------------------------------------------------
// Reading either IR version
// ----------------------------------------------------------------------------

/// A name as the IR spells it, with the writing style removed: an Elm `Foo_Bar`,
/// a classic `["foo","bar"]` and a v4 `fooBar` all read back as `foobar`.
fn canonical(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn canonical_module(name: &str) -> String {
    name.split(['.', '/'])
        .map(canonical)
        .collect::<Vec<_>>()
        .join(".")
}

/// A classic name is a list of words; a module path is a list of those.
fn classic_name(value: &Value) -> String {
    canonical(
        &value
            .as_array()
            .expect("a classic name is a list of words")
            .iter()
            .filter_map(Value::as_str)
            .collect::<String>(),
    )
}

fn classic_path(value: &Value) -> String {
    value
        .as_array()
        .expect("a classic module path is a list of names")
        .iter()
        .map(classic_name)
        .collect::<Vec<_>>()
        .join(".")
}

/// What a scenario asks about one declaration, read out of either IR version.
#[derive(Debug)]
enum TypeInfo {
    Alias { params: usize },
    Custom { constructors: Vec<String> },
}

fn type_info(ir: &Value, version: &str, module: &str, type_name: &str) -> TypeInfo {
    let wanted_module = canonical_module(module);
    let wanted_type = canonical(type_name);
    if version == "3" {
        let modules = ir["distribution"][3]["modules"]
            .as_array()
            .expect("a classic package definition with a module list");
        let module = modules
            .iter()
            .find(|entry| classic_path(&entry[0]) == wanted_module)
            .unwrap_or_else(|| panic!("no module `{wanted_module}` in the distribution"));
        let types = module[1]["value"]["types"]
            .as_array()
            .expect("a classic module definition with a type list");
        let declaration = types
            .iter()
            .find(|entry| classic_name(&entry[0]) == wanted_type)
            .unwrap_or_else(|| panic!("no type `{wanted_type}` in module `{wanted_module}`"));
        let definition = &declaration[1]["value"]["value"];
        match definition[0].as_str() {
            Some("TypeAliasDefinition") => TypeInfo::Alias {
                params: definition[1]
                    .as_array()
                    .expect("a type parameter list")
                    .len(),
            },
            Some("CustomTypeDefinition") => TypeInfo::Custom {
                constructors: definition[2]["value"]
                    .as_array()
                    .expect("a constructor list")
                    .iter()
                    .map(|constructor| classic_name(&constructor[0]))
                    .collect(),
            },
            other => panic!("unexpected classic type definition tag: {other:?}"),
        }
    } else {
        let modules = ir["distribution"]["Library"]["def"]["modules"]
            .as_object()
            .expect("a v4 library with a module map");
        let module = modules
            .iter()
            .find(|(key, _)| canonical_module(key) == wanted_module)
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("no module `{wanted_module}` in the distribution"));
        let types = access_controlled(module)["types"]
            .as_object()
            .expect("a v4 module definition with a type map");
        let declaration = types
            .iter()
            .find(|(key, _)| canonical(key) == wanted_type)
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("no type `{wanted_type}` in module `{wanted_module}`"));
        let definition = access_controlled(declaration);
        if let Some(alias) = definition.get("TypeAliasDefinition") {
            TypeInfo::Alias {
                params: alias["typeParams"]
                    .as_array()
                    .expect("a type parameter list")
                    .len(),
            }
        } else {
            TypeInfo::Custom {
                constructors: definition["CustomTypeDefinition"]["constructors"]
                    .as_object()
                    .expect("a constructor map")
                    .keys()
                    .map(|name| canonical(name))
                    .collect(),
            }
        }
    }
}

/// v4 writes access as the single member's tag, so the value behind it is the
/// definition itself.
fn access_controlled(value: &Value) -> &Value {
    let members = value.as_object().expect("an access-controlled value");
    assert_eq!(
        members.len(),
        1,
        "an access tag is the only member: {value}"
    );
    members.values().next().expect("the tagged value")
}

/// The modules a classic distribution carries, in the order it writes them.
fn library_modules(ir: &Value) -> Vec<String> {
    ir["distribution"][3]["modules"]
        .as_array()
        .expect("a classic package definition with a module list")
        .iter()
        .map(|entry| classic_path(&entry[0]))
        .collect()
}

// ----------------------------------------------------------------------------
// The world
// ----------------------------------------------------------------------------

#[derive(Debug, Default, World)]
struct ElmWorld {
    documents: Vec<SourceDocument>,
    prelude: Option<Value>,
    version: String,
    compiled: Option<CompileResult>,
    generated: Option<GenerateResult>,
    baseline: Option<CompileBaseline>,
    first: Option<CompileResult>,
}

impl ElmWorld {
    fn compile(&mut self, version: &str) {
        self.version = version.into();
        self.compiled = Some(compile(
            self.documents.clone(),
            version,
            self.prelude.as_ref(),
            self.baseline.clone(),
        ));
    }

    fn result(&self) -> &CompileResult {
        self.compiled.as_ref().expect("compile the module first")
    }

    fn ir(&self) -> &Value {
        self.result()
            .ir
            .as_ref()
            .expect("a successful compile carries a distribution")
    }

    fn module(&self, name: &str) -> &ModuleResult {
        let result = self.result();
        result
            .module_results
            .iter()
            .find(|module| module.name == name)
            .unwrap_or_else(|| panic!("no result for module {name} in {:?}", result.module_results))
    }

    fn first_module(&self, name: &str) -> &ModuleResult {
        self.first
            .as_ref()
            .expect("a first run to compare against")
            .module_results
            .iter()
            .find(|module| module.name == name)
            .unwrap_or_else(|| panic!("no first-run result for module {name}"))
    }

    /// Compiles with the held baseline and folds the outcome back into it, the
    /// way a daemon would between edits.
    fn extend_baseline(&mut self, a: &str, b: &str) {
        let held = self.baseline.clone().unwrap_or_default();
        let result = compile(
            vec![document(A_URI, a_source(a)), document(B_URI, b_source(b))],
            "3",
            None,
            Some(held.clone()),
        );
        self.baseline = Some(baseline_from(&held, &result));
        if self.first.is_none() {
            self.first = Some(result);
        }
    }
}

fn compile(
    documents: Vec<SourceDocument>,
    ir_version: &str,
    prelude: Option<&Value>,
    baseline: Option<CompileBaseline>,
) -> CompileResult {
    let mut extra = std::collections::HashMap::new();
    if let Some(prelude) = prelude {
        extra.insert("elmPrelude".to_string(), prelude.clone());
    }
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    extension
        .frontend()
        .unwrap()
        .compile(CompileRequest {
            language_id: "elm".into(),
            documents,
            package: CompilePackage {
                name: "local/example".into(),
                exposed_modules: None,
            },
            dependencies: vec![],
            options: CompileOptions {
                types_only: false,
                ir_version: ir_version.into(),
                extra,
            },
            baseline,
        })
        .unwrap()
}

/// The baseline a host holds after `result`: freshly compiled modules replace
/// their entry, reused or failed modules keep the entry they had, and modules
/// the request no longer names are dropped.
fn baseline_from(previous: &CompileBaseline, result: &CompileResult) -> CompileBaseline {
    let mut modules: Vec<BaselineModule> = previous
        .modules
        .iter()
        .filter(|entry| {
            result
                .module_results
                .iter()
                .any(|module| module.name == entry.name)
        })
        .cloned()
        .collect();

    for module in &result.module_results {
        if module.status != ModuleStatus::Compiled {
            continue;
        }
        let entry = BaselineModule {
            name: module.name.clone(),
            uri: module.uri.clone(),
            source_digest: module
                .source_digest
                .clone()
                .expect("a compiled module reports its source digest"),
            interface_digest: module
                .interface_digest
                .clone()
                .expect("a compiled module reports its interface digest"),
            depends_on: module.depends_on.clone(),
            ir: module
                .ir
                .clone()
                .expect("a compiled module reports its module IR"),
        };
        match modules.iter_mut().find(|held| held.name == entry.name) {
            Some(held) => *held = entry,
            None => modules.push(entry),
        }
    }

    CompileBaseline { modules }
}

fn status(word: &str) -> ModuleStatus {
    match word {
        "compiled" => ModuleStatus::Compiled,
        "unchanged" => ModuleStatus::Unchanged,
        "failed" => ModuleStatus::Failed,
        "blocked" => ModuleStatus::Blocked,
        other => panic!("no module status named `{other}`"),
    }
}

fn diagnostics<'a>(result: &'a CompileResult, code: &str) -> Vec<&'a Diagnostic> {
    result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.as_deref() == Some(code))
        .collect()
}

// ----------------------------------------------------------------------------
// Sources
// ----------------------------------------------------------------------------

#[given("the Elm module fixture Types.elm")]
fn types_fixture(world: &mut ElmWorld) {
    world.documents = vec![document(TYPES_URI, TYPES), document(OTHER_URI, OTHER)];
}

#[given("an Elm module that does not parse")]
fn invalid_fixture(world: &mut ElmWorld) {
    world.documents = vec![document(INVALID_URI, INVALID)];
}

#[given("an Elm module aliasing Int")]
fn int_alias(world: &mut ElmWorld) {
    world.documents = vec![document(
        "file:///work/A.elm",
        "module A exposing (Id)\n\ntype alias Id = Int\n",
    )];
}

#[given("two imported modules that both expose the type T")]
fn ambiguous_imports(world: &mut ElmWorld) {
    world.documents = vec![
        document(
            "file:///work/A.elm",
            "module A exposing (U)\n\nimport X exposing (..)\nimport Y exposing (..)\n\n\ntype alias U = T\n",
        ),
        document(
            "file:///work/X.elm",
            "module X exposing (T)\n\ntype T = T\n",
        ),
        document(
            "file:///work/Y.elm",
            "module Y exposing (T)\n\ntype T = T\n",
        ),
    ];
}

#[given(expr = "the Elm prelude option {string}")]
fn prelude_option(world: &mut ElmWorld, prelude: String) {
    world.prelude = Some(Value::String(prelude));
}

// ----------------------------------------------------------------------------
// Compiling and generating
// ----------------------------------------------------------------------------

#[when(expr = "I compile it to Morphir IR version {string}")]
fn compile_it(world: &mut ElmWorld, version: String) {
    world.compile(&version);
}

#[when("I generate Elm from the compiled model")]
fn generate_elm(world: &mut ElmWorld) {
    let result = world.result();
    assert!(result.success, "{:?}", result.diagnostics);
    let extension = NativeExtension::frontend_backend(ElmExtension).unwrap();
    let generated = extension
        .backend()
        .unwrap()
        .generate(GenerateRequest {
            ir: result.ir.clone().expect("a distribution"),
            target: "elm".into(),
            options: Default::default(),
        })
        .unwrap();
    assert!(generated.success, "{:?}", generated.diagnostics);
    world.generated = Some(generated);
}

// ----------------------------------------------------------------------------
// Type declarations
// ----------------------------------------------------------------------------

#[then(expr = "the IR contains type {string} as an alias with {int} type parameter")]
fn alias_with_parameters(world: &mut ElmWorld, name: String, params: usize) {
    let result = world.result();
    assert!(result.success, "{:?}", result.diagnostics);
    match type_info(world.ir(), &world.version, "My.Domain.Types", &name) {
        TypeInfo::Alias { params: actual } => assert_eq!(actual, params),
        other => panic!("`{name}` is not an alias: {other:?}"),
    }
}

#[then(expr = "the IR contains custom type {string} with constructors {string}")]
fn custom_with_constructors(world: &mut ElmWorld, name: String, expected: String) {
    match type_info(world.ir(), &world.version, "My.Domain.Types", &name) {
        TypeInfo::Custom { constructors } => assert_eq!(
            constructors,
            expected
                .split(',')
                .map(|constructor| canonical(constructor.trim()))
                .collect::<Vec<_>>()
        ),
        other => panic!("`{name}` is not a custom type: {other:?}"),
    }
}

#[then(expr = "value {string} is reported as skipped")]
fn value_skipped(world: &mut ElmWorld, name: String) {
    let result = world.result();
    let skipped = diagnostics(result, "ELM_VALUE_SKIPPED");
    assert!(
        skipped
            .iter()
            .any(|diagnostic| diagnostic.message.contains(&name)),
        "{name} is not reported as skipped: {:?}",
        result.diagnostics
    );
    assert!(
        skipped
            .iter()
            .all(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
    );
    assert!(result.success, "a skipped value is not a failure");
}

// ----------------------------------------------------------------------------
// Diagnostics
// ----------------------------------------------------------------------------

#[then("compilation succeeds")]
fn succeeds(world: &mut ElmWorld) {
    let result = world.result();
    assert!(result.success, "{:?}", result.diagnostics);
}

#[then("compilation fails")]
fn fails(world: &mut ElmWorld) {
    let result = world.result();
    assert!(!result.success, "{:?}", result.diagnostics);
    assert!(
        result
            .diagnostics
            .iter()
            .chain(
                result
                    .module_results
                    .iter()
                    .flat_map(|module| module.diagnostics.iter())
            )
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
    );
}

#[then(expr = "diagnostic {string} is reported at line {int} of {string}")]
fn diagnostic_at(world: &mut ElmWorld, code: String, line: u32, uri: String) {
    let result = world.result();
    let reported = diagnostics(result, &code);
    assert!(!reported.is_empty(), "{:?}", result.diagnostics);
    let location = reported[0]
        .location
        .as_ref()
        .expect("a source diagnostic carries a location");
    assert_eq!(location.uri, uri);
    assert_eq!(location.range.start.line, line);
}

#[then(expr = "diagnostic {string} mentions {string}")]
fn diagnostic_mentions(world: &mut ElmWorld, code: String, text: String) {
    let result = world.result();
    let reported = diagnostics(result, &code);
    assert!(!reported.is_empty(), "{:?}", result.diagnostics);
    assert!(
        reported
            .iter()
            .any(|diagnostic| diagnostic.message.contains(&text)),
        "no `{code}` diagnostic mentions `{text}`: {reported:?}"
    );
}

// ----------------------------------------------------------------------------
// Round trip
// ----------------------------------------------------------------------------

#[then("reading the generated Elm yields the same distribution")]
fn same_distribution(world: &mut ElmWorld) {
    let generated = world.generated.as_ref().expect("generate Elm first");
    let documents = generated
        .artifacts
        .iter()
        .map(|artifact| document(&artifact.path, &artifact.content))
        .collect();
    let again = compile(documents, &world.version, None, None);
    assert!(again.success, "{:?}", again.diagnostics);
    assert_eq!(again.ir, world.result().ir);
}

#[then("the generated Elm is one artifact per module")]
fn one_artifact_per_module(world: &mut ElmWorld) {
    let generated = world.generated.as_ref().expect("generate Elm first");
    let mut paths: Vec<&str> = generated
        .artifacts
        .iter()
        .map(|artifact| artifact.path.as_str())
        .collect();
    paths.sort_unstable();
    assert_eq!(paths, ["src/My/Domain/Types.elm", "src/My/Other.elm"]);
}

// ----------------------------------------------------------------------------
// Incremental
// ----------------------------------------------------------------------------

#[given("no baseline")]
fn no_baseline(world: &mut ElmWorld) {
    world.baseline = None;
}

#[given(expr = "a baseline from compiling A {string} and B {string}")]
fn a_baseline(world: &mut ElmWorld, a: String, b: String) {
    world.extend_baseline(&a, &b);
}

#[given(expr = "a further baseline from compiling A {string} and B {string}")]
fn a_further_baseline(world: &mut ElmWorld, a: String, b: String) {
    world.extend_baseline(&a, &b);
}

#[when(expr = "I compile A {string} and B {string}")]
fn compile_pair(world: &mut ElmWorld, a: String, b: String) {
    world.documents = vec![document(A_URI, a_source(&a)), document(B_URI, b_source(&b))];
    world.compile("3");
}

#[when(expr = "I compile A {string} with B deleted")]
fn compile_without_b(world: &mut ElmWorld, a: String) {
    world.documents = vec![document(A_URI, a_source(&a))];
    world.compile("3");
}

#[then(expr = "module {string} is {word}")]
fn module_status(world: &mut ElmWorld, name: String, expected: String) {
    assert_eq!(world.module(&name).status, status(&expected));
}

#[then(expr = "module {string} reports diagnostic {string}")]
fn module_diagnostic(world: &mut ElmWorld, name: String, code: String) {
    let module = world.module(&name);
    assert!(
        module
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_deref() == Some(code.as_str())),
        "module {name} has no `{code}`: {:?}",
        module.diagnostics
    );
}

#[then(expr = "module {string} reports no IR")]
fn module_without_ir(world: &mut ElmWorld, name: String) {
    assert!(world.module(&name).ir.is_none());
}

#[then(expr = "module {string} carries a located diagnostic")]
fn module_located_diagnostic(world: &mut ElmWorld, name: String) {
    assert!(
        world
            .module(&name)
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.location.is_some())
    );
}

#[then(expr = "the distribution contains modules {string}")]
fn distribution_modules(world: &mut ElmWorld, expected: String) {
    assert_eq!(
        library_modules(world.ir()),
        expected
            .split(',')
            .map(|name| canonical_module(name.trim()))
            .collect::<Vec<_>>()
    );
}

#[then(expr = "module {string} has a new interface digest")]
fn new_interface_digest(world: &mut ElmWorld, name: String) {
    assert_ne!(
        world.module(&name).interface_digest,
        world.first_module(&name).interface_digest
    );
}

#[then(expr = "module {string} keeps its interface digest")]
fn kept_interface_digest(world: &mut ElmWorld, name: String) {
    assert_eq!(
        world.module(&name).interface_digest,
        world.first_module(&name).interface_digest
    );
}

#[then(expr = "module {string} has a new source digest")]
fn new_source_digest(world: &mut ElmWorld, name: String) {
    assert_ne!(
        world.module(&name).source_digest,
        world.first_module(&name).source_digest
    );
}

#[then(expr = "the compile reports {int} module")]
fn module_count(world: &mut ElmWorld, count: usize) {
    assert_eq!(world.result().module_results.len(), count);
}

#[tokio::main]
async fn main() {
    ElmWorld::cucumber()
        .fail_on_skipped()
        .run_and_exit(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/features"))
        .await;
}
