use morphir_core::ir::classic;
use serde_json::{Value, json};

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

/// A Classic library covering the schema-projection forms: a record alias with
/// a multi-word field, an optional field, a reference to a sibling declaration,
/// and a nullary custom type.
pub fn classic_schema_library() -> Value {
    checked_classic(json!({
        "formatVersion": 3,
        "distribution": [
            "Library",
            [["acme"], ["customer"]],
            [],
            {
                "modules": [
                    [
                        [["customer"]],
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
                                                            [
                                                                ["customer", "id"],
                                                                classic_ref("string", "string")
                                                            ],
                                                            [
                                                                ["nickname"],
                                                                classic_maybe(classic_ref(
                                                                    "string", "string"
                                                                ))
                                                            ],
                                                            [
                                                                ["status"],
                                                                classic_local_ref("status")
                                                            ]
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
                                    ]
                                ],
                                "values": [],
                                "doc": "Customer domain."
                            }
                        }
                    ]
                ]
            }
        ]
    }))
}

/// A Classic library whose two modules declare the same local type name, so any
/// backend that derives a schema name from the local name sees a collision.
pub fn classic_colliding_names_library() -> Value {
    checked_classic(json!({
        "formatVersion": 3,
        "distribution": [
            "Library",
            [["acme"], ["customer"]],
            [],
            {
                "modules": [
                    classic_alias_module("customer", "A customer identifier."),
                    classic_alias_module("billing", "A billing identifier.")
                ]
            }
        ]
    }))
}

/// A Classic library holding one projectable record alias next to a record
/// alias whose field is a function, which no data schema can represent.
pub fn classic_function_field_library() -> Value {
    checked_classic(json!({
        "formatVersion": 3,
        "distribution": [
            "Library",
            [["acme"], ["customer"]],
            [],
            {
                "modules": [
                    [
                        [["customer"]],
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
                                                        [[
                                                            ["customer", "id"],
                                                            classic_ref("string", "string")
                                                        ]]
                                                    ]
                                                ]
                                            }
                                        }
                                    ],
                                    [
                                        ["handler"],
                                        {
                                            "access": "Public",
                                            "value": {
                                                "doc": "Holds a function as data.",
                                                "value": [
                                                    "TypeAliasDefinition",
                                                    [],
                                                    [
                                                        "Record",
                                                        {},
                                                        [[
                                                            ["run"],
                                                            [
                                                                "Function",
                                                                {},
                                                                classic_ref("string", "string"),
                                                                classic_ref("basics", "bool")
                                                            ]
                                                        ]]
                                                    ]
                                                ]
                                            }
                                        }
                                    ]
                                ],
                                "values": [],
                                "doc": "Customer domain."
                            }
                        }
                    ]
                ]
            }
        ]
    }))
}

fn classic_alias_module(module: &str, doc: &str) -> Value {
    json!([
        [[module]],
        {
            "access": "Public",
            "value": {
                "types": [
                    [
                        ["customer"],
                        {
                            "access": "Public",
                            "value": {
                                "doc": doc,
                                "value": [
                                    "TypeAliasDefinition",
                                    [],
                                    classic_ref("string", "string")
                                ]
                            }
                        }
                    ]
                ],
                "values": [],
                "doc": doc
            }
        }
    ])
}

fn classic_local_ref(local: &str) -> Value {
    json!([
        "Reference",
        {},
        [[["acme"], ["customer"]], [["customer"]], [local]],
        []
    ])
}

fn classic_maybe(argument: Value) -> Value {
    json!([
        "Reference",
        {},
        [[["morphir"], ["s", "d", "k"]], [["maybe"]], ["maybe"]],
        [argument]
    ])
}
