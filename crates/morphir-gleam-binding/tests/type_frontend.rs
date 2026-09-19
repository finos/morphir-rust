use morphir_gleam_binding::frontend::parse_gleam;

#[test]
fn parses_aliases_imports_opaque_types_and_function_types() {
    let source = r#"
//// Models
import domain/customer.{type Customer as Buyer} as customer
/// An identifier.
pub type Id = Int
pub opaque type Box(a) { Box(value: a) }
pub type Handler = fn(customer.Customer, List(Buyer)) -> Result(Nil, String)
"#;
    let module = parse_gleam("models.gleam", source).expect("supported Gleam type declarations");
    assert_eq!(module.types.len(), 3);
}

#[test]
fn preserves_type_documentation_labels_and_function_parameter_types() {
    use morphir_common::vfs::MemoryVfs;
    use morphir_core::naming::{ModuleName, PackageName};
    use morphir_gleam_binding::frontend::GleamToMorphirVisitor;
    let module = parse_gleam("models.gleam", "/// Wrapped\npub opaque type Box(a) { Box(value: a) }\npub fn id(value: Int) -> Int { value }").unwrap();
    let visitor = GleamToMorphirVisitor::new(
        MemoryVfs::new(),
        "/out".into(),
        PackageName::parse("demo"),
        ModuleName::parse("models"),
    );
    let definition = visitor
        .build_module_definition(&module, morphir_core::ir::v4::Access::Public)
        .unwrap();
    let json = serde_json::to_string(&definition).unwrap();
    let morphir_core::ir::v4::TypeDefinition::CustomTypeDefinition { constructors, .. } =
        &definition.value.types["box"].value.value
    else {
        panic!("custom type")
    };
    assert_eq!(constructors.access, morphir_core::ir::v4::Access::Private);
    assert_eq!(constructors.value[0].args[0].name.to_string(), "value");
    assert!(json.contains("Wrapped"), "{json}");
    assert!(json.contains("morphir/SDK"), "{json}");
    assert!(
        definition.value.values["id"]
            .value
            .value
            .input_types
            .contains_key("value")
    );
}

#[test]
fn rejects_unknown_private_ambiguous_and_wrong_arity_types() {
    use morphir_core::naming::PackageName;
    use morphir_gleam_binding::frontend::resolver::resolve_modules;
    let package = PackageName::parse("demo");
    for source in [
        "pub type A = Missing",
        "pub type A = List(Int, String)",
        "pub type A = a",
    ] {
        let module = parse_gleam("models.gleam", source).unwrap();
        assert!(
            resolve_modules(&package, &[module], &Default::default()).is_err(),
            "{source}"
        );
    }
    let hidden = parse_gleam("hidden.gleam", "type Secret { Secret }").unwrap();
    let user = parse_gleam("user.gleam", "import hidden\npub type A = hidden.Secret").unwrap();
    assert!(resolve_modules(&package, &[hidden, user], &Default::default()).is_err());
    let a = parse_gleam("a.gleam", "pub type T { T }").unwrap();
    let b = parse_gleam("b.gleam", "pub type T { T }").unwrap();
    let user = parse_gleam(
        "user.gleam",
        "import a.{type T}\nimport b.{type T}\npub type A = T",
    )
    .unwrap();
    assert!(resolve_modules(&package, &[a, b, user], &Default::default()).is_err());
}

#[test]
fn rejects_alias_cycles_but_accepts_custom_recursion() {
    use morphir_core::naming::PackageName;
    use morphir_gleam_binding::frontend::resolver::resolve_modules;
    let package = PackageName::parse("demo");
    let aliases = parse_gleam("models.gleam", "pub type A = B\npub type B = List(A)").unwrap();
    assert!(resolve_modules(&package, &[aliases], &Default::default()).is_err());
    let recursive = parse_gleam(
        "models.gleam",
        "pub type Tree(a) { Leaf(a) Node(List(Tree(a))) }",
    )
    .unwrap();
    assert!(resolve_modules(&package, &[recursive], &Default::default()).is_ok());
}

#[test]
fn resolves_qualified_and_aliased_imports_and_sdk_types() {
    use morphir_core::naming::PackageName;
    use morphir_gleam_binding::frontend::{ast::TypeExpr, resolver::resolve_modules};
    let customer = parse_gleam("domain/customer.gleam", "pub type Customer { Customer }").unwrap();
    let user = parse_gleam("user.gleam", "import domain/customer.{type Customer as Buyer} as customer\npub type A = #(Buyer, customer.Customer, Int, Bool, Float, String, List(Int), Result(Int, String), Nil)").unwrap();
    let modules = resolve_modules(
        &PackageName::parse("demo"),
        &[customer, user],
        &Default::default(),
    )
    .unwrap();
    let TypeExpr::Tuple { elements } = &modules[1].types[0].body else {
        panic!("tuple")
    };
    let TypeExpr::Resolved { name, .. } = &elements[0] else {
        panic!("resolved imported type")
    };
    assert_eq!(name.module_path.to_string(), "domain/customer");
    let TypeExpr::Resolved { name, .. } = &elements[2] else {
        panic!("SDK Int")
    };
    assert_eq!(name.package_path.to_string(), "morphir/SDK");
    assert!(matches!(elements[8], TypeExpr::Unit));
}

#[test]
fn comparison_distinguishes_constructor_labels_and_opacity() {
    use morphir_gleam_binding::frontend::modules_equivalent;
    let public = parse_gleam("models.gleam", "pub type Box { Box(value: Int) }").unwrap();
    let opaque = parse_gleam("models.gleam", "pub opaque type Box { Box(value: Int) }").unwrap();
    let renamed = parse_gleam("models.gleam", "pub type Box { Box(other: Int) }").unwrap();
    assert!(!modules_equivalent(&public, &opaque));
    assert!(!modules_equivalent(&public, &renamed));
}

#[test]
fn resolves_baseline_and_dependency_types_in_single_module_mode() {
    use morphir_common::vfs::MemoryVfs;
    use morphir_core::{
        ir::v4::{Access, PackageDefinition},
        naming::{ModuleName, PackageName},
    };
    use morphir_gleam_binding::frontend::{
        GleamToMorphirVisitor,
        resolver::{resolve_modules, resolve_one},
    };
    let package = PackageName::parse("demo");
    let original = parse_gleam(
        "domain/customer.gleam",
        "pub type Customer(a) { Customer(value: a) }\ntype Hidden { Hidden }",
    )
    .unwrap();
    let resolved = resolve_modules(&package, &[original], &Default::default())
        .unwrap()
        .remove(0);
    let definition = GleamToMorphirVisitor::new(
        MemoryVfs::new(),
        "/out".into(),
        package.clone(),
        ModuleName::parse("domain/customer"),
    )
    .build_module_definition(&resolved, Access::Public)
    .unwrap();
    let local = indexmap::indexmap! {"domain/customer".to_owned() => definition};
    let source = parse_gleam(
        "main.gleam",
        "import domain/customer as customer\npub type Buyer = customer.Customer(Int)",
    )
    .unwrap();
    assert!(resolve_one(&package, &source, &local, &Default::default()).is_ok());
    let hidden = parse_gleam(
        "main.gleam",
        "import domain/customer as customer\npub type Secret = customer.Hidden",
    )
    .unwrap();
    assert!(resolve_one(&package, &hidden, &local, &Default::default()).is_err());
    let dependency = PackageDefinition { modules: local }.to_specification();
    let dependencies = indexmap::indexmap! { "vendor".to_owned() => dependency };
    let resolved = resolve_one(&package, &source, &Default::default(), &dependencies).unwrap();
    let morphir_gleam_binding::frontend::ast::TypeExpr::Resolved { name, .. } =
        &resolved.types[0].body
    else {
        panic!("dependency reference")
    };
    assert_eq!(name.package_path.to_string(), "vendor");
    assert!(resolve_one(&package, &hidden, &Default::default(), &dependencies).is_err());
}

#[test]
fn rejects_unknown_types_in_function_and_local_annotations() {
    use morphir_core::naming::PackageName;
    use morphir_gleam_binding::frontend::resolver::resolve_modules;
    for source in [
        "pub fn f(x: Missing) -> Int { 1 }",
        "pub fn f() { let x: Missing = 1 x }",
    ] {
        let module = parse_gleam("main.gleam", source).unwrap();
        assert!(
            resolve_modules(&PackageName::parse("demo"), &[module], &Default::default()).is_err()
        );
    }
}

#[test]
fn gleam_option_import_maps_to_sdk_maybe() {
    use morphir_core::naming::PackageName;
    use morphir_gleam_binding::frontend::{ast::TypeExpr, resolver::resolve_modules};
    let module = parse_gleam(
        "main.gleam",
        "import gleam/option.{type Option}\npub type Value = Option(Int)",
    )
    .unwrap();
    let resolved =
        resolve_modules(&PackageName::parse("demo"), &[module], &Default::default()).unwrap();
    let TypeExpr::Resolved { name, .. } = &resolved[0].types[0].body else {
        panic!("resolved option")
    };
    assert_eq!(name.to_string(), "morphir/SDK:maybe#maybe");
}

#[test]
fn rejects_opaque_aliases_and_duplicate_constructors() {
    use morphir_core::naming::PackageName;
    use morphir_gleam_binding::frontend::resolver::resolve_modules;
    assert!(parse_gleam("main.gleam", "pub opaque type Id = Int").is_err());
    let module = parse_gleam("main.gleam", "pub type A { A A }").unwrap();
    assert!(resolve_modules(&PackageName::parse("demo"), &[module], &Default::default()).is_err());
}

fn compile_sources(sources: &[(&str, &str)]) -> morphir_extension_sdk::CompileResult {
    use morphir_extension_sdk::prelude::*;
    use morphir_gleam_binding::GleamExtension;
    GleamExtension
        .compile(CompileRequest {
            language_id: "gleam".into(),
            documents: sources
                .iter()
                .map(|(name, text)| SourceDocument {
                    uri: format!("file:///src/{name}.gleam"),
                    language_id: "gleam".into(),
                    version: 1,
                    text: (*text).into(),
                })
                .collect(),
            package: CompilePackage {
                name: "demo".into(),
                exposed_modules: None,
            },
            dependencies: vec![],
            options: CompileOptions {
                ir_version: "4".into(),
                types_only: false,
                extra: [("emitParseStage".into(), serde_json::json!(false))].into(),
            },
            baseline: None,
        })
        .unwrap()
}

#[test]
fn public_compile_rejects_import_cycles_and_normalized_type_duplicates() {
    let cycle = compile_sources(&[
        ("a", "import b\npub type A = b.B"),
        ("b", "import a\npub type B = a.A"),
    ]);
    assert!(!cycle.success);
    assert!(
        cycle
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_deref() == Some("GLEAM_IMPORT_CYCLE")),
        "{:?}",
        cycle.diagnostics
    );
    let duplicate = compile_sources(&[(
        "main",
        "pub type FooBar { First }\npub type Foo_Bar { Second }",
    )]);
    assert!(!duplicate.success);
    assert!(
        duplicate
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_deref() == Some("GLEAM_DUPLICATE_TYPE")),
        "{:?}",
        duplicate.diagnostics
    );
}

#[test]
fn nested_argument_block_annotations_are_resolved() {
    let result = compile_sources(&[("main", "pub fn f() { identity({ let x: Missing = 1 x }) }")]);
    assert!(
        !result.success,
        "unknown annotation inside an argument must fail"
    );
}

#[test]
fn anonymous_record_type_source_has_an_explicit_diagnostic() {
    let result = compile_sources(&[("main", "pub type Person = { name: String }")]);
    assert!(!result.success);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_deref() == Some("GLEAM_UNSUPPORTED_TYPE")),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn function_type_diagnostics_point_to_the_function_declaration() {
    let result = compile_sources(&[(
        "main",
        "//// Module docs\n\npub fn broken(x: Missing) -> Int { 1 }",
    )]);
    let diagnostic = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code.as_deref() == Some("GLEAM_RESOLVE_NOT_FOUND"))
        .unwrap();
    assert_eq!(diagnostic.location.as_ref().unwrap().range.start.line, 2);
}

#[test]
fn gleam_dict_and_set_imports_map_to_sdk_types() {
    use morphir_core::naming::PackageName;
    use morphir_gleam_binding::frontend::{ast::TypeExpr, resolver::resolve_modules};
    let module = parse_gleam("main.gleam", "import gleam/dict as d\nimport gleam/set.{type Set}\npub type Index = #(d.Dict(String, Int), Set(Int))").unwrap();
    let resolved =
        resolve_modules(&PackageName::parse("demo"), &[module], &Default::default()).unwrap();
    let TypeExpr::Tuple { elements } = &resolved[0].types[0].body else {
        panic!("tuple")
    };
    for (ty, expected) in elements
        .iter()
        .zip(["morphir/SDK:dict#dict", "morphir/SDK:set#set"])
    {
        let TypeExpr::Resolved { name, .. } = ty else {
            panic!("resolved SDK type")
        };
        assert_eq!(name.to_string(), expected);
    }
}

#[test]
fn positional_constructor_roundtrip_is_semantically_equivalent() {
    use morphir_gleam_binding::{frontend::modules_equivalent, roundtrip::roundtrip_gleam};
    let result = roundtrip_gleam("pub type Box { Box(Int, String) }").unwrap();
    assert!(
        modules_equivalent(&result.original, &result.regenerated),
        "{}",
        result.generated_code
    );
}
