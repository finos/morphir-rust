use morphir_core::ir::v4;
use serde_json::{Value, json};

const STRING: &str = "morphir/SDK:string#string";
const BOOL: &str = "morphir/SDK:basics#bool";
const CUSTOMER: &str = "acme/customer:domain#customer";
const RESULT: &str = "morphir/SDK:result#result";

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
    // Only the Application distribution declares this: `v4_customer_library()`
    // builds `v4_library_content()` directly, so a Result-returning public
    // value here has no effect on the v3/v4 parity check or the sorted-values
    // assertions that pin `v4_customer_library()`'s value list exactly.
    content["def"]["modules"]["domain"]["value"]["values"]["validate-customer"] = v4_value(
        "Validate a customer, returning an error message or the customer.",
        "Public",
        json!({ "id": { "type": STRING } }),
        json!({ "Reference": { "fqname": RESULT, "args": [STRING, CUSTOMER] } }),
    );
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
                            "curried-with-normalized-explicit": documented(
                                "An Avro-normalized colliding explicit name.",
                                json!({
                                    "inputs": { "arg-1": STRING },
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
fn checked_v4(value: Value) -> Value {
    let typed: v4::IRFile = serde_json::from_value(value)
        .expect("the test mother must use a supported Morphir IR v4 shape");
    serde_json::to_value(typed).expect("Morphir IR v4 should serialize")
}
