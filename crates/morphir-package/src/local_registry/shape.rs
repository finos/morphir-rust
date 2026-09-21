use super::diagnostics::invalid;
use super::json::child;
use super::*;
use std::collections::BTreeMap;

pub(crate) struct Shape {
    pub subject: Subject,
    pub violations: Vec<(String, Rule)>,
    pub support: Vec<Fault>,
}
impl Shape {
    pub fn new(subject: &Subject) -> Self {
        Self {
            subject: subject.clone(),
            violations: vec![],
            support: vec![],
        }
    }
    pub fn add(&mut self, p: impl Into<String>, rule: Rule) {
        self.violations.push((p.into(), rule))
    }
    pub fn object<'a>(
        &mut self,
        v: Option<&'a JsonNode>,
        p: &str,
        required: &[&str],
        allowed: &[&str],
    ) -> Option<&'a BTreeMap<String, JsonNode>> {
        let v = v?;
        let JsonNode::Object(m) = v else {
            self.add(p, Rule::InvalidType);
            return None;
        };
        for k in required {
            if !m.contains_key(*k) {
                self.add(child(p, k), Rule::MissingField)
            }
        }
        for k in m.keys() {
            if !allowed.contains(&k.as_str()) {
                self.add(child(p, k), Rule::UnknownField)
            }
        }
        Some(m)
    }
    pub fn closed<'a>(
        &mut self,
        v: Option<&'a JsonNode>,
        p: &str,
        names: &[&str],
    ) -> Option<&'a BTreeMap<String, JsonNode>> {
        self.object(v, p, names, names)
    }
    pub fn array<'a>(
        &mut self,
        v: Option<&'a JsonNode>,
        p: &str,
        nonempty: bool,
    ) -> &'a [JsonNode] {
        match v {
            None => &[],
            Some(JsonNode::Array(a)) => {
                if nonempty && a.is_empty() {
                    self.add(p, Rule::InvalidValue)
                }
                a
            }
            _ => {
                self.add(p, Rule::InvalidType);
                &[]
            }
        }
    }
    pub fn string<'a>(&mut self, v: Option<&'a JsonNode>, p: &str) -> Option<&'a str> {
        match v {
            None => None,
            Some(JsonNode::String(s)) => Some(s),
            _ => {
                self.add(p, Rule::InvalidType);
                None
            }
        }
    }
    pub fn parsed<T, E>(
        &mut self,
        v: Option<&JsonNode>,
        p: &str,
        parse: impl FnOnce(&str) -> Result<T, E>,
        rule: Rule,
    ) -> Option<T> {
        let s = self.string(v, p)?;
        match parse(s) {
            Ok(t) => Some(t),
            Err(_) => {
                self.add(p, rule);
                None
            }
        }
    }
    pub fn literal(&mut self, v: Option<&JsonNode>, p: &str, allowed: &[&str], code: Code) -> bool {
        let Some(s) = self.string(v, p) else {
            return false;
        };
        if allowed.contains(&s) {
            return true;
        }
        self.support.push(Fault {
            phase: Phase::Support,
            code,
            witnesses: vec![Witness::Unsupported {
                subject: (&self.subject).into(),
                pointer: p.into(),
                value: s.into(),
            }],
        });
        false
    }
    pub fn positive(&mut self, v: Option<&JsonNode>, p: &str) -> Option<u64> {
        let v = v?;
        let JsonNode::Number(n) = v else {
            self.add(p, Rule::InvalidType);
            return None;
        };
        let parsed = n
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0 && *n <= 9_007_199_254_740_991);
        if !n.bytes().all(|b| b.is_ascii_digit()) || n.starts_with('0') || parsed.is_none() {
            self.add(p, Rule::InvalidValue);
            None
        } else {
            parsed
        }
    }
    pub fn finish(&self, phase: Phase) -> Result<(), Diagnostic> {
        if self.violations.is_empty() {
            Ok(())
        } else {
            Err(invalid(&self.subject, phase, self.violations.clone()))
        }
    }
    pub fn supported(&self, phase: Option<Phase>) -> Result<(), Diagnostic> {
        let mut faults = self.support.clone();
        if let Some(phase) = phase {
            for f in &mut faults {
                f.phase = phase
            }
        }
        select_failure(&faults).map_or(Ok(()), Err)
    }
}
pub(crate) fn member<'a>(v: Option<&'a JsonNode>, k: &str) -> Option<&'a JsonNode> {
    v.and_then(|v| v.get(k))
}
pub(crate) fn top(document: &JsonNode, s: &mut Shape, kind: &str, names: &[&str]) -> bool {
    let supported = document.get("kind").and_then(JsonNode::as_str) == Some(kind)
        && document.get("formatVersion").and_then(JsonNode::as_str) == Some(CONTRACT_VERSION);
    let allowed: Vec<_> = [&["formatVersion", "kind"][..], names].concat();
    let required = if supported {
        allowed.as_slice()
    } else {
        &["formatVersion", "kind"]
    };
    if s.object(Some(document), "", required, &allowed).is_none() {
        return false;
    }
    s.literal(
        document.get("formatVersion"),
        "/formatVersion",
        &[CONTRACT_VERSION],
        Code::UnsupportedProfile,
    );
    s.literal(
        document.get("kind"),
        "/kind",
        &[kind],
        Code::UnsupportedProfile,
    );
    supported
}
pub(crate) fn path(
    v: Option<&JsonNode>,
    s: &mut Shape,
    p: &str,
    prefix: &str,
    registry: Option<&JsonNode>,
) -> Option<RegistryPath> {
    let text = s.string(v, p)?;
    let mut subject = SubjectWire::from(&s.subject);
    if let Subject::Object { registry, .. } = &s.subject {
        subject = SubjectWire::Object {
            registry: registry.as_str().into(),
            path: text.into(),
        }
    }
    if let Some(registry) = registry
        .and_then(JsonNode::as_str)
        .filter(|s| LocalId::parse(s).is_ok())
    {
        subject = SubjectWire::Object {
            registry: registry.into(),
            path: text.into(),
        }
    }
    match registry_path_fault(text) {
        Some(RegistryPathFault::Resource { resource, maximum }) => s.support.push(Fault {
            phase: Phase::Shape,
            code: Code::ResourceLimit,
            witnesses: vec![Witness::Resource {
                subject,
                resource,
                scope: ProfileScope::Profile,
                maximum: maximum.to_string(),
                observed: (maximum + 1).to_string(),
            }],
        }),
        Some(RegistryPathFault::Path(rule)) => s.support.push(Fault {
            phase: Phase::Shape,
            code: Code::UnsafePath,
            witnesses: vec![Witness::Path { subject, rule }],
        }),
        None => {
            if !text.starts_with(prefix) {
                s.add(p, Rule::InvalidValue)
            }
            return RegistryPath::parse(text).ok();
        }
    }
    None
}
pub(crate) fn unique<'a>(values: impl IntoIterator<Item = (String, &'a str)>, s: &mut Shape) {
    let mut seen = std::collections::HashSet::new();
    for (pointer, key) in values {
        if !seen.insert(key) {
            s.add(pointer, Rule::DuplicateIdentity)
        }
    }
}
