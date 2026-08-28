use std::io::Cursor;

use morphir_common::ir_transport::{
    CodecOptions, DocumentTreeSink, DocumentTreeSource, EventSink, FormatId, IrCodec, IrVersion,
    JsonCodec, Layout, YamlCodec,
};
use morphir_common::vfs::memory_root;
use morphir_core::ir::v4;

const V4_JSON: &[u8] =
    include_bytes!("../../morphir-core/tests/fixtures/ir/v4/complete-example.json");

fn options(format: FormatId, layout: Layout) -> CodecOptions {
    CodecOptions::new(IrVersion::V4, layout, format)
}

#[test]
fn json_file_to_yaml_tree_to_json_file_is_semantically_lossless() {
    let root = memory_root();
    let json_options = options(FormatId::json(), Layout::SingleFile);
    let yaml_tree_options = options(FormatId::yaml(), Layout::DocumentTree);
    let mut input = Cursor::new(V4_JSON);
    let mut tree_sink = DocumentTreeSink::new(root.clone(), yaml_tree_options.clone()).unwrap();

    JsonCodec::new()
        .decode(&mut input, &json_options, &mut tree_sink)
        .unwrap();

    let mut tree_source = DocumentTreeSource::open(root, yaml_tree_options).unwrap();
    let mut output = Vec::new();
    JsonCodec::new()
        .encode(&mut tree_source, &mut output, &json_options)
        .unwrap();
    let expected: v4::IRFile = serde_json::from_slice(V4_JSON).unwrap();
    let actual: v4::IRFile = serde_json::from_slice(&output).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn yaml_file_to_json_tree_to_yaml_file_is_semantically_lossless() {
    let root = memory_root();
    let yaml_options = options(FormatId::yaml(), Layout::SingleFile);
    let json_tree_options = options(FormatId::json(), Layout::DocumentTree);
    let mut input = Cursor::new(include_bytes!("fixtures/yaml/v4-explicit.yaml"));
    let mut tree_sink = DocumentTreeSink::new(root.clone(), json_tree_options.clone()).unwrap();

    YamlCodec::new()
        .decode(&mut input, &yaml_options, &mut tree_sink)
        .unwrap();

    let mut tree_source = DocumentTreeSource::open(root, json_tree_options).unwrap();
    let mut output = Vec::new();
    YamlCodec::new()
        .encode(&mut tree_source, &mut output, &yaml_options)
        .unwrap();
    let mut expected_reader = Cursor::new(include_bytes!("fixtures/yaml/v4-explicit.yaml"));
    let mut actual_reader = Cursor::new(output);
    let expected = collect_json(&YamlCodec::new(), &mut expected_reader, &yaml_options);
    let actual = collect_json(&YamlCodec::new(), &mut actual_reader, &yaml_options);
    assert_eq!(actual, expected);
}

fn collect_json(
    codec: &dyn IrCodec,
    reader: &mut dyn std::io::Read,
    input_options: &CodecOptions,
) -> v4::IRFile {
    let mut output = Vec::new();
    let json_options = options(FormatId::json(), Layout::SingleFile);
    let mut encoder = JsonCodec::new()
        .encoder(&mut output, &json_options)
        .unwrap();
    codec
        .decode(reader, input_options, encoder.as_mut())
        .unwrap();
    drop(encoder);
    serde_json::from_slice(&output).unwrap()
}

#[test]
fn json_tree_to_yaml_tree_to_json_tree_is_semantically_lossless() {
    let json_root = memory_root();
    let yaml_root = memory_root();
    let result_root = memory_root();
    let json_options = options(FormatId::json(), Layout::SingleFile);
    let json_tree_options = options(FormatId::json(), Layout::DocumentTree);
    let yaml_tree_options = options(FormatId::yaml(), Layout::DocumentTree);
    let mut input = Cursor::new(V4_JSON);
    let mut json_sink =
        DocumentTreeSink::new(json_root.clone(), json_tree_options.clone()).unwrap();
    JsonCodec::new()
        .decode(&mut input, &json_options, &mut json_sink)
        .unwrap();

    let mut json_source = DocumentTreeSource::open(json_root, json_tree_options.clone()).unwrap();
    let mut yaml_sink =
        DocumentTreeSink::new(yaml_root.clone(), yaml_tree_options.clone()).unwrap();
    while let Some(event) =
        morphir_common::ir_transport::EventSource::next_event(&mut json_source).unwrap()
    {
        yaml_sink.accept(event).unwrap();
    }
    yaml_sink.finish().unwrap();

    let mut yaml_source = DocumentTreeSource::open(yaml_root, yaml_tree_options).unwrap();
    let mut result_sink =
        DocumentTreeSink::new(result_root.clone(), json_tree_options.clone()).unwrap();
    while let Some(event) =
        morphir_common::ir_transport::EventSource::next_event(&mut yaml_source).unwrap()
    {
        result_sink.accept(event).unwrap();
    }
    result_sink.finish().unwrap();

    let mut result_source = DocumentTreeSource::open(result_root, json_tree_options).unwrap();
    let mut output = Vec::new();
    JsonCodec::new()
        .encode(&mut result_source, &mut output, &json_options)
        .unwrap();
    let expected: v4::IRFile = serde_json::from_slice(V4_JSON).unwrap();
    let actual: v4::IRFile = serde_json::from_slice(&output).unwrap();
    assert_eq!(actual, expected);
}
