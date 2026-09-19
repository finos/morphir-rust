use indexmap::IndexMap;
use morphir_core::ir::v4::*;
use morphir_core::naming::{FQName, ModuleName, Name, PackageName};
use morphir_extension_sdk::prelude::*;
use morphir_gleam_binding::{GleamExtension, frontend::parse_gleam};
use std::collections::HashMap;

fn reference(package: &str, module: &str, name: &str, args: Vec<Type>) -> Type {
    Type::Reference(
        TypeAttributes::default(),
        FQName {
            package_path: PackageName::parse(package).into(),
            module_path: ModuleName::parse(module).into(),
            local_name: Name::from(name),
        },
        args,
    )
}

fn generate(definition: TypeDefinition) -> GenerateResult {
    let module = ModuleDefinition {
        types: IndexMap::from([(
            "customer-id".into(),
            AccessControlled {
                access: Access::Public,
                value: Documented::new(
                    Some("Customer identifier.\nKept stable.".into()),
                    definition,
                ),
            },
        )]),
        values: IndexMap::new(),
        doc: Some("Domain models.".into()),
    };
    let ir = IRFile {
        format_version: FormatVersion::String("4.0.0".into()),
        distribution: Distribution::Library(LibraryContent {
            package_name: PackageName::parse("example/package"),
            dependencies: IndexMap::new(),
            def: PackageDefinition {
                modules: IndexMap::from([(
                    "domain/customer-records".into(),
                    AccessControlled {
                        access: Access::Public,
                        value: module,
                    },
                )]),
            },
        }),
    };
    GleamExtension
        .generate(GenerateRequest {
            ir: serde_json::to_value(ir).unwrap(),
            target: "gleam".into(),
            options: HashMap::new(),
        })
        .unwrap()
}

#[test]
fn aliases_have_real_bodies_readable_names_and_docs() {
    let result = generate(TypeDefinition::TypeAliasDefinition {
        type_params: vec![],
        type_expr: reference("morphir/SDK", "basics", "int", vec![]),
    });
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.artifacts[0].path, "domain/customer_records.gleam");
    let code = &result.artifacts[0].content;
    assert!(code.contains("//// Domain models."), "{code}");
    assert!(
        code.contains("/// Customer identifier.\n/// Kept stable."),
        "{code}"
    );
    assert!(code.contains("pub type CustomerId = Int"), "{code}");
    parse_gleam(&result.artifacts[0].path, code).expect("valid generated Gleam");
}

#[test]
fn opaque_constructors_preserve_labels() {
    let result = generate(TypeDefinition::CustomTypeDefinition {
        type_params: vec![Name::from("a")],
        constructors: AccessControlled {
            access: Access::Private,
            value: vec![ConstructorDefinition {
                name: Name::from("customer-id"),
                args: vec![ConstructorArg {
                    name: Name::from("raw-value"),
                    arg_type: Type::Variable(TypeAttributes::default(), Name::from("a")),
                }],
            }],
        },
    });
    assert!(result.success, "{:?}", result.diagnostics);
    let code = &result.artifacts[0].content;
    assert!(code.contains("pub opaque type CustomerId(a)"), "{code}");
    assert!(code.contains("CustomerId(raw_value: a)"), "{code}");
}

#[test]
fn references_import_siblings_external_modules_and_sdk_types() {
    let result = generate(TypeDefinition::TypeAliasDefinition {
        type_params: vec![],
        type_expr: Type::Tuple(
            TypeAttributes::default(),
            vec![
                reference(
                    "example/package",
                    "domain/customer-records",
                    "local-type",
                    vec![],
                ),
                reference(
                    "example/package",
                    "domain/shared-types",
                    "shared-id",
                    vec![],
                ),
                reference("vendor/library", "remote-types", "external-id", vec![]),
                reference(
                    "morphir/SDK",
                    "maybe",
                    "maybe",
                    vec![reference("morphir/SDK", "string", "string", vec![])],
                ),
            ],
        ),
    });
    assert!(result.success, "{:?}", result.diagnostics);
    let code = &result.artifacts[0].content;
    assert!(code.contains("import domain/shared_types"), "{code}");
    assert!(code.contains("import remote_types"), "{code}");
    assert!(code.contains("import gleam/option"), "{code}");
    assert!(code.contains("LocalType"), "{code}");
    assert!(code.contains("shared_types.SharedId"), "{code}");
    assert!(code.contains("remote_types.ExternalId"), "{code}");
    assert!(code.contains("option.Option(String)"), "{code}");
}

#[test]
fn extensible_and_anonymous_nested_records_fail_without_artifacts() {
    for type_expr in [
        Type::Tuple(
            TypeAttributes::default(),
            vec![Type::Record(TypeAttributes::default(), vec![])],
        ),
        Type::ExtensibleRecord(TypeAttributes::default(), Name::from("row"), vec![]),
    ] {
        let result = generate(TypeDefinition::TypeAliasDefinition {
            type_params: vec![],
            type_expr,
        });
        assert!(!result.success);
        assert!(result.artifacts.is_empty());
        assert!(result.diagnostics[0].message.contains("structural record"));
    }
}

#[test]
fn sdk_result_reorders_error_and_success_arguments() {
    let result = generate(TypeDefinition::TypeAliasDefinition {
        type_params: vec![],
        type_expr: reference(
            "morphir/SDK",
            "result",
            "result",
            vec![
                reference("morphir/SDK", "string", "string", vec![]),
                reference("morphir/SDK", "basics", "int", vec![]),
            ],
        ),
    });
    assert!(result.success, "{:?}", result.diagnostics);
    assert!(
        result.artifacts[0].content.contains("Result(Int, String)"),
        "{}",
        result.artifacts[0].content
    );
}

#[test]
fn imports_with_the_same_leaf_get_distinct_aliases() {
    let result = generate(TypeDefinition::TypeAliasDefinition {
        type_params: vec![],
        type_expr: Type::Tuple(
            TypeAttributes::default(),
            vec![
                reference("example/package", "one/types", "first", vec![]),
                reference("example/package", "two/types", "second", vec![]),
            ],
        ),
    });
    assert!(result.success, "{:?}", result.diagnostics);
    let code = &result.artifacts[0].content;
    assert!(code.contains("import one/types\n"), "{code}");
    assert!(code.contains("import two/types as types_2\n"), "{code}");
    assert!(code.contains("#(types.First, types_2.Second)"), "{code}");
}

#[test]
fn sdk_types_without_exact_gleam_equivalents_are_diagnostics() {
    let result = generate(TypeDefinition::TypeAliasDefinition {
        type_params: vec![],
        type_expr: reference("morphir/SDK", "decimal", "decimal", vec![]),
    });
    assert!(!result.success);
    assert!(result.artifacts.is_empty());
    assert!(
        result.diagnostics[0]
            .message
            .contains("No Gleam mapping for SDK type")
    );
}

#[test]
fn unsupported_releases_fail_before_legacy_fallback() {
    for version in [
        serde_json::json!("4.1.0"),
        serde_json::json!("4.0.1"),
        serde_json::json!("4.0.0-preview"),
        serde_json::json!(5),
    ] {
        let result = GleamExtension.generate(GenerateRequest { ir: serde_json::json!({"formatVersion":version,"distribution":{},"name":"fallback","types":[],"values":[]}), target: "gleam".into(), options: HashMap::new() }).unwrap();
        assert!(!result.success);
        assert!(result.artifacts.is_empty());
        assert!(
            result.diagnostics[0]
                .message
                .to_lowercase()
                .contains("version")
        );
    }
}

#[test]
fn generated_functions_keep_readable_parameter_and_value_names() {
    let module = ModuleDefinition {
        types: IndexMap::new(),
        doc: None,
        values: IndexMap::from([(
            "keep-value".into(),
            AccessControlled {
                access: Access::Public,
                value: Documented::new(
                    Some("Keep the value.".into()),
                    ValueDefinition {
                        input_types: IndexMap::from([(
                            "raw-value".into(),
                            reference("morphir/SDK", "basics", "int", vec![]),
                        )]),
                        output_type: Some(reference("morphir/SDK", "basics", "int", vec![])),
                        body: ValueBody::Expression(Value::Variable(
                            ValueAttributes::default(),
                            Name::from("raw-value"),
                        )),
                    },
                ),
            },
        )]),
    };
    let package = PackageDefinition {
        modules: IndexMap::from([(
            "main".into(),
            AccessControlled {
                access: Access::Public,
                value: module,
            },
        )]),
    };
    let result = GleamExtension
        .generate(GenerateRequest {
            ir: serde_json::to_value(package).unwrap(),
            target: "gleam".into(),
            options: HashMap::new(),
        })
        .unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    let code = &result.artifacts[0].content;
    assert!(code.contains("/// Keep the value."), "{code}");
    assert!(code.contains("pub fn keep_value(raw_value)"), "{code}");
    assert!(code.contains("\n  raw_value\n"), "{code}");
    parse_gleam("main.gleam", code).expect("generated function parses");
}

#[test]
fn labels_matching_synthetic_argument_names_are_preserved() {
    let result = generate(TypeDefinition::CustomTypeDefinition {
        type_params: vec![],
        constructors: AccessControlled {
            access: Access::Public,
            value: vec![ConstructorDefinition {
                name: Name::from("customer-id"),
                args: vec![ConstructorArg {
                    name: Name::from("arg-1"),
                    arg_type: reference("morphir/SDK", "basics", "int", vec![]),
                }],
            }],
        },
    });
    assert!(result.success, "{:?}", result.diagnostics);
    assert!(
        result.artifacts[0]
            .content
            .contains("CustomerId(arg_1: Int)"),
        "{}",
        result.artifacts[0].content
    );
}

#[test]
fn classic_v3_library_generates_aliases_through_core_migration() {
    let ir = serde_json::json!({
        "formatVersion": 3,
        "distribution": ["Library", [["example"], ["package"]], [], {
            "modules": [[[["domain"], ["customer", "records"]], {"access":"Public", "value": {
                "types": [[["customer", "id"], {"access":"Public", "value":{"doc":"Customer identifier.", "value":["TypeAliasDefinition",[],["Reference",{},[[["morphir"],["s","d","k"]],[["basics"]],["int"]],[]]]}}]],
                "values": []
            }}]]
        }]
    });
    let result = GleamExtension
        .generate(GenerateRequest {
            ir,
            target: "gleam".into(),
            options: HashMap::new(),
        })
        .unwrap();
    assert!(result.success, "{:?}", result.diagnostics);
    assert_eq!(result.artifacts[0].path, "domain/customer_records.gleam");
    assert!(
        result.artifacts[0]
            .content
            .contains("pub type CustomerId = Int"),
        "{}",
        result.artifacts[0].content
    );
}

#[test]
fn public_frontend_backend_roundtrip_preserves_type_definitions() {
    let request = |documents| CompileRequest {
        language_id: "gleam".into(),
        documents,
        package: CompilePackage {
            name: "example/package".into(),
            exposed_modules: None,
        },
        dependencies: vec![],
        baseline: None,
        options: CompileOptions {
            types_only: false,
            ir_version: "4.0.0".into(),
            extra: HashMap::from([("emitParseStage".into(), serde_json::json!(false))]),
        },
    };
    let source = |path: &str, text: &str| SourceDocument {
        uri: format!("file:///workspace/src/{path}"),
        language_id: "gleam".into(),
        version: 1,
        text: text.into(),
    };
    let original = GleamExtension.compile(request(vec![
        source("domain/shared_types.gleam", "pub type SharedId = Int"),
        source("domain/customer_records.gleam", "//// Domain records.\nimport domain/shared_types\n/// Identity.\npub opaque type CustomerId { CustomerId(raw_value: shared_types.SharedId) }\npub type Pair(a) { Pair(left: a, right: a) }\npub type Handler = fn(List(Result(Int, String))) -> #(String, Nil)"),
    ])).unwrap();
    assert!(original.success, "{:?}", original.diagnostics);
    let original_ir = original.ir.unwrap();
    let generated = GleamExtension
        .generate(GenerateRequest {
            ir: original_ir.clone(),
            target: "gleam".into(),
            options: HashMap::new(),
        })
        .unwrap();
    assert!(generated.success, "{:?}", generated.diagnostics);
    let rebuilt = GleamExtension
        .compile(request(
            generated
                .artifacts
                .iter()
                .map(|artifact| source(&artifact.path, &artifact.content))
                .collect(),
        ))
        .unwrap();
    assert!(rebuilt.success, "{:?}", rebuilt.diagnostics);
    assert_eq!(
        original_ir["distribution"],
        rebuilt.ir.unwrap()["distribution"]
    );
}

#[test]
fn closed_record_aliases_become_labelled_custom_types() {
    let result = generate(TypeDefinition::TypeAliasDefinition {
        type_params: vec![Name::from("a")],
        type_expr: Type::Record(
            TypeAttributes::default(),
            vec![
                Field {
                    name: Name::from("raw-value"),
                    tpe: Type::Variable(TypeAttributes::default(), Name::from("a")),
                },
                Field {
                    name: Name::from("count"),
                    tpe: reference("morphir/SDK", "basics", "int", vec![]),
                },
            ],
        ),
    });
    assert!(result.success, "{:?}", result.diagnostics);
    let code = &result.artifacts[0].content;
    assert!(code.contains("pub type CustomerId(a) {"), "{code}");
    assert!(
        code.contains("CustomerId(raw_value: a, count: Int)"),
        "{code}"
    );
    assert!(code.contains("/// Customer identifier."), "{code}");
    let compiled = GleamExtension
        .compile(CompileRequest {
            language_id: "gleam".into(),
            documents: vec![SourceDocument {
                uri: "file:///workspace/src/domain/customer_records.gleam".into(),
                language_id: "gleam".into(),
                version: 1,
                text: code.clone(),
            }],
            package: CompilePackage {
                name: "example/package".into(),
                exposed_modules: None,
            },
            dependencies: vec![],
            baseline: None,
            options: CompileOptions {
                types_only: true,
                ir_version: "4.0.0".into(),
                extra: HashMap::from([("emitParseStage".into(), serde_json::json!(false))]),
            },
        })
        .unwrap();
    assert!(compiled.success, "{:?}", compiled.diagnostics);
    let ir: IRFile = serde_json::from_value(compiled.ir.unwrap()).unwrap();
    let Distribution::Library(library) = ir.distribution else {
        panic!("expected library")
    };
    let definition = &library.def.modules["domain/customer-records"].value.types["customer-id"]
        .value
        .value;
    let TypeDefinition::CustomTypeDefinition {
        type_params,
        constructors,
    } = definition
    else {
        panic!("record must generate a named Gleam ADT")
    };
    assert_eq!(type_params, &[Name::from("a")]);
    assert_eq!(constructors.value.len(), 1);
    assert_eq!(constructors.value[0].name, Name::from("customer-id"));
    assert_eq!(constructors.value[0].args[0].name, Name::from("raw-value"));
    assert_eq!(
        constructors.value[0].args[0].arg_type,
        Type::Variable(TypeAttributes::default(), Name::from("a"))
    );
    assert_eq!(constructors.value[0].args[1].name, Name::from("count"));
    assert_eq!(
        constructors.value[0].args[1].arg_type,
        reference("morphir/SDK", "basics", "int", vec![])
    );
}

#[test]
fn unsupported_generation_target_returns_a_diagnostic() {
    let result = GleamExtension
        .generate(GenerateRequest {
            ir: serde_json::json!({"modules":{}}),
            target: "elm".into(),
            options: HashMap::new(),
        })
        .unwrap();
    assert!(!result.success);
    assert!(result.artifacts.is_empty());
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("target")
                && diagnostic.message.contains("elm"))
    );
}

#[test]
fn sdk_collections_generate_standard_library_imports() {
    let result = generate(TypeDefinition::TypeAliasDefinition {
        type_params: vec![],
        type_expr: reference(
            "morphir/SDK",
            "dict",
            "dict",
            vec![
                reference("morphir/SDK", "string", "string", vec![]),
                reference(
                    "morphir/SDK",
                    "set",
                    "set",
                    vec![reference("morphir/SDK", "basics", "int", vec![])],
                ),
            ],
        ),
    });
    assert!(result.success, "{:?}", result.diagnostics);
    let code = &result.artifacts[0].content;
    assert!(code.contains("import gleam/dict\n"), "{code}");
    assert!(code.contains("import gleam/set\n"), "{code}");
    assert!(code.contains("dict.Dict(String, set.Set(Int))"), "{code}");
}

#[test]
fn file_roundtrip_retains_declared_types_and_value_bodies() {
    let result = morphir_gleam_binding::roundtrip::roundtrip_gleam(
        "/// Order status.\npub type OrderStatus { Ready }\npub fn answer() { 42 }",
    )
    .expect("file-based roundtrip succeeds");
    assert!(result.generated_code.contains("pub type OrderStatus"));
    assert!(result.generated_code.contains("/// Order status."));
    assert!(result.generated_code.contains("pub fn answer()"));
    assert!(result.generated_code.contains("42"));
}

/// Run with MORPHIR_TEST_GLEAM=/path/to/gleam cargo test --test backend_types -- --ignored.
#[test]
#[ignore = "requires a locally installed Gleam compiler"]
fn generated_records_aliases_and_opaque_types_pass_the_gleam_compiler() {
    let definition = |value| AccessControlled {
        access: Access::Public,
        value: Documented::new(None, value),
    };
    let module = |types| AccessControlled {
        access: Access::Public,
        value: ModuleDefinition {
            types,
            values: IndexMap::new(),
            doc: None,
        },
    };
    let package = PackageDefinition {
        modules: IndexMap::from([
            (
                "shared-types".into(),
                module(IndexMap::from([(
                    "id".into(),
                    definition(TypeDefinition::TypeAliasDefinition {
                        type_params: vec![],
                        type_expr: reference("morphir/SDK", "basics", "int", vec![]),
                    }),
                )])),
            ),
            (
                "models".into(),
                module(IndexMap::from([
                    (
                        "box".into(),
                        definition(TypeDefinition::TypeAliasDefinition {
                            type_params: vec![Name::from("a")],
                            type_expr: Type::Record(
                                TypeAttributes::default(),
                                vec![
                                    Field {
                                        name: Name::from("value"),
                                        tpe: Type::Variable(
                                            TypeAttributes::default(),
                                            Name::from("a"),
                                        ),
                                    },
                                    Field {
                                        name: Name::from("count"),
                                        tpe: reference("morphir/SDK", "basics", "int", vec![]),
                                    },
                                ],
                            ),
                        }),
                    ),
                    (
                        "token".into(),
                        definition(TypeDefinition::CustomTypeDefinition {
                            type_params: vec![],
                            constructors: AccessControlled {
                                access: Access::Private,
                                value: vec![ConstructorDefinition {
                                    name: Name::from("token"),
                                    args: vec![ConstructorArg {
                                        name: Name::from("raw-value"),
                                        arg_type: reference(
                                            "morphir/SDK",
                                            "string",
                                            "string",
                                            vec![],
                                        ),
                                    }],
                                }],
                            },
                        }),
                    ),
                    (
                        "shared-id".into(),
                        definition(TypeDefinition::TypeAliasDefinition {
                            type_params: vec![],
                            type_expr: reference("example/package", "shared-types", "id", vec![]),
                        }),
                    ),
                ])),
            ),
        ]),
    };
    let generated = GleamExtension
        .generate(GenerateRequest {
            ir: serde_json::to_value(package).unwrap(),
            target: "gleam".into(),
            options: HashMap::from([("packageName".into(), serde_json::json!("example/package"))]),
        })
        .unwrap();
    assert!(generated.success, "{:?}", generated.diagnostics);
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("gleam.toml"),
        "name = \"backend_types\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    for artifact in generated.artifacts {
        let path = project.path().join("src").join(artifact.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, artifact.content).unwrap();
    }
    let compiler = std::env::var_os("MORPHIR_TEST_GLEAM").unwrap_or_else(|| "gleam".into());
    let output = std::process::Command::new(compiler)
        .args(["check", "--target", "javascript"])
        .current_dir(project.path())
        .output()
        .expect("run installed Gleam compiler");
    assert!(
        output.status.success(),
        "Gleam rejected generated sources:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn reserved_module_segments_survive_generation_and_recompilation() {
    let compile = |documents, version: &str| {
        let result = GleamExtension
            .compile(CompileRequest {
                language_id: "gleam".into(),
                documents,
                package: CompilePackage {
                    name: "example/keywords".into(),
                    exposed_modules: None,
                },
                options: CompileOptions {
                    types_only: true,
                    ir_version: version.into(),
                    extra: [("emitParseStage".into(), false.into())].into(),
                },
                ..Default::default()
            })
            .unwrap();
        assert!(result.success, "{:?}", result.diagnostics);
        result.ir.unwrap()
    };
    let document = |path: &str, text: String| SourceDocument {
        uri: format!("file:///src/{path}"),
        language_id: "gleam".into(),
        version: 1,
        text,
    };
    for version in ["3", "4"] {
        let original = compile(vec![
            document("type_.gleam", "pub type Tree(a) { Leaf(a) Branch(Tree(a)) }".into()),
            document("domain/type_.gleam", "pub type Id = Int".into()),
            document("fn_/case_.gleam", "import type_\nimport domain/type_ as other\npub type Model = #(type_.Tree(Int), other.Id)".into()),
        ], version);
        let generated = GleamExtension
            .generate(GenerateRequest {
                ir: original.clone(),
                target: "gleam".into(),
                options: Default::default(),
            })
            .unwrap();
        assert!(generated.success, "{:?}", generated.diagnostics);
        let model = generated
            .artifacts
            .iter()
            .find(|a| a.path == "fn_/case_.gleam")
            .expect("escaped artifact path");
        assert_eq!(
            model.content,
            "// Generated by Morphir Gleam Backend\n\nimport domain/type_ as type__2\nimport type_\n\npub type Model = #(type_.Tree(Int), type__2.Id)\n\n"
        );
        let regenerated = compile(
            generated
                .artifacts
                .into_iter()
                .map(|a| {
                    parse_gleam(&a.path, &a.content).expect("valid Gleam imports");
                    document(&a.path, a.content)
                })
                .collect(),
            version,
        );
        assert_eq!(original, regenerated, "{version}");
    }
}
