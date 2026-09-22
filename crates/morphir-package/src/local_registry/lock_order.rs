use super::*;
use crate::resolution::StableVersion;
use std::collections::BTreeMap;

fn text(v: Option<&JsonNode>) -> &str {
    v.and_then(JsonNode::as_str).unwrap_or_default()
}
fn raw_identity(v: Option<&JsonNode>) -> (&str, &str) {
    (text(member(v, "packagePath")), text(member(v, "version")))
}
pub(super) fn order_faults(faults: &mut [Fault], doc: &JsonNode) {
    if faults.len() < 2 {
        return;
    }
    let root = raw_identity(member(doc.get("graph"), "root"));
    let mut acquisitions: Vec<_> = doc
        .get("acquisitions")
        .and_then(JsonNode::as_array)
        .unwrap_or_default()
        .iter()
        .collect();
    acquisitions.sort_by(|a, b| {
        let a = raw_identity(a.get("release"));
        let b = raw_identity(b.get("release"));
        (b == root)
            .cmp(&(a == root))
            .then_with(|| a.0.as_bytes().cmp(b.0.as_bytes()))
            .then_with(
                || match (StableVersion::parse(a.1), StableVersion::parse(b.1)) {
                    (Ok(a), Ok(b)) => b.cmp(&a),
                    _ => std::cmp::Ordering::Equal,
                },
            )
    });
    let mut order: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for entry in doc
        .get("evidence")
        .and_then(JsonNode::as_array)
        .unwrap_or_default()
    {
        if let (Some(registry), Some(path)) = (
            entry.get("registry").and_then(JsonNode::as_str),
            entry.get("path").and_then(JsonNode::as_str),
        ) {
            order.entry((registry.into(), path.into())).or_insert(vec![
                registry.into(),
                "0".into(),
                path.into(),
            ]);
        }
    }
    for (i, entry) in acquisitions.iter().enumerate() {
        if let Some(registry) = entry.get("registry").and_then(JsonNode::as_str) {
            for name in ["record", "source"] {
                if let Some(path) = member(entry.get(name), "path").and_then(JsonNode::as_str) {
                    order.entry((registry.into(), path.into())).or_insert(vec![
                        registry.into(),
                        "1".into(),
                        format!("{i:08}"),
                        path.into(),
                    ]);
                }
            }
        }
    }
    faults.sort_by_cached_key(|f| {
        let subject = match f.witnesses.first() {
            Some(
                Witness::Path { subject, .. }
                | Witness::Resource { subject, .. }
                | Witness::Violation { subject, .. }
                | Witness::Unsupported { subject, .. }
                | Witness::Authentication { subject, .. },
            ) => Some(subject),
            _ => None,
        };
        if let Some(SubjectWire::Object { registry, path }) = subject {
            order
                .get(&(registry.clone(), path.clone()))
                .cloned()
                .unwrap_or_else(|| {
                    vec![
                        registry.clone(),
                        "1".into(),
                        "000000-1".into(),
                        path.clone(),
                    ]
                })
        } else {
            vec![String::new()]
        }
    });
}
