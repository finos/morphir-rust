use super::*;

#[test]
fn protocol_registry_validation_resolves_request_response_and_error_names() {
    validate_protocol_registry(&serde_json::json!({
        "types": [{
            "type": "record",
            "name": "Customer",
            "namespace": "acme.customer",
            "fields": []
        }],
        "messages": {
            "exchange": {
                "request": [{"name": "customer", "type": "acme.customer.Customer"}],
                "response": "acme.customer.Customer",
                "errors": ["acme.customer.Customer"]
            }
        }
    }));
}

#[test]
fn protocol_registry_validation_handles_direct_array_map_and_wrapped_types() {
    validate_protocol_registry(&serde_json::json!({
        "types": [{
            "type": "record",
            "name": "Customer",
            "namespace": "acme.customer",
            "fields": []
        }],
        "messages": {
            "exchange": {
                "request": [{
                    "name": "customers",
                    "type": {
                        "type": "array",
                        "items": "acme.customer.Customer",
                        "morphir.collection-kind": "set"
                    }
                }],
                "response": {
                    "type": "map",
                    "values": "acme.customer.Customer"
                },
                "errors": [{
                    "type": {
                        "type": "array",
                        "items": "acme.customer.Customer"
                    },
                    "morphir.wrapper-kind": "annotated"
                }]
            }
        }
    }));
}
