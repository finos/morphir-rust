use morphir_package::local_registry::*;
use serde_json::json;

fn syntax(bytes: &[u8], domain: JsonDomain) -> Result<DecodedJson, Diagnostic> {
    decode_json_domain(bytes, domain, &Subject::Lock, Phase::Decode)
}
fn wire(error: Diagnostic) -> serde_json::Value {
    serde_json::to_value(error).unwrap()
}
#[test]
fn byte_boundary_rejects_bom_invalid_utf8_and_duplicate_decoded_keys() {
    for bytes in [b"\xef\xbb\xbf{}".as_slice(), b"\xff", b"{", b"{} {}"] {
        assert_eq!(
            wire(syntax(bytes, JsonDomain::Lock).unwrap_err())["witnesses"][0]["rule"],
            "malformed-json"
        );
    }
    let error = wire(
        syntax(
            br#"{"a/b":1,"a\u002fb":2,"~":{"x":1,"x":2}}"#,
            JsonDomain::Lock,
        )
        .unwrap_err(),
    );
    assert_eq!(
        error["witnesses"],
        json!([
            {"kind":"violation","subject":{"kind":"lock"},"pointer":"/a~1b","rule":"duplicate-key"},
            {"kind":"violation","subject":{"kind":"lock"},"pointer":"/~0/x","rule":"duplicate-key"}
        ])
    );
}
#[test]
fn depth_and_byte_limits_are_inclusive_and_precede_parsing() {
    for domain in [
        JsonDomain::Lock,
        JsonDomain::Record,
        JsonDomain::Statement,
        JsonDomain::Policy,
        JsonDomain::Dsse,
        JsonDomain::Tuf(TufRole::Root),
        JsonDomain::Tuf(TufRole::Targets),
    ] {
        let maximum = if matches!(domain, JsonDomain::Lock | JsonDomain::Tuf(TufRole::Targets)) {
            16_777_216
        } else {
            1_048_576
        };
        let mut bytes = vec![b' '; maximum];
        bytes[0] = b'0';
        assert!(syntax(&bytes, domain).is_ok());
        bytes.push(b' ');
        assert_eq!(
            wire(syntax(&bytes, domain).unwrap_err())["code"],
            "resource-limit"
        );
    }
    assert!(
        syntax(
            format!("{}0{}", "[".repeat(64), "]".repeat(64)).as_bytes(),
            JsonDomain::Dsse
        )
        .is_ok()
    );
    assert!(
        syntax(
            format!("{}{}", "[".repeat(65), "]".repeat(65)).as_bytes(),
            JsonDomain::Dsse
        )
        .is_ok()
    );
    let error = wire(
        syntax(
            format!("{}0{}", "[".repeat(65), "]".repeat(65)).as_bytes(),
            JsonDomain::Dsse,
        )
        .unwrap_err(),
    );
    assert_eq!(error["witnesses"][0]["resource"], "json-depth");
}
#[test]
fn syntax_preserves_open_members_and_number_lexemes() {
    let text = r#"{"unknown":{"a":-0,"b":9999999999999999999999999999},"decimal":1e0}"#;
    let decoded = syntax(text.as_bytes(), JsonDomain::Dsse).unwrap();
    assert_eq!(decoded.text(), text);
    assert_eq!(
        decoded
            .document()
            .get("unknown")
            .unwrap()
            .get("a")
            .unwrap()
            .number_text(),
        Some("-0")
    );
    assert_eq!(
        decoded.document().get("decimal").unwrap().number_text(),
        Some("1e0")
    );
    assert_eq!(
        wire(syntax(text.as_bytes(), JsonDomain::Tuf(TufRole::Root)).unwrap_err())["witnesses"][0]
            ["pointer"],
        "/decimal"
    );
    assert!(
        syntax(
            br#"{"$serde_json::private::Number":"data"}"#,
            JsonDomain::Dsse
        )
        .is_ok()
    );
}
#[path = "local_registry/mothers.rs"]
#[allow(dead_code)]
mod mothers;
fn decode(v: serde_json::Value) -> Result<LibraryLock, Diagnostic> {
    decode_library_lock(&serde_json::to_vec(&v).unwrap())
}
fn object_subject() -> Subject {
    Subject::Object {
        registry: LocalId::parse("finance").unwrap(),
        path: RegistryPath::parse("records/root.json").unwrap(),
    }
}
#[test]
fn typed_lock_and_canonical_records_decode_without_authentication() {
    let lock = decode(mothers::lock()).unwrap();
    assert_eq!(lock.acquisitions()[0].registry().as_str(), "finance");
    assert_eq!(
        lock.graph().root().package_path().as_str(),
        "example.com/finance/root"
    );
    let s = canonical_diagnostic(&mothers::statement());
    assert!(decode_release_statement(s.as_bytes(), &object_subject()).is_ok());
    let r = canonical_diagnostic(&mothers::record());
    assert!(decode_registry_record(format!("{r}\n").as_bytes(), &object_subject()).is_ok());
    for suffix in ["", "\n\n", "\r\n"] {
        assert_eq!(
            wire(
                decode_registry_record(format!("{r}{suffix}").as_bytes(), &object_subject())
                    .unwrap_err()
            )["witnesses"][0]["rule"],
            "noncanonical"
        );
    }
    assert_eq!(
        wire(decode_release_statement(format!("{s}\n").as_bytes(), &object_subject()).unwrap_err())
            ["witnesses"][0]["rule"],
        "noncanonical"
    );
}
#[test]
fn supported_literals_precede_other_shape_errors_and_suppress_unknown_variants() {
    for (pointer, code) in [
        ("/formatVersion", "unsupported-profile"),
        ("/kind", "unsupported-profile"),
        ("/resolution/profile", "unsupported-profile"),
        ("/resolution/policy", "unsupported-profile"),
        (
            "/resolution/requiredCapabilities/0",
            "unsupported-capability",
        ),
        ("/acquisitions/0/source/kind", "unsupported-source"),
        ("/evidence/0/kind", "unsupported-profile"),
    ] {
        let mut v = mothers::lock();
        *v.pointer_mut(pointer).unwrap() = json!("future");
        v["extra"] = json!(true);
        let error = wire(decode(v).unwrap_err());
        assert_eq!(error["code"], code, "{pointer}");
        assert_eq!(error["phase"], "support");
    }
    let error =
        wire(decode(json!({"formatVersion":"0.1.0-draft.3","kind":[],"graph":5})).unwrap_err());
    assert_eq!(
        error["witnesses"],
        json!([{"kind":"violation","subject":{"kind":"lock"},"pointer":"/kind","rule":"invalid-type"}])
    );
    let mut v = mothers::lock();
    v["graph"]["root"]["unexpected"] = json!(false);
    assert_eq!(
        wire(decode(v).unwrap_err())["witnesses"][0]["pointer"],
        "/graph/root/unexpected"
    );
}
#[test]
fn lock_resources_precede_shape_and_topology() {
    for (pointer, n, res) in [
        ("/graph/nodes", 513, "graph-nodes"),
        ("/graph/nodes/0/bindings", 513, "node-bindings"),
        ("/evidence", 2049, "evidence-entries"),
    ] {
        let mut v = mothers::lock();
        *v.pointer_mut(pointer).unwrap() = json!(vec![json!(null); n]);
        assert_eq!(
            wire(decode(v).unwrap_err())["witnesses"][0]["resource"],
            res
        );
    }
    let mut v = mothers::lock();
    v["graph"]["nodes"] = json!(vec![json!({"bindings":vec![json!(null);512]}); 65]);
    assert_eq!(
        wire(decode(v).unwrap_err())["witnesses"][0]["resource"],
        "graph-bindings"
    );
    let mut v = mothers::statement();
    v["dependencies"] = json!(vec![json!(null); 513]);
    assert_eq!(
        wire(decode_release_statement(v.to_string().as_bytes(), &object_subject()).unwrap_err())["witnesses"]
            [0]["resource"],
        "node-bindings"
    );
}
#[test]
fn lock_closure_suppresses_only_dependent_diagnostics() {
    let mut v = mothers::lock();
    let mut a = v["acquisitions"][0].clone();
    a["statement"] = json!("absent");
    v["acquisitions"].as_array_mut().unwrap().push(a);
    let error = wire(decode(v.clone()).unwrap_err());
    assert_eq!(error["witnesses"].as_array().unwrap().len(), 1);
    assert_eq!(error["witnesses"][0]["pointer"], "/acquisitions/1/release");
    v["registries"][0]["snapshot"] = json!("absent");
    assert_eq!(
        wire(decode(v).unwrap_err())["witnesses"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let mut v = mothers::lock();
    v["acquisitions"][0]["statement"] = json!("absent");
    let e = wire(decode(v).unwrap_err());
    assert_eq!(e["witnesses"].as_array().unwrap().len(), 1);
    assert_eq!(e["witnesses"][0]["rule"], "missing-reference");
    for (pointer, value, rule) in [
        (
            "/acquisitions/0/statement",
            json!("snapshot"),
            "evidence-kind-mismatch",
        ),
        (
            "/acquisitions/0/registry",
            json!("absent"),
            "missing-reference",
        ),
        ("/graph/root/version", json!("2.0.0"), "missing-root"),
    ] {
        let mut v = mothers::lock();
        *v.pointer_mut(pointer).unwrap() = value;
        let error = wire(decode(v).unwrap_err());
        assert!(
            error["witnesses"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w["rule"] == rule),
            "{error}"
        );
    }
}
#[test]
fn unsafe_path_selection_uses_logical_order_and_portable_limits() {
    for (path, code, rule) in [
        ("/tmp/unsafe".into(), "unsafe-path", "grammar"),
        ("bundles/con".into(), "unsafe-path", "reserved-name"),
        ("bundles/lpt0.json".into(), "unsafe-path", "reserved-name"),
        (
            format!("bundles/{}", "a".repeat(129)),
            "resource-limit",
            "component-bytes",
        ),
        (
            format!("bundles/{}leaf", "a/".repeat(32)),
            "resource-limit",
            "path-components",
        ),
        (
            format!("bundles/{}/{}", "a".repeat(120), "b".repeat(120)),
            "resource-limit",
            "path-bytes",
        ),
    ] {
        let mut v = mothers::lock();
        v["acquisitions"][0]["source"]["path"] = json!(path);
        let error = wire(decode(v).unwrap_err());
        assert_eq!(error["code"], code);
        assert_eq!(
            error["witnesses"][0][if code == "unsafe-path" {
                "rule"
            } else {
                "resource"
            }],
            rule
        );
    }
    let mut v = mothers::lock();
    v["acquisitions"][0]["source"]["path"] = json!("bundles/z/con");
    let mut child = v["acquisitions"][0].clone();
    child["release"]["packagePath"] = json!("example.com/finance/child");
    child["source"]["path"] = json!("bundles/a/con");
    v["acquisitions"].as_array_mut().unwrap().insert(0, child);
    assert_eq!(
        wire(decode(v.clone()).unwrap_err())["witnesses"][0]["subject"]["path"],
        "bundles/z/con"
    );
    v["evidence"][0]["path"] = json!("metadata/con");
    v["evidence"][1]["path"] = json!("metadata/aux");
    assert_eq!(
        wire(decode(v).unwrap_err())["witnesses"][0]["subject"]["path"],
        "metadata/aux"
    );
}
#[test]
fn canonicality_precedes_dependency_identity_and_intervals() {
    let mut v = mothers::statement();
    v["dependencies"] = json!([{"irPackageName":"root","packagePath":"example.com/finance/root","versionRange":{"minimumInclusive":"2.0.0","maximumExclusive":"1.0.0"}}]);
    let text = canonical_diagnostic(&v);
    assert_eq!(
        wire(
            decode_release_statement(format!("{text}\n").as_bytes(), &object_subject())
                .unwrap_err()
        )["witnesses"][0]["rule"],
        "noncanonical"
    );
    assert_eq!(
        wire(decode_release_statement(text.as_bytes(), &object_subject()).unwrap_err())["witnesses"]
            [0]["rule"],
        "invalid-interval"
    );
}

#[test]
fn diagnostics_order_phases_codes_and_canonical_deduplicated_witnesses() {
    let witness = |p: &str| Witness::Violation {
        subject: SubjectWire::Lock,
        pointer: p.into(),
        rule: Rule::UnknownField,
    };
    let faults = [
        Fault {
            phase: Phase::Shape,
            code: Code::InvalidInput,
            witnesses: vec![witness("/z")],
        },
        Fault {
            phase: Phase::Support,
            code: Code::UnsupportedSource,
            witnesses: vec![],
        },
        Fault {
            phase: Phase::Support,
            code: Code::UnsupportedProfile,
            witnesses: vec![],
        },
    ];
    assert_eq!(
        select_failure(&faults).unwrap().code,
        Code::UnsupportedProfile
    );
    let d = select_failure(&[Fault {
        phase: Phase::Shape,
        code: Code::InvalidInput,
        witnesses: vec![witness("/z"), witness("/a"), witness("/z")],
    }])
    .unwrap();
    assert_eq!(d.witnesses, vec![witness("/a"), witness("/z")]);
}
#[test]
fn graph_reports_only_cycle_edges_and_independent_dangling_unreachable_nodes() {
    let mut v = mothers::lock();
    let releases: Vec<_> = (0..8)
        .map(|i| json!({"packagePath":format!("example.com/node-{i}"),"version":"1.0.0"}))
        .collect();
    let edges = [
        vec![1],
        vec![2],
        vec![1, 3],
        vec![3, 4],
        vec![5],
        vec![4],
        vec![7],
    ];
    let node = v["graph"]["nodes"][0].clone();
    let acquisition = v["acquisitions"][0].clone();
    let d = mothers::digest();
    v["graph"]["root"] = releases[0].clone();
    v["graph"]["nodes"] = json!(
        edges
            .iter()
            .enumerate()
            .map(|(i, targets)| {
                let mut n = node.clone();
                n["release"] = releases[i].clone();
                n["irPackageName"] = json!(format!("node-{i}"));
                n["bindings"] = json!(
                    targets
                        .iter()
                        .map(|j| json!({"irPackageName":format!("node-{j}"),"target":releases[*j]}))
                        .collect::<Vec<_>>()
                );
                n
            })
            .collect::<Vec<_>>()
    );
    v["acquisitions"] = json!(
        (0..7)
            .map(|i| {
                let mut a = acquisition.clone();
                a["release"] = releases[i].clone();
                a["record"] = json!({"path":format!("records/node-{i}.json"),"digest":d});
                a["statement"] = json!(format!("statement-{i}"));
                a
            })
            .collect::<Vec<_>>()
    );
    v["evidence"].as_array_mut().unwrap().pop();
    v["evidence"].as_array_mut().unwrap().extend((0..7).map(|i|json!({"id":format!("statement-{i}"),"registry":"finance","kind":"release-statement","path":format!("statements/node-{i}.json"),"digest":d})));
    let error = wire(decode(v).unwrap_err());
    let pairs: Vec<_> = error["witnesses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (w["pointer"].as_str().unwrap(), w["rule"].as_str().unwrap()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("/graph/nodes/1/bindings/0/target", "cycle"),
            ("/graph/nodes/2/bindings/0/target", "cycle"),
            ("/graph/nodes/3/bindings/0/target", "cycle"),
            ("/graph/nodes/4/bindings/0/target", "cycle"),
            ("/graph/nodes/5/bindings/0/target", "cycle"),
            ("/graph/nodes/6/bindings/0/target", "dangling-binding"),
            ("/graph/nodes/6/release", "unreachable-node")
        ]
    );
}
#[test]
fn nonadjacent_acquisition_and_capability_duplicates_keep_original_pointers() {
    let mut v = mothers::lock();
    let root = v["acquisitions"][0].clone();
    let mut extra = root.clone();
    extra["release"]["version"] = json!("2.0.0");
    extra["record"]["path"] = json!("records/extra.json");
    let mut duplicate = root;
    duplicate["statement"] = json!("absent");
    let mut duplicate_extra = extra.clone();
    duplicate_extra["statement"] = json!("absent");
    v["acquisitions"]
        .as_array_mut()
        .unwrap()
        .extend([extra, duplicate, duplicate_extra]);
    let error = wire(decode(v).unwrap_err());
    let pointers: Vec<_> = error["witnesses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["pointer"].as_str().unwrap())
        .collect();
    assert_eq!(
        pointers,
        vec![
            "/acquisitions/1/release",
            "/acquisitions/2/release",
            "/acquisitions/3/release"
        ]
    );
    let mut v = mothers::lock();
    v["resolution"]["requiredCapabilities"]
        .as_array_mut()
        .unwrap()
        .extend([
            json!("dsse-ed25519"),
            json!("tuf-1.0.36"),
            json!("dsse-ed25519"),
        ]);
    let error = wire(decode(v).unwrap_err());
    let pointers: Vec<_> = error["witnesses"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["pointer"].as_str().unwrap())
        .collect();
    assert_eq!(
        pointers,
        vec![
            "/resolution/requiredCapabilities/3",
            "/resolution/requiredCapabilities/4",
            "/resolution/requiredCapabilities/5"
        ]
    );
}
#[test]
fn all_lock_closure_reference_kinds_are_validated() {
    for kind in [
        "orphan-acquisition",
        "orphan-statement",
        "duplicate-evidence",
        "extra-snapshot",
        "orphan-registry",
        "missing-registry",
        "wrong-registry",
        "missing-role",
    ] {
        let mut v = mothers::lock();
        match kind {
            "orphan-acquisition" => {
                let mut a = v["acquisitions"][0].clone();
                a["release"]["version"] = json!("2.0.0");
                v["acquisitions"].as_array_mut().unwrap().push(a)
            }
            "orphan-statement" => {
                let mut e = v["evidence"][4].clone();
                e["id"] = json!("extra");
                e["path"] = json!("statements/extra.json");
                v["evidence"].as_array_mut().unwrap().push(e)
            }
            "duplicate-evidence" => {
                let e = v["evidence"][0].clone();
                v["evidence"].as_array_mut().unwrap().push(e)
            }
            "extra-snapshot" => {
                let mut e = v["evidence"][2].clone();
                e["id"] = json!("extra");
                e["path"] = json!("metadata/2.snapshot.json");
                v["evidence"].as_array_mut().unwrap().push(e)
            }
            "orphan-registry" => v["registries"]
                .as_array_mut()
                .unwrap()
                .push(json!({"id":"unused","snapshot":"snapshot"})),
            "missing-registry" => v["evidence"][0]["registry"] = json!("absent"),
            "wrong-registry" => v["evidence"][4]["registry"] = json!("other"),
            "missing-role" => {
                v["evidence"].as_array_mut().unwrap().remove(0);
            }
            _ => unreachable!(),
        }
        assert_eq!(wire(decode(v).unwrap_err())["phase"], "structure", "{kind}");
    }
}
#[test]
fn number_tokens_in_capabilities_are_invalid_types_without_derived_identity_errors() {
    let mut v = mothers::lock();
    v["resolution"]["requiredCapabilities"]
        .as_array_mut()
        .unwrap()
        .extend([json!(1), json!(1)]);
    let error = wire(decode(v).unwrap_err());
    assert_eq!(error["witnesses"].as_array().unwrap().len(), 2);
    assert!(
        error["witnesses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["rule"] == "invalid-type")
    );
}

#[test]
fn decoded_graph_is_normalized_only_after_wire_order_diagnostics() {
    let mut input = mothers::lock();
    let template = input["graph"]["nodes"][0].clone();
    let mut nodes = ["beta", "alpha"]
        .map(|name| {
            let mut node = template.clone();
            node["release"]["packagePath"] = json!(format!("example.com/finance/{name}"));
            node["irPackageName"] = json!(name);
            node
        })
        .to_vec();
    let mut root = template;
    root["bindings"] = json!(
        nodes
            .iter()
            .map(|node| json!({
                "irPackageName": node["irPackageName"], "target": node["release"]
            }))
            .collect::<Vec<_>>()
    );
    nodes.push(root);
    input["graph"]["nodes"] = json!(nodes);
    let acquisition = input["acquisitions"][0].clone();
    input["acquisitions"] = json!(
        nodes
            .iter()
            .map(|node| {
                let mut acquisition = acquisition.clone();
                acquisition["release"] = node["release"].clone();
                acquisition["record"]["path"] = json!(format!(
                    "records/{}.json",
                    node["irPackageName"].as_str().unwrap()
                ));
                acquisition
            })
            .collect::<Vec<_>>()
    );

    let decoded = decode(input.clone()).unwrap();
    let graph = decoded.graph();
    assert_eq!(graph.nodes()[0].release(), graph.root());
    assert_eq!(
        graph
            .nodes()
            .iter()
            .map(|node| node.ir_package_name().as_str())
            .collect::<Vec<_>>(),
        ["root", "alpha", "beta"]
    );
    assert_eq!(
        graph.nodes()[0]
            .bindings()
            .iter()
            .map(|binding| binding.ir_package_name().as_str())
            .collect::<Vec<_>>(),
        ["alpha", "beta"]
    );

    input["graph"]["nodes"][2]["bindings"][0]["target"]["version"] = json!("2.0.0");
    let rejected = wire(decode(input).unwrap_err());
    assert_eq!(
        rejected["witnesses"],
        json!([
            {"kind":"violation", "subject":{"kind":"lock"}, "pointer":"/graph/nodes/0/release", "rule":"unreachable-node"},
            {"kind":"violation", "subject":{"kind":"lock"}, "pointer":"/graph/nodes/2/bindings/0/target", "rule":"dangling-binding"}
        ])
    );
}
