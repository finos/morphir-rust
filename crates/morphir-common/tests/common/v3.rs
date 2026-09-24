//! What a v3 tree test compares: the single v3 JSON document an event stream encodes to.

use morphir_common::ir_transport::{CodecOptions, FormatId, IrCodec, IrVersion, JsonCodec, Layout};
use morphir_core::traversal::SemanticEvent;

/// A v3 Specs distribution: a dependency and one own module specification.
pub const SPECS: &str = r#"{"formatVersion":"3.1.0","distribution":["Specs",[["my"],["pkg"]],[[[["morphir"],["s","d","k"]],{"modules":[]}]],{"modules":[[[["basics"]],{"types":[[["int"],{"doc":"","value":["OpaqueTypeSpecification",[]]}]],"values":[],"doc":"Basics."}]]}]}"#;

/// The v3 JSON of `events`, with each module's types and values sorted by name.
///
/// A tree orders the members of a module by path, as it orders modules, so a stream read back
/// from a tree is compared with the one written after both are put in that order.
pub fn sorted_v3(events: Vec<SemanticEvent>) -> serde_json::Value {
    let mut json = Vec::new();
    {
        let mut sink = JsonCodec::new()
            .encoder(
                &mut json,
                &CodecOptions::new(IrVersion::V3, Layout::SingleFile, FormatId::json()),
            )
            .unwrap();
        for event in events {
            sink.accept(event).unwrap();
        }
        sink.finish().unwrap();
    }
    let mut value: serde_json::Value = serde_json::from_slice(&json).unwrap();
    let modules = value["distribution"][3]["modules"].as_array_mut().unwrap();
    for module in modules {
        // A Library module is access controlled; a Specs module is the specification itself.
        let definition = match module[1].get("value") {
            Some(_) => &mut module[1]["value"],
            None => &mut module[1],
        };
        for members in ["types", "values"] {
            if let Some(list) = definition[members].as_array_mut() {
                list.sort_by_key(|entry| entry[0].to_string());
            }
        }
    }
    if let Some(dependencies) = value["distribution"][2].as_array_mut() {
        for dependency in dependencies {
            for module in dependency[1]["modules"].as_array_mut().unwrap() {
                for members in ["types", "values"] {
                    if let Some(list) = module[1][members].as_array_mut() {
                        list.sort_by_key(|entry| entry[0].to_string());
                    }
                }
            }
        }
    }
    value
}
