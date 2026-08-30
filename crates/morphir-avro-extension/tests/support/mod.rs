#![allow(dead_code)]

pub mod mothers {
    use morphir_core::ir::{classic, v4};
    use serde_json::{Value, json};

    const STRING: &str = "morphir/SDK:string#string";
    const BOOL: &str = "morphir/SDK:basics#bool";
    const CUSTOMER: &str = "acme/customer:domain#customer";

    pub fn classic_customer_library() -> Value {
        checked_classic(json!({
            "formatVersion": 3,
            "distribution": [
                "Library",
                [["acme"], ["customer"]],
                classic_dependencies(),
                {
                    "modules": [
                        [
                            [["domain"]],
                            {
                                "access": "Public",
                                "value": {
                                    "types": [
                                        [
                                            ["customer"],
                                            {
                                                "access": "Public",
                                                "value": {
                                                    "doc": "A customer record.",
                                                    "value": [
                                                        "TypeAliasDefinition",
                                                        [],
                                                        [
                                                            "Record",
                                                            {},
                                                            [
                                                                [["name"], classic_ref("string", "string")],
                                                                [["id"], classic_ref("string", "string")]
                                                            ]
                                                        ]
                                                    ]
                                                }
                                            }
                                        ],
                                        [
                                            ["status"],
                                            {
                                                "access": "Public",
                                                "value": {
                                                    "doc": "Customer status.",
                                                    "value": [
                                                        "CustomTypeDefinition",
                                                        [],
                                                        {
                                                            "access": "Public",
                                                            "value": [
                                                                [["inactive"], []],
                                                                [["active"], []]
                                                            ]
                                                        }
                                                    ]
                                                }
                                            }
                                        ],
                                        [
                                            ["complex"],
                                            {
                                                "access": "Public",
                                                "value": {
                                                    "doc": "Shared type forms.",
                                                    "value": [
                                                        "TypeAliasDefinition",
                                                        [["a"]],
                                                        classic_complex_type()
                                                    ]
                                                }
                                            }
                                        ],
                                        [
                                            ["secret"],
                                            {
                                                "access": "Public",
                                                "value": {
                                                    "doc": "Constructors are hidden.",
                                                    "value": [
                                                        "CustomTypeDefinition",
                                                        [["a"]],
                                                        {
                                                            "access": "Private",
                                                            "value": [[ ["secret"], [] ]]
                                                        }
                                                    ]
                                                }
                                            }
                                        ],
                                        [
                                            ["private", "type"],
                                            {
                                                "access": "Private",
                                                "value": {
                                                    "doc": "Must not be projected.",
                                                    "value": ["TypeAliasDefinition", [], classic_ref("basics", "bool")]
                                                }
                                            }
                                        ]
                                    ],
                                    "values": [
                                        classic_value(
                                            vec!["find", "customer"],
                                            "Find a customer.",
                                            "Public",
                                            vec![(vec!["id"], classic_ref("string", "string"))],
                                            customer_ref()
                                        ),
                                        classic_value(
                                            vec!["default", "customer"],
                                            "The default customer.",
                                            "Public",
                                            vec![],
                                            customer_ref()
                                        ),
                                        classic_value(
                                            vec!["helper"],
                                            "Must not be projected.",
                                            "Private",
                                            vec![],
                                            classic_ref("basics", "bool")
                                        )
                                    ],
                                    "doc": "Customer domain."
                                }
                            }
                        ],
                        [
                            [["private", "module"]],
                            {
                                "access": "Private",
                                "value": { "types": [], "values": [], "doc": "Hidden." }
                            }
                        ]
                    ]
                }
            ]
        }))
    }

    pub fn v4_customer_library() -> Value {
        checked_v4(v4_file("Library", v4_library_content()))
    }

    pub fn v4_customer_application() -> Value {
        v4_customer_application_with_entry_points(json!({
            "customer-query": {
                "target": "acme/customer:domain#find-customer",
                "kind": "command",
                "doc": "Application command."
            },
            "unfinished": {
                "target": "acme/customer:domain#unfinished",
                "kind": "handler"
            }
        }))
    }

    pub fn v4_customer_application_with_entry_points(entry_points: Value) -> Value {
        let mut content = v4_library_content();
        content["entryPoints"] = entry_points;
        content["def"]["modules"]["domain"]["value"]["values"]["unfinished"] = json!({
            "access": "Public",
            "value": documented(
                "An incomplete handler.",
                json!({
                    "inputTypes": {},
                    "body": {
                        "IncompleteBody": {
                            "incompleteness": { "Draft": {} }
                        }
                    }
                })
            )
        });
        checked_v4(v4_file("Application", content))
    }

    pub fn v4_customer_specs() -> Value {
        checked_v4(v4_file(
            "Specs",
            json!({
                "packageName": "acme/customer",
                "dependencies": v4_dependencies(),
                "spec": {
                    "modules": {
                        "domain": {
                            "types": {
                                "customer": documented(
                                    "A customer record.",
                                    json!({
                                        "TypeAliasSpecification": {
                                            "typeParams": [],
                                            "typeExp": { "Record": { "name": STRING, "id": STRING } }
                                        }
                                    })
                                ),
                                "token": documented(
                                    "An opaque token.",
                                    json!({ "OpaqueTypeSpecification": { "typeParams": ["a"] } })
                                )
                            },
                            "values": {
                                "curried": documented(
                                    "A curried signature.",
                                    json!({
                                        "inputs": {},
                                        "output": {
                                            "Function": {
                                                "argumentType": STRING,
                                                "returnType": {
                                                    "Function": {
                                                        "argumentType": STRING,
                                                        "returnType": BOOL
                                                    }
                                                }
                                            }
                                        }
                                    })
                                ),
                                "curried-with-explicit": documented(
                                    "A colliding explicit name.",
                                    json!({
                                        "inputs": { "arg2": STRING },
                                        "output": {
                                            "Function": {
                                                "argumentType": STRING,
                                                "returnType": BOOL
                                            }
                                        }
                                    })
                                ),
                                "default-customer": documented(
                                    "The default customer.",
                                    json!({ "inputs": {}, "output": CUSTOMER })
                                )
                            },
                            "doc": ["Customer", "specifications."]
                        }
                    }
                }
            }),
        ))
    }

    pub fn v4_incomplete_library() -> Value {
        checked_v4(v4_file(
            "Library",
            json!({
                "packageName": "acme/customer",
                "dependencies": v4_dependencies(),
                "def": {
                    "modules": {
                        "domain": {
                            "access": "Public",
                            "value": {
                                "types": {
                                    "draft-customer": {
                                        "access": "Public",
                                        "value": documented(
                                            "Work in progress.",
                                            json!({
                                                "IncompleteTypeDefinition": {
                                                    "typeParams": ["a"],
                                                    "incompleteness": {
                                                        "Hole": {
                                                            "UnresolvedReference": {
                                                                "target": "acme/customer:domain#missing"
                                                            }
                                                        }
                                                    },
                                                    "partialTypeExp": {
                                                        "Record": { "id": STRING }
                                                    }
                                                }
                                            })
                                        )
                                    }
                                },
                                "values": {}
                            }
                        }
                    }
                }
            }),
        ))
    }

    fn v4_file(kind: &str, content: Value) -> Value {
        json!({
            "formatVersion": "4.0.0",
            "distribution": { kind: content }
        })
    }

    fn v4_library_content() -> Value {
        json!({
            "packageName": "acme/customer",
            "dependencies": v4_dependencies(),
            "def": {
                "modules": {
                    "private-module": {
                        "access": "Private",
                        "value": { "types": {}, "values": {} }
                    },
                    "domain": {
                        "access": "Public",
                        "value": {
                            "types": {
                                "private-type": {
                                    "access": "Private",
                                    "value": documented(
                                        "Must not be projected.",
                                        json!({
                                            "TypeAliasDefinition": {
                                                "typeParams": [],
                                                "typeExp": BOOL
                                            }
                                        })
                                    )
                                },
                                "status": {
                                    "access": "Public",
                                    "value": documented(
                                        "Customer status.",
                                        json!({
                                            "CustomTypeDefinition": {
                                                "typeParams": [],
                                                "access": "Public",
                                                "constructors": {
                                                    "inactive": [],
                                                    "active": []
                                                }
                                            }
                                        })
                                    )
                                },
                                "complex": {
                                    "access": "Public",
                                    "value": documented(
                                        "Shared type forms.",
                                        json!({
                                            "TypeAliasDefinition": {
                                                "typeParams": ["a"],
                                                "typeExp": v4_complex_type()
                                            }
                                        })
                                    )
                                },
                                "secret": {
                                    "access": "Public",
                                    "value": documented(
                                        "Constructors are hidden.",
                                        json!({
                                            "CustomTypeDefinition": {
                                                "typeParams": ["a"],
                                                "access": "Private",
                                                "constructors": { "secret": [] }
                                            }
                                        })
                                    )
                                },
                                "customer": {
                                    "access": "Public",
                                    "value": documented(
                                        "A customer record.",
                                        json!({
                                            "TypeAliasDefinition": {
                                                "typeParams": [],
                                                "typeExp": {
                                                    "Record": { "name": STRING, "id": STRING }
                                                }
                                            }
                                        })
                                    )
                                }
                            },
                            "values": {
                                "helper": v4_value(
                                    "Must not be projected.",
                                    "Private",
                                    json!({}),
                                    json!(BOOL)
                                ),
                                "default-customer": v4_value(
                                    "The default customer.",
                                    "Public",
                                    json!({}),
                                    json!(CUSTOMER)
                                ),
                                "find-customer": v4_value(
                                    "Find a customer.",
                                    "Public",
                                    json!({ "id": { "type": STRING } }),
                                    json!(CUSTOMER)
                                )
                            },
                            "doc": "Customer domain."
                        }
                    }
                }
            }
        })
    }

    fn classic_value(
        name: Vec<&str>,
        doc: &str,
        access: &str,
        inputs: Vec<(Vec<&str>, Value)>,
        output: Value,
    ) -> Value {
        let inputs = inputs
            .into_iter()
            .map(|(name, tpe)| json!([name, tpe.clone(), tpe]))
            .collect::<Vec<_>>();
        json!([
            name,
            {
                "access": access,
                "value": {
                    "doc": doc,
                    "value": {
                        "inputTypes": inputs,
                        "outputType": output.clone(),
                        "body": ["Unit", output]
                    }
                }
            }
        ])
    }

    fn classic_dependencies() -> Value {
        json!([
            [
                [["shared"], ["z"]],
                classic_dependency_spec("z", "Z dependency.")
            ],
            [
                [["shared"], ["a"]],
                classic_dependency_spec("a", "A dependency.")
            ]
        ])
    }

    fn classic_dependency_spec(local: &str, doc: &str) -> Value {
        json!({
            "modules": [
                [
                    [["api"]],
                    {
                        "types": [
                            [
                                [local, "id"],
                                {
                                    "doc": doc,
                                    "value": [
                                        "TypeAliasSpecification",
                                        [],
                                        classic_ref("string", "string")
                                    ]
                                }
                            ]
                        ],
                        "values": [
                            [
                                ["lookup"],
                                {
                                    "doc": "Lookup by identifier.",
                                    "value": {
                                        "inputs": [
                                            [["id"], classic_ref("string", "string")]
                                        ],
                                        "output": classic_ref("basics", "bool")
                                    }
                                }
                            ]
                        ],
                        "doc": "Dependency API."
                    }
                ]
            ]
        })
    }

    fn v4_dependencies() -> Value {
        json!({
            "shared/z": v4_dependency_spec("z-id", "Z dependency."),
            "shared/a": v4_dependency_spec("a-id", "A dependency.")
        })
    }

    fn v4_dependency_spec(local: &str, doc: &str) -> Value {
        json!({
            "modules": {
                "api": {
                    "types": {
                        local: documented(
                            doc,
                            json!({
                                "TypeAliasSpecification": {
                                    "typeParams": [],
                                    "typeExp": STRING
                                }
                            })
                        )
                    },
                    "values": {
                        "lookup": documented(
                            "Lookup by identifier.",
                            json!({
                                "inputs": { "id": STRING },
                                "output": BOOL
                            })
                        )
                    },
                    "doc": "Dependency API."
                }
            }
        })
    }

    fn v4_value(doc: &str, access: &str, inputs: Value, output: Value) -> Value {
        json!({
            "access": access,
            "value": documented(
                doc,
                json!({
                    "inputTypes": inputs,
                    "outputType": output,
                    "body": { "ExpressionBody": { "body": { "Unit": {} } } }
                })
            )
        })
    }

    fn documented(doc: &str, value: Value) -> Value {
        json!({ "doc": doc, "value": value })
    }

    fn classic_ref(module: &str, local: &str) -> Value {
        json!([
            "Reference",
            {},
            [[["morphir"], ["s", "d", "k"]], [[module]], [local]],
            []
        ])
    }

    fn classic_complex_type() -> Value {
        json!([
            "Tuple",
            {},
            [
                ["Variable", {}, ["a"]],
                [
                    "ExtensibleRecord",
                    {},
                    ["row"],
                    [[
                        ["payload"],
                        [
                            "Reference",
                            {},
                            [[["morphir"], ["s", "d", "k"]], [["list"]], ["list"]],
                            [["Tuple", {}, [["Unit", {}], ["Variable", {}, ["a"]]]]]
                        ]
                    ]]
                ],
                ["Unit", {}]
            ]
        ])
    }

    fn v4_complex_type() -> Value {
        json!({
            "Tuple": {
                "elements": [
                    { "Variable": { "name": "a" } },
                    {
                        "ExtensibleRecord": {
                            "variable": "row",
                            "fields": {
                                "payload": {
                                    "Reference": {
                                        "fqname": "morphir/SDK:list#list",
                                        "args": [
                                            {
                                                "Tuple": {
                                                    "elements": [
                                                        { "Unit": {} },
                                                        { "Variable": { "name": "a" } }
                                                    ]
                                                }
                                            }
                                        ]
                                    }
                                }
                            }
                        }
                    },
                    { "Unit": {} }
                ]
            }
        })
    }

    fn customer_ref() -> Value {
        json!([
            "Reference",
            {},
            [[["acme"], ["customer"]], [["domain"]], ["customer"]],
            []
        ])
    }

    fn checked_classic(value: Value) -> Value {
        let typed: classic::Distribution = serde_json::from_value(value)
            .expect("the test mother must use the canonical Classic IR shape");
        serde_json::to_value(typed).expect("Classic IR should serialize")
    }

    fn checked_v4(value: Value) -> Value {
        let typed: v4::IRFile = serde_json::from_value(value)
            .expect("the test mother must use a supported Morphir IR v4 shape");
        serde_json::to_value(typed).expect("Morphir IR v4 should serialize")
    }
}

pub mod projection {
    use morphir_avro_extension::{
        DistributionKind, EntryPointMetadata, NamedType, ProjectionModule, ProjectionPackage,
        TypeDeclaration, TypeExpr, ValueKind, ValueSpecification,
    };

    pub fn reference(source_name: &str, arguments: Vec<TypeExpr>) -> TypeExpr {
        TypeExpr::Reference {
            source_name: source_name.to_owned(),
            arguments,
        }
    }

    pub fn field(name: &str, tpe: TypeExpr) -> NamedType {
        NamedType {
            name: name.to_owned(),
            tpe,
        }
    }

    pub fn value_specification(
        source_name: &str,
        name: &str,
        inputs: Vec<NamedType>,
        output: Option<TypeExpr>,
        value_kind: ValueKind,
        entry_point: Option<EntryPointMetadata>,
    ) -> ValueSpecification {
        ValueSpecification {
            source_name: source_name.to_owned(),
            name: name.to_owned(),
            inputs,
            output,
            value_kind,
            entry_point,
            doc: Some(format!("Documentation for {name}.")),
        }
    }

    pub fn package(types: Vec<TypeDeclaration>) -> ProjectionPackage {
        ProjectionPackage {
            kind: DistributionKind::Library,
            package_name: "acme/customer".to_owned(),
            dependencies: Vec::new(),
            modules: vec![ProjectionModule {
                path: vec!["customer".to_owned()],
                types,
                values: Vec::new(),
                doc: None,
            }],
        }
    }

    pub fn alias(source_name: &str, name: &str, value: TypeExpr) -> TypeDeclaration {
        TypeDeclaration::Alias {
            source_name: source_name.to_owned(),
            name: name.to_owned(),
            type_params: Vec::new(),
            value,
            doc: None,
        }
    }

    pub fn customer_record() -> TypeDeclaration {
        alias(
            "Acme:Customer:Customer",
            "customer",
            TypeExpr::Record(vec![
                field("active", reference("morphir/SDK:basics#bool", Vec::new())),
                field("age", reference("morphir/SDK:basics#int", Vec::new())),
                field("name", reference("morphir/SDK:string#string", Vec::new())),
            ]),
        )
    }
}
