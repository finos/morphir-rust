use super::*;

mod customer;
mod dependencies;

pub(crate) use customer::*;
pub(crate) use dependencies::*;

pub(super) fn json_cases() -> Vec<GoldenCase> {
    vec![
        GoldenCase {
            golden: "customer-schemas.avsc",
            expected_path: "acme/customer/customer/Customer.avsc",
            package: package(vec![documented_customer_record()]),
            options: AvroOptions::default(),
        },
        GoldenCase {
            golden: "customer-entry-points.avpr",
            expected_path: "acme/customer/Customer.avpr",
            package: customer_package(),
            options: options(Projection::ProtocolEntryPoints),
        },
        GoldenCase {
            golden: "customer-public.avpr",
            expected_path: "acme/customer/Customer.avpr",
            package: customer_package(),
            options: options(Projection::ProtocolPublic),
        },
        GoldenCase {
            golden: "edge-alias-wrapper.avsc",
            expected_path: "acme/customer/customer/CustomerLabels.avsc",
            package: alias_wrapper_package(),
            options: AvroOptions {
                aliases: Aliases::WrapperRecord,
                ..AvroOptions::default()
            },
        },
        GoldenCase {
            golden: "edge-generic-result.avsc",
            expected_path: "acme/customer/customer/LookupResult.avsc",
            package: generic_result_package(),
            options: AvroOptions::default(),
        },
        GoldenCase {
            golden: "edge-logical-constants.avpr",
            expected_path: "acme/customer/Customer.avpr",
            package: logical_constants_package(),
            options: options(Projection::ProtocolPublic),
        },
        GoldenCase {
            golden: "edge-partial.avsc",
            expected_path: "acme/customer/customer/Supported.avsc",
            package: partial_package(),
            options: AvroOptions {
                unsupported: Unsupported::WarnAndSkip,
                ..AvroOptions::default()
            },
        },
    ]
}

pub(super) fn idl_cases() -> Vec<GoldenCase> {
    let idl = |projection| AvroOptions {
        representation: morphir_avro_extension::Representation::Idl,
        projection,
        ..AvroOptions::default()
    };
    vec![
        GoldenCase {
            golden: "customer-schemas.avdl",
            expected_path: "acme/customer/customer/CustomerSchemas.avdl",
            package: package(vec![documented_customer_record()]),
            options: idl(Projection::Schemas),
        },
        GoldenCase {
            golden: "customer-entry-points.avdl",
            expected_path: "acme/customer/Customer.avdl",
            package: customer_package(),
            options: idl(Projection::ProtocolEntryPoints),
        },
        GoldenCase {
            golden: "customer-public.avdl",
            expected_path: "acme/customer/Customer.avdl",
            package: customer_package(),
            options: idl(Projection::ProtocolPublic),
        },
        GoldenCase {
            golden: "edge-linked.avdl",
            expected_path: "acme/customer/customer/OrderSchemas.avdl",
            package: linked_package(),
            options: AvroOptions {
                representation: morphir_avro_extension::Representation::Idl,
                dependencies: morphir_avro_extension::Dependencies::Linked,
                ..AvroOptions::default()
            },
        },
        GoldenCase {
            golden: "edge-linked-chain.avdl",
            expected_path: "acme/customer/customer/ChainOrderSchemas.avdl",
            package: idl_linked_chain_package(),
            options: AvroOptions {
                representation: morphir_avro_extension::Representation::Idl,
                dependencies: morphir_avro_extension::Dependencies::Linked,
                ..AvroOptions::default()
            },
        },
        GoldenCase {
            golden: "edge-custom-types.avdl",
            expected_path: "acme/customer/Customer.avdl",
            package: idl_custom_types_package(),
            options: idl_custom_types_options(),
        },
        GoldenCase {
            golden: "edge-escaping.avdl",
            expected_path: "acme/customer/Customer.avdl",
            package: idl_escaping_package(),
            options: idl(Projection::ProtocolPublic),
        },
        GoldenCase {
            golden: "edge-primitive-protocol.avdl",
            expected_path: "acme/customer/Customer.avdl",
            package: idl_primitive_protocol_package(),
            options: idl(Projection::ProtocolPublic),
        },
    ]
}
