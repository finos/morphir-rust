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

#[test]
fn draft_one_rejects_every_node_local_metadata_carrier() {
    let specs = "morphir::{ionVersion:\"0.1.0-draft.1\",formatVersion:\"4.0.0\",kind:specs,packageName:\"example\"}";
    let library = "morphir::{ionVersion:\"0.1.0-draft.1\",formatVersion:\"4.0.0\",kind:library,packageName:\"example\"}";
    let scope = format!("'@context':{{deprecated:\"{PREDICATE}\"}},facts:{{deprecated:true}}");
    let cases = [
        (
            "type attributes",
            specs,
            "module::spec::{name:\"api\",values:[public::spec::value::{name:\"lookup\",output:unit::{SCOPE}}]}",
        ),
        (
            "value attributes",
            library,
            "public::def::module::{name:\"api\",values:[public::def::value::{name:\"lookup\",outputType:\"morphir/SDK:basics#int\",body:(variable {SCOPE} x)}]}",
        ),
        (
            "annotation facts",
            specs,
            "module::spec::{name:\"api\",annotations:{SCOPE}}",
        ),
    ];
    for (carrier, header, body) in cases {
        let body = body.replace("SCOPE", &scope);
        let draft_two_header = header
            .replace("0.1.0-draft.1", "0.1.0-draft.2")
            .replace("4.0.0", "4.1.0");
        let accepted = format!("{draft_two_header}\n{body}\nmorphir_footer::{{}}");
        IonCodec::new()
            .decode(
                &mut Cursor::new(accepted),
                &options(true),
                &mut Events::default(),
            )
            .unwrap_or_else(|error| panic!("invalid {carrier} fixture: {error:?}"));
        let text = format!("{header}\n{body}\nmorphir_footer::{{}}");
        for linked in [false, true] {
            let result = IonCodec::new().decode(
                &mut Cursor::new(&text),
                &options(linked),
                &mut Events::default(),
            );
            assert!(
                result.is_err(),
                "draft.1 accepted {carrier} with linked option {linked}"
            );
        }
    }
}

#[test]
fn draft_two_rejects_unknown_annotation_entry_fields() {
    let text = r#"morphir::{ionVersion:"0.1.0-draft.2",formatVersion:"4.1.0",kind:specs,packageName:"example"}
module::spec::{name:"api",annotations:{entries:[{name:"example:api#tag",garbage:42}]}}
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
fn draft_two_inherited_annotation_alias_roundtrips_from_array_and_envelope() {
    for spelling in ["[\"publicApi\"]", "{entries:[\"publicApi\"]}"] {
        let text = format!(
            r#"morphir::{{ionVersion:"0.1.0-draft.2",formatVersion:"4.1.0",kind:specs,packageName:"example"}}
morphir::$meta::{{'@context':{{publicApi:"{PREDICATE}"}}}}
module::spec::{{name:"api",annotations:{spelling}}}
morphir_footer::{{}}"#
        );
        let mut events = Events::default();
        IonCodec::new()
            .decode(&mut Cursor::new(text), &options(true), &mut events)
            .unwrap();
        let mut encoded = Vec::new();
        IonCodec::new()
            .encode(
                &mut Replay(events.0.clone().into()),
                &mut encoded,
                &options(true),
            )
            .unwrap();
        let mut reread = Events::default();
        IonCodec::new()
            .decode(&mut Cursor::new(encoded), &options(true), &mut reread)
            .unwrap();
        assert_eq!(reread.0, events.0, "{spelling}");
    }
}
