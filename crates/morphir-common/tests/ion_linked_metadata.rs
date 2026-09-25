use std::collections::VecDeque;
use std::io::Cursor;

use morphir_common::ir_transport::{
    CodecOptions, DocumentTreeSink, DocumentTreeSource, EventSink, EventSource, FormatId, IonCodec,
    IrCodec, IrVersion, Layout, TransportDiagnostic,
};
use morphir_common::vfs::memory_root;
use morphir_core::traversal::{SemanticEvent, SemanticEventKind};

#[derive(Default)]
struct Events(Vec<SemanticEvent>);
impl EventSink for Events {
    fn accept(&mut self, event: SemanticEvent) -> Result<(), TransportDiagnostic> {
        self.0.push(event);
        Ok(())
    }
}
struct Replay(VecDeque<SemanticEvent>);
impl EventSource for Replay {
    fn next_event(&mut self) -> Result<Option<SemanticEvent>, TransportDiagnostic> {
        Ok(self.0.pop_front())
    }
}
fn options(linked: bool) -> CodecOptions {
    let options = CodecOptions::new(IrVersion::V4, Layout::SingleFile, FormatId::ion());
    if linked {
        options.with_linked_metadata()
    } else {
        options
    }
}
const PREDICATE: &str =
    "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/deprecated";
const SUBJECT: &str = "morphir://ir/pkg/example?format=4.1.0#/module/api/value/calculate-total";

fn document() -> String {
    format!(
        r#"
morphir::{{ionVersion: "0.1.0-draft.2", formatVersion: "4.1.0", kind: library, packageName: "example"}}
morphir::$meta::{{'@context': {{deprecated: "{PREDICATE}"}}, '@graph': [{{'@id': "{SUBJECT}", deprecated: true}}]}}
morphir_footer::{{}}
"#
    )
}

#[test]
fn draft_two_document_metadata_roundtrips_without_loss() {
    let mut events = Events::default();
    IonCodec::new()
        .decode(&mut Cursor::new(document()), &options(true), &mut events)
        .unwrap();
    assert!(
        events
            .0
            .iter()
            .any(|event| matches!(event.kind(), SemanticEventKind::DocumentMetadata(_)))
    );
    let mut output = Vec::new();
    IonCodec::new()
        .encode(&mut Replay(events.0.into()), &mut output, &options(true))
        .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("morphir::$meta::"), "{text}");
    assert!(text.contains(PREDICATE), "{text}");
    assert!(text.contains(SUBJECT), "{text}");
}

#[test]
fn draft_two_requires_opt_in_and_draft_one_rejects_metadata() {
    let codec = IonCodec::new();
    assert!(
        codec
            .decode(
                &mut Cursor::new(document()),
                &options(false),
                &mut Events::default()
            )
            .is_err()
    );
    let old = document().replace("0.1.0-draft.2", "0.1.0-draft.1");
    assert!(
        codec
            .decode(
                &mut Cursor::new(old),
                &options(true),
                &mut Events::default()
            )
            .is_err()
    );
    let duplicate = document().replace(
        "morphir_footer::{}",
        "morphir::$meta::{}\nmorphir_footer::{}",
    );
    assert!(
        codec
            .decode(
                &mut Cursor::new(duplicate),
                &options(true),
                &mut Events::default()
            )
            .is_err()
    );
    let old_release = document()
        .replace("0.1.0-draft.2", "0.1.0-draft.1")
        .replace("formatVersion: \"4.1.0\"", "formatVersion: \"4.0.0\"");
    assert!(
        codec
            .decode(
                &mut Cursor::new(old_release),
                &options(true),
                &mut Events::default()
            )
            .is_err()
    );
}

#[test]
fn draft_two_tree_manifest_keeps_one_document_metadata_element() {
    let mut events = Events::default();
    IonCodec::new()
        .decode(&mut Cursor::new(document()), &options(true), &mut events)
        .unwrap();
    let root = memory_root();
    let tree_options = CodecOptions::new(IrVersion::V4, Layout::DocumentTree, FormatId::ion())
        .with_linked_metadata();
    let mut sink = DocumentTreeSink::new(root.clone(), tree_options.clone()).unwrap();
    for event in events.0 {
        sink.accept(event).unwrap();
    }
    sink.finish().unwrap();
    let manifest = root.join("manifest.ion").unwrap().read_to_string().unwrap();
    assert_eq!(
        manifest.matches("morphir::$meta::").count(),
        1,
        "{manifest}"
    );
    let mut source = DocumentTreeSource::open(root, tree_options).unwrap();
    let mut count = 0;
    while let Some(event) = source.next_event().unwrap() {
        if matches!(event.kind(), SemanticEventKind::DocumentMetadata(_)) {
            count += 1;
        }
    }
    assert_eq!(count, 1);
}

#[test]
fn draft_two_single_record_reads_document_metadata_member() {
    let record = format!(
        r#"morphir::{{
        ionVersion: "0.1.0-draft.2", formatVersion: "4.1.0",
        kind: library, packageName: "example",
        '$meta': {{'@context': {{deprecated: "{PREDICATE}"}}, '@graph': [{{'@id': "{SUBJECT}", deprecated: true}}]}},
        modules: []
    }}"#
    );
    let mut events = Events::default();
    IonCodec::new()
        .decode(&mut Cursor::new(record), &options(true), &mut events)
        .unwrap();
    assert_eq!(
        events
            .0
            .iter()
            .filter(|event| matches!(event.kind(), SemanticEventKind::DocumentMetadata(_)))
            .count(),
        1
    );
}

#[test]
fn draft_two_rejects_metadata_on_a_value_spec_instead_of_its_annotations() {
    let text = r#"morphir::{ionVersion:"0.1.0-draft.2", formatVersion:"4.1.0", kind:specs, packageName:"example"}
module::spec::{name:"api", values:[public::spec::value::{name:"lookup", output:unit::{}, facts:{deprecated:true}}]}
morphir_footer::{}"#;
    assert!(
        IonCodec::new()
            .decode(
                &mut Cursor::new(text),
                &options(true),
                &mut Events::default()
            )
            .is_err()
    );
}

#[test]
fn draft_two_rejects_override_of_protected_document_alias() {
    let text = format!(
        r#"morphir::{{ionVersion:"0.1.0-draft.2", formatVersion:"4.1.0", kind:specs, packageName:"example"}}
morphir::$meta::{{'@context':{{deprecated:{{'@id':"{PREDICATE}",'@protected':true}}}}}}
module::spec::{{name:"api",values:[public::spec::value::{{name:"lookup",output:unit::{{'@context':{{deprecated:"morphir://ir/pkg/acme/metadata?format=4.0.0#/module/lifecycle/value/other"}},facts:{{deprecated:true}}}}}}]}}
morphir_footer::{{}}"#
    );
    assert!(
        IonCodec::new()
            .decode(
                &mut Cursor::new(text),
                &options(true),
                &mut Events::default()
            )
            .is_err()
    );
}
