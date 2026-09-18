//! Bead `morphir-ir-v4-stabilize.55`: serde_json's private number token must never reach an IR
//! artifact, and no IR crate may hand an IR value to a serializer that is not serde_json.
//!
//! `serde_json` is built here with `arbitrary_precision`, which carries every number through
//! `deserialize_any` as a one-member map keyed `$serde_json::private::Number` holding the lexeme.
//! serde_json's own serializers understand that token; nothing else does, so a foreign serializer
//! writes the token out as an object — or rounds the number through `f64` to avoid it. Both are
//! silent corruption of a `DocumentLiteral`. The YAML profile therefore serialises to a
//! `serde_json::Value` and writes that tree with the kit's own canonical writer.

use std::collections::BTreeMap;
use std::io::Cursor;

use morphir_common::ir_transport::{
    CodecOptions, EventSink, EventSource, FormatId, IrCodec, IrVersion, JsonCodec, Layout,
    TransportDiagnostic, YamlCodec,
};
use morphir_core::traversal::SemanticEvent;

const NUMBER_TOKEN: &str = "$serde_json::private::Number";

/// A v4 library whose one value is a document literal holding two numbers no `f64` keeps: an
/// integer past 2^53 and a tenth.
const DOCUMENT_LITERAL_IR: &str = r#"{
  "formatVersion": 4,
  "distribution": {
    "Library": {
      "packageName": "example",
      "dependencies": {},
      "def": {
        "modules": {
          "domain": {
            "access": "Public",
            "value": {
              "types": {},
              "values": {
                "payload": {
                  "access": "Public",
                  "ExpressionBody": {
                    "inputTypes": {},
                    "outputType": "morphir/SDK:document#document",
                    "body": {
                      "Literal": {
                        "DocumentLiteral": { "id": 9007199254740993, "ratio": 0.10 }
                      }
                    }
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}"#;

#[derive(Default)]
struct CollectingSink(Vec<SemanticEvent>);

impl EventSink for CollectingSink {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        self.0.push(event);
        Ok(())
    }
}

struct QueueSource(std::collections::VecDeque<SemanticEvent>);

impl EventSource for QueueSource {
    fn next_event(&mut self) -> Result<Option<SemanticEvent>, TransportDiagnostic> {
        Ok(self.0.pop_front())
    }
}

fn v4_options(format: FormatId) -> CodecOptions {
    CodecOptions::new(IrVersion::V4, Layout::SingleFile, format)
}

fn events() -> Vec<SemanticEvent> {
    let mut sink = CollectingSink::default();
    JsonCodec::new()
        .decode(
            &mut Cursor::new(DOCUMENT_LITERAL_IR.as_bytes()),
            &v4_options(FormatId::json()),
            &mut sink,
        )
        .expect("the document-literal fixture decodes");
    sink.0
}

fn encode(codec: &dyn IrCodec, format: FormatId) -> String {
    let mut output = Vec::new();
    codec
        .encode(
            &mut QueueSource(events().into()),
            &mut output,
            &v4_options(format),
        )
        .expect("the document-literal fixture encodes");
    String::from_utf8(output).expect("encoded output is UTF-8")
}

#[test]
fn neither_encoder_writes_the_private_number_token() {
    for (name, output) in [
        ("yaml", encode(&YamlCodec::new(), FormatId::yaml())),
        ("json", encode(&JsonCodec::new(), FormatId::json())),
    ] {
        assert!(
            !output.contains(NUMBER_TOKEN) && !output.contains("serde_json"),
            "{name}: serde_json's private number token leaked into the output:\n{output}"
        );
        assert!(
            output.contains("9007199254740993"),
            "{name}: the integer lost its lexeme:\n{output}"
        );
        assert!(
            output.contains("0.10"),
            "{name}: the tenth lost its lexeme:\n{output}"
        );
    }
}

/// Every serializer crate a workspace crate may link, and who may link it.
///
/// `serde_yaml` and `serde-saphyr` read and write YAML that is not IR (the knowledge base, the
/// OKF bundles, the Gleam binding's fixtures, `morphir.toml`'s sibling `morphir.yaml`); `toml`
/// reads configuration. The Elm binding's use of `toml` is likewise limited to parsing
/// `preludes/*.toml` (prelude configuration, never an IR value). None of them may be handed an
/// IR value, and the surest way to keep that true is for the IR crates not to depend on them at
/// all.
const ALLOWED: &[(&str, &[&str])] = &[
    (
        "serde_yaml",
        &["morphir-kb", "morphir-okf", "morphir-gleam-binding"],
    ),
    ("serde-saphyr", &["morphir-config"]),
    (
        "toml",
        &[
            "morphir-common",
            "morphir-config",
            "morphir-daemon",
            "morphir-elm-binding",
        ],
    ),
];

#[test]
fn no_new_crate_links_a_serializer_that_is_not_serde_json() {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crates directory");
    let allowed: BTreeMap<&str, &[&str]> = ALLOWED.iter().copied().collect();
    let mut offenders = Vec::new();

    for entry in std::fs::read_dir(crates).expect("the crates directory reads") {
        let manifest = entry.expect("a crates entry").path().join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest).expect("a manifest reads");
        let document: toml::Table = toml::from_str(&text).expect("a manifest parses");
        let Some(name) = document
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(toml::Value::as_str)
        else {
            continue;
        };
        for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
            let Some(dependencies) = document.get(table).and_then(toml::Value::as_table) else {
                continue;
            };
            for (dependency, allow) in &allowed {
                if dependencies.contains_key(*dependency) && !allow.contains(&name) {
                    offenders.push(format!("{name} depends on {dependency} ({table})"));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "bead .55: an IR value must never reach a serializer that is not serde_json, because \
         serde_json's `arbitrary_precision` number token means nothing to one. Add the crate to \
         ALLOWED only once it is clear no IR value can reach it:\n{}",
        offenders.join("\n")
    );
}
