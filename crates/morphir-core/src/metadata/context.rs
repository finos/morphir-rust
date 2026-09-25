//! Pure expansion of the bounded authored metadata context grammar.

use super::{Fact, GraphName, ObjectTerm};
use crate::node_address::{NodeRoot, NodeUri};
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

const MAX_CONTEXT_IMPORT_DEPTH: usize = 128;

/// Object interpretation requested by an authored term definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Coercion {
    /// Ordinary literal data, including URI-looking strings.
    #[default]
    None,
    /// A string containing a canonical node address.
    NodeId,
    /// One opaque structured data value whose datatype comes from the schema closure.
    Json,
}

/// A context grammar, resource, or expansion error.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContextError {
    /// The authored shape is outside the bounded context grammar.
    #[error("invalid context form")]
    InvalidForm,
    /// A JSON-LD keyword is outside the accepted subset.
    #[error("unsupported context keyword: {0}")]
    UnsupportedKeyword(String),
    /// Two imports assign different meanings to the same term.
    #[error("context term collision: {0}")]
    TermCollision(String),
    /// Two imports assign different default vocabularies.
    #[error("context vocabulary collision")]
    VocabCollision,
    /// A narrower scope changed a protected inherited definition.
    #[error("protected term redefinition: {0}")]
    ProtectedTermRedefinition(String),
    /// One resource appeared twice in a transitive import closure.
    #[error("duplicate context import: {0}")]
    DuplicateImport(String),
    /// An import recurs into an active resource.
    #[error("context import cycle: {0}")]
    ImportCycle(String),
    /// The finite context import depth budget was exceeded.
    #[error("context import depth exceeds {0}")]
    ImportDepthExceeded(usize),
    /// A local reference leaves the supplied root.
    #[error("context path escapes the supplied root")]
    PathEscape,
    /// The caller did not supply this resource.
    #[error("context resource unavailable: {0}")]
    ResourceUnavailable(String),
    /// An explicit nonlocal resource has not been trusted by the caller.
    #[error("context resource untrusted: {0}")]
    ResourceUntrusted(String),
    /// The bytes do not match a content-addressed reference.
    #[error("context digest mismatch: {0}")]
    DigestMismatch(String),
    /// HTTP and other remote schemes are not context references.
    #[error("remote context forbidden: {0}")]
    RemoteForbidden(String),
    /// A digest-addressed context cannot resolve a local child.
    #[error("relative import without a file base")]
    RelativeImportWithoutBase,
    /// A target is not a canonical semantic node address.
    #[error("invalid context target: {0}")]
    InvalidTarget(String),
    /// No binding, prefix, or default vocabulary names the key.
    #[error("unbound context key: {0}")]
    UnboundKey(String),
    /// A node-link value is not a canonical node URI string.
    #[error("invalid node-link value")]
    InvalidNodeLink,
    /// The schema closure did not supply the `@json` datatype.
    #[error("@json coercion requires a declared datatype")]
    MissingJsonDatatype,
    /// A non-JSON fact object needs a recognized expanded value or node form.
    #[error("fact object must be @value or @id; structured data requires @json")]
    InvalidFactObject,
    /// A supplied context file is malformed or has the wrong envelope.
    #[error("invalid context resource: {0}")]
    InvalidResource(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Binding {
    target: String,
    prefix: bool,
    coercion: Coercion,
    protected: bool,
}

/// Effective bindings at one metadata container scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectiveContext {
    terms: BTreeMap<String, Binding>,
    vocab: Option<String>,
}

/// A compact key expanded to a semantic URI and its object coercion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedKey {
    uri: NodeUri,
    coercion: Coercion,
}

impl ExpandedKey {
    /// The canonical semantic declaration address.
    pub fn uri(&self) -> &NodeUri {
        &self.uri
    }

    /// The authored value coercion, without schema validation.
    pub fn coercion(&self) -> Coercion {
        self.coercion
    }
}

impl EffectiveContext {
    /// A self-contained authored spelling of the effective bindings.
    /// Imported aliases are expanded to their canonical targets, so a writer
    /// can carry the same scope without access to the import workspace.
    pub fn to_inline_value(&self) -> Value {
        let mut inline = Map::new();
        if let Some(vocab) = &self.vocab {
            inline.insert("@vocab".to_owned(), Value::String(vocab.clone()));
        }
        for (name, binding) in &self.terms {
            let value =
                if !binding.prefix && !binding.protected && binding.coercion == Coercion::None {
                    Value::String(binding.target.clone())
                } else {
                    let mut definition = Map::new();
                    definition.insert("@id".to_owned(), Value::String(binding.target.clone()));
                    if binding.prefix {
                        definition.insert("@prefix".to_owned(), Value::Bool(true));
                    }
                    if binding.protected {
                        definition.insert("@protected".to_owned(), Value::Bool(true));
                    }
                    match binding.coercion {
                        Coercion::None => {}
                        Coercion::NodeId => {
                            definition.insert("@type".to_owned(), Value::String("@id".to_owned()));
                        }
                        Coercion::Json => {
                            definition
                                .insert("@type".to_owned(), Value::String("@json".to_owned()));
                        }
                    }
                    Value::Object(definition)
                };
            inline.insert(name.clone(), value);
        }
        Value::Object(inline)
    }

    /// Expand an authored key. An exact alias precedes a prefix and `@vocab`.
    pub fn expand_key(&self, key: &str) -> Result<ExpandedKey, ContextError> {
        if let Some(binding) = self.terms.get(key) {
            return Ok(ExpandedKey {
                uri: parse_uri(&binding.target)?,
                coercion: binding.coercion,
            });
        }
        if key.starts_with("morphir://ir/") {
            return Ok(ExpandedKey {
                uri: parse_uri(key)?,
                coercion: Coercion::None,
            });
        }
        if let Some((prefix, suffix)) = key.split_once(':')
            && let Some(binding) = self.terms.get(prefix).filter(|binding| binding.prefix)
        {
            return Ok(ExpandedKey {
                uri: parse_uri(&format!("{}{suffix}", binding.target))?,
                coercion: Coercion::None,
            });
        }
        if let Some(vocab) = &self.vocab {
            return Ok(ExpandedKey {
                uri: parse_uri(&format!("{vocab}{key}"))?,
                coercion: Coercion::None,
            });
        }
        Err(ContextError::UnboundKey(key.to_owned()))
    }

    /// The active default declaration stem, if one was supplied.
    pub fn vocab(&self) -> Option<&str> {
        self.vocab.as_deref()
    }
}

#[derive(Debug, Clone)]
struct LoadedResource {
    bytes: Vec<u8>,
    trusted: bool,
}

/// Explicit, already-loaded context bytes. Resolution never reads a file or fetches a URL.
#[derive(Debug, Clone)]
pub struct ContextResources {
    root: String,
    loaded: BTreeMap<String, LoadedResource>,
}

impl ContextResources {
    /// Set the local path root used for lexical confinement of relative imports.
    ///
    /// Absolute roots remain absolute; `.` and `./` spellings normalize without
    /// filesystem access. The caller must supply already-loaded bytes and must
    /// apply any filesystem symlink, byte-size, and total-resource budgets before
    /// constructing this map. Resolution itself caps import nesting at 128.
    pub fn new(root: impl Into<String>) -> Self {
        let root = root.into();
        Self {
            root: normalize_lexical_path(&root).unwrap_or(root),
            loaded: BTreeMap::new(),
        }
    }

    /// Supply bytes under a local path inside the root.
    pub fn insert_local(&mut self, path: impl Into<String>, bytes: Vec<u8>) {
        let path = path.into();
        let identity = normalize_lexical_path(&path).unwrap_or(path);
        self.loaded.insert(
            identity,
            LoadedResource {
                bytes,
                trusted: true,
            },
        );
    }

    /// Supply bytes for one digest reference and the caller's trust decision.
    pub fn insert_verified(&mut self, reference: &str, bytes: Vec<u8>, trusted: bool) {
        self.loaded
            .insert(reference.to_owned(), LoadedResource { bytes, trusted });
    }
}

/// Resolve an authored `@context` value over a parent and supplied resource closure.
///
/// The `base_file` identifies the containing local context file, if any. A
/// digest-addressed resource has no file base. Reuse the same parent for sibling
/// attribute and annotation scopes so their local bindings remain isolated.
///
/// ```
/// use morphir_core::metadata::{ContextResources, Coercion, resolve_context};
/// use serde_json::json;
/// let resources = ContextResources::new("contexts");
/// let context = resolve_context(None, &json!({
///     "@vocab": "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/value/",
///     "replacement": {"@id": "morphir://ir/pkg/acme/metadata?format=4.0.0#/module/naming/value/replacement", "@type": "@id"}
/// }), &resources, None).unwrap();
/// assert_eq!(context.expand_key("replacement").unwrap().coercion(), Coercion::NodeId);
/// ```
pub fn resolve_context(
    parent: Option<&EffectiveContext>,
    authored: &Value,
    resources: &ContextResources,
    base_file: Option<&str>,
) -> Result<EffectiveContext, ContextError> {
    let mut seen = BTreeSet::new();
    let mut active = BTreeSet::new();
    resolve_value(
        parent.cloned().unwrap_or_default(),
        authored,
        resources,
        base_file
            .map(ContextBase::Local)
            .unwrap_or(ContextBase::Workspace),
        &mut seen,
        &mut active,
    )
}

/// Replace authored context references with their effective inline bindings
/// using only resources the caller has already acquired and verified. This is
/// an in-memory validation view; callers retain the original document bytes
/// for archive identity and roundtrip.
pub fn inline_document_contexts(
    document: &Value,
    resources: &ContextResources,
    source_file: Option<&str>,
) -> Result<Value, ContextError> {
    let mut result = document.clone();
    let parent = if let Some(context) = result
        .get_mut("$meta")
        .and_then(|meta| meta.get_mut("@context"))
    {
        let effective = resolve_context(None, context, resources, source_file)?;
        *context = effective.to_inline_value();
        effective
    } else {
        EffectiveContext::default()
    };
    if let Some(graph) = result
        .get_mut("$meta")
        .and_then(|meta| meta.get_mut("@graph"))
        .and_then(Value::as_array_mut)
    {
        for node in graph {
            if let Some(context) = node.get_mut("@context") {
                let effective = resolve_context(Some(&parent), context, resources, source_file)?;
                *context = effective.to_inline_value();
            }
        }
    }
    inline_node_contexts(&mut result, &parent, resources, source_file)?;
    Ok(result)
}

fn inline_node_contexts(
    value: &mut Value,
    parent: &EffectiveContext,
    resources: &ContextResources,
    source_file: Option<&str>,
) -> Result<(), ContextError> {
    match value {
        Value::Object(object) => {
            for (name, child) in object {
                if matches!(
                    name.as_str(),
                    "$meta" | "@context" | "facts" | "@graph" | "assertionSources" | "extensions"
                ) {
                    continue;
                }
                if matches!(name.as_str(), "attributes" | "annotations")
                    && let Some(context) = child.get_mut("@context")
                {
                    let effective = resolve_context(Some(parent), context, resources, source_file)?;
                    *context = effective.to_inline_value();
                }
                inline_node_contexts(child, parent, resources, source_file)?;
            }
        }
        Value::Array(items) => {
            for child in items {
                inline_node_contexts(child, parent, resources, source_file)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ContextBase<'a> {
    Workspace,
    Local(&'a str),
    ContentAddressed,
}

/// Convert one authored object using context coercion. The caller supplies the
/// validated schema datatype for `@json` and validates the resulting data.
pub fn expand_object(
    coercion: Coercion,
    value: Value,
    json_datatype: Option<NodeUri>,
) -> Result<ObjectTerm, ContextError> {
    match coercion {
        Coercion::None => Ok(ObjectTerm::value(value)),
        Coercion::NodeId => value
            .as_str()
            .and_then(|s| NodeUri::parse(s).ok())
            .map(ObjectTerm::NodeRef)
            .ok_or(ContextError::InvalidNodeLink),
        Coercion::Json => json_datatype
            .map(|uri| ObjectTerm::typed_json(value, uri))
            .ok_or(ContextError::MissingJsonDatatype),
    }
}

/// Expand authored fact properties into default-graph terms without claiming
/// declaration or data validation. The caller supplies the `@json` datatype
/// from its verified predicate closure, then validates each resulting object.
/// A bare array repeats objects; an `@json` array is one structured object.
pub fn expand_properties<'a>(
    subject: &NodeUri,
    properties: impl IntoIterator<Item = (&'a str, &'a Value)>,
    context: &EffectiveContext,
    json_datatype: impl Fn(&NodeUri) -> Option<NodeUri>,
) -> Result<Vec<Fact>, ContextError> {
    let mut facts = Vec::new();
    for (key, authored) in properties {
        let (predicate, objects) = expand_property_objects(key, authored, context, &json_datatype)?;
        facts.extend(objects.into_iter().map(|object| {
            Fact::new(
                subject.clone(),
                predicate.uri().clone(),
                object,
                GraphName::Default,
            )
        }));
    }
    Ok(facts)
}

/// Shared object expansion for a carrier's authored property. The V4 parser
/// also uses this path when matching detailed-source selectors so expanded
/// forms cannot acquire a different identity there.
pub(crate) fn expand_property_objects(
    key: &str,
    authored: &Value,
    context: &EffectiveContext,
    json_datatype: &impl Fn(&NodeUri) -> Option<NodeUri>,
) -> Result<(ExpandedKey, Vec<ObjectTerm>), ContextError> {
    let predicate = context.expand_key(key)?;
    let datatype = || json_datatype(predicate.uri());
    let objects = if predicate.coercion() == Coercion::Json {
        vec![expand_object(Coercion::Json, authored.clone(), datatype())?]
    } else {
        let values: Vec<&Value> = match authored {
            Value::Array(items) => items.iter().collect(),
            _ => vec![authored],
        };
        values
            .into_iter()
            .filter(|value| !value.is_null())
            .map(|value| expand_fact_object(predicate.coercion(), value, datatype()))
            .collect::<Result<Vec<_>, _>>()?
    };
    Ok((predicate, objects))
}

fn expand_fact_object(
    coercion: Coercion,
    authored: &Value,
    json_datatype: Option<NodeUri>,
) -> Result<ObjectTerm, ContextError> {
    match authored {
        Value::Array(_) => Err(ContextError::InvalidFactObject),
        Value::Object(members) if members.len() == 1 && members.contains_key("@id") => {
            expand_object(Coercion::NodeId, members["@id"].clone(), None)
        }
        Value::Object(members) if members.len() == 1 && members.contains_key("@value") => {
            match &members["@value"] {
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                    Ok(ObjectTerm::value(members["@value"].clone()))
                }
                _ => Err(ContextError::InvalidFactObject),
            }
        }
        Value::Object(members)
            if members.len() == 2
                && members.get("@type") == Some(&Value::String("@json".to_owned()))
                && members.contains_key("@value") =>
        {
            expand_object(Coercion::Json, members["@value"].clone(), json_datatype)
        }
        Value::Object(_) => Err(ContextError::InvalidFactObject),
        _ => expand_object(coercion, authored.clone(), json_datatype),
    }
}

fn resolve_value(
    mut parent: EffectiveContext,
    authored: &Value,
    resources: &ContextResources,
    base: ContextBase<'_>,
    seen: &mut BTreeSet<String>,
    active: &mut BTreeSet<String>,
) -> Result<EffectiveContext, ContextError> {
    match authored {
        Value::Object(object) => {
            apply_inline(&mut parent, object)?;
            Ok(parent)
        }
        Value::String(reference) => {
            let imported = resolve_import(reference, resources, base, seen, active)?;
            apply_imports(&mut parent, imported)?;
            Ok(parent)
        }
        Value::Array(items) if !items.is_empty() => {
            if !items[0].is_string() {
                return Err(ContextError::InvalidForm);
            }
            let mut imports = EffectiveContext::default();
            let mut inline = None;
            for (index, item) in items.iter().enumerate() {
                match item {
                    Value::String(reference) if inline.is_none() => {
                        let imported = resolve_import(reference, resources, base, seen, active)?;
                        merge_import(&mut imports, imported)?;
                    }
                    Value::Object(object) if index + 1 == items.len() && inline.is_none() => {
                        inline = Some(object)
                    }
                    _ => return Err(ContextError::InvalidForm),
                }
            }
            apply_imports(&mut parent, imports)?;
            if let Some(object) = inline {
                apply_inline(&mut parent, object)?;
            }
            Ok(parent)
        }
        _ => Err(ContextError::InvalidForm),
    }
}

fn resolve_import(
    reference: &str,
    resources: &ContextResources,
    base: ContextBase<'_>,
    seen: &mut BTreeSet<String>,
    active: &mut BTreeSet<String>,
) -> Result<EffectiveContext, ContextError> {
    let hashed = reference.starts_with("morphir://context/sha256/");
    let identity = if hashed {
        let digest = reference
            .strip_prefix("morphir://context/sha256/")
            .expect("hashed reference has the checked prefix");
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(ContextError::InvalidForm);
        }
        reference.to_owned()
    } else {
        if reference.contains("://") {
            return Err(ContextError::RemoteForbidden(reference.to_owned()));
        }
        if reference.starts_with('/') {
            return Err(ContextError::PathEscape);
        }
        if !reference.ends_with(".jsonld") {
            return Err(ContextError::InvalidForm);
        }
        let candidate = match base {
            ContextBase::Workspace if reference.starts_with(&format!("{}/", resources.root)) => {
                reference.to_owned()
            }
            ContextBase::Workspace => format!("{}/{}", resources.root, reference),
            ContextBase::Local(file) => format!(
                "{}/{}",
                file.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("."),
                reference
            ),
            ContextBase::ContentAddressed => return Err(ContextError::RelativeImportWithoutBase),
        };
        normalize_path(&candidate, &resources.root)?
    };
    if active.contains(&identity) {
        return Err(ContextError::ImportCycle(identity));
    }
    if active.len() >= MAX_CONTEXT_IMPORT_DEPTH {
        return Err(ContextError::ImportDepthExceeded(MAX_CONTEXT_IMPORT_DEPTH));
    }
    if !seen.insert(identity.clone()) {
        return Err(ContextError::DuplicateImport(identity));
    }
    let loaded = resources
        .loaded
        .get(&identity)
        .ok_or_else(|| ContextError::ResourceUnavailable(identity.clone()))?;
    if hashed {
        if !loaded.trusted {
            return Err(ContextError::ResourceUntrusted(identity));
        }
        let actual = Sha256::digest(&loaded.bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        if !reference.ends_with(&actual) {
            return Err(ContextError::DigestMismatch(reference.to_owned()));
        }
    }
    let document: Value = serde_json::from_slice::<UniqueValue>(&loaded.bytes)
        .map(|value| value.0)
        .map_err(|_| ContextError::InvalidResource(identity.clone()))?;
    let envelope = document
        .as_object()
        .filter(|object| object.len() == 1)
        .and_then(|object| object.get("@context"))
        .ok_or_else(|| ContextError::InvalidResource(identity.clone()))?;
    active.insert(identity.clone());
    let result = resolve_value(
        EffectiveContext::default(),
        envelope,
        resources,
        if hashed {
            ContextBase::ContentAddressed
        } else {
            ContextBase::Local(&identity)
        },
        seen,
        active,
    );
    active.remove(&identity);
    result
}

fn normalize_path(path: &str, root: &str) -> Result<String, ContextError> {
    let normalized = normalize_lexical_path(path)?;
    let within_root = match root {
        "." => !normalized.starts_with('/'),
        "/" => normalized.starts_with('/'),
        _ => normalized == root || normalized.starts_with(&format!("{root}/")),
    };
    if within_root {
        Ok(normalized)
    } else {
        Err(ContextError::PathEscape)
    }
}

fn normalize_lexical_path(path: &str) -> Result<String, ContextError> {
    let absolute = path.starts_with('/');
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(ContextError::PathEscape);
                }
            }
            _ => parts.push(part),
        }
    }
    let joined = parts.join("/");
    if absolute {
        Ok(format!("/{joined}"))
    } else if joined.is_empty() {
        Ok(".".to_owned())
    } else {
        Ok(joined)
    }
}

fn merge_import(
    target: &mut EffectiveContext,
    imported: EffectiveContext,
) -> Result<(), ContextError> {
    for (key, binding) in imported.terms {
        if target.terms.get(&key).is_some_and(|old| old != &binding) {
            return Err(ContextError::TermCollision(key));
        }
        target.terms.insert(key, binding);
    }
    if let Some(vocab) = imported.vocab {
        if target.vocab.as_ref().is_some_and(|old| old != &vocab) {
            return Err(ContextError::VocabCollision);
        }
        target.vocab = Some(vocab);
    }
    Ok(())
}

fn apply_imports(
    target: &mut EffectiveContext,
    imported: EffectiveContext,
) -> Result<(), ContextError> {
    for (key, binding) in imported.terms {
        if target
            .terms
            .get(&key)
            .is_some_and(|old| old.protected && old != &binding)
        {
            return Err(ContextError::ProtectedTermRedefinition(key));
        }
        target.terms.insert(key, binding);
    }
    if imported.vocab.is_some() {
        target.vocab = imported.vocab;
    }
    Ok(())
}

fn apply_inline(
    target: &mut EffectiveContext,
    object: &Map<String, Value>,
) -> Result<(), ContextError> {
    if let Some(key) = object
        .keys()
        .find(|key| key.starts_with('@') && key.as_str() != "@vocab")
    {
        return Err(ContextError::UnsupportedKeyword(key.clone()));
    }
    if let Some(vocab) = object.get("@vocab") {
        let stem = vocab.as_str().ok_or(ContextError::InvalidForm)?;
        validate_stem(stem)?;
        target.vocab = Some(stem.to_owned());
    }
    // Prefixes are installed first, so term definitions can refer to a prefix
    // declared later in the same JSON object.
    for (key, value) in object.iter().filter(|(key, _)| !key.starts_with('@')) {
        if value.as_object().and_then(|obj| obj.get("@prefix")) == Some(&Value::Bool(true)) {
            install_binding(target, key, value)?;
        }
    }
    for (key, value) in object.iter().filter(|(key, value)| {
        !key.starts_with('@')
            && value.as_object().and_then(|obj| obj.get("@prefix")) != Some(&Value::Bool(true))
    }) {
        install_binding(target, key, value)?;
    }
    Ok(())
}

fn install_binding(
    target: &mut EffectiveContext,
    key: &str,
    value: &Value,
) -> Result<(), ContextError> {
    if key.is_empty()
        || key.chars().any(char::is_whitespace)
        || key.split_once(':').is_some_and(|(prefix, suffix)| {
            prefix.is_empty() || suffix.is_empty() || suffix.contains(':')
        })
    {
        return Err(ContextError::InvalidForm);
    }
    let (raw, prefix, coercion, protected) = match value {
        Value::String(raw) => (raw.as_str(), false, Coercion::None, false),
        Value::Object(object) => {
            if let Some(member) = object.keys().find(|member| {
                !matches!(member.as_str(), "@id" | "@type" | "@prefix" | "@protected")
            }) {
                return Err(if member.starts_with('@') {
                    ContextError::UnsupportedKeyword(member.clone())
                } else {
                    ContextError::InvalidForm
                });
            }
            let raw = object
                .get("@id")
                .and_then(Value::as_str)
                .ok_or(ContextError::InvalidForm)?;
            let prefix = object
                .get("@prefix")
                .map(Value::as_bool)
                .transpose_bool()?
                .unwrap_or(false);
            if object.contains_key("@prefix") && !prefix {
                return Err(ContextError::InvalidForm);
            }
            let protected = object
                .get("@protected")
                .map(Value::as_bool)
                .transpose_bool()?
                .unwrap_or(false);
            let coercion = match object.get("@type") {
                None => Coercion::None,
                Some(Value::String(value)) if value == "@id" => Coercion::NodeId,
                Some(Value::String(value)) if value == "@json" => Coercion::Json,
                _ => return Err(ContextError::InvalidForm),
            };
            (raw, prefix, coercion, protected)
        }
        _ => return Err(ContextError::InvalidForm),
    };
    let expanded = if raw.starts_with("morphir://ir/") {
        raw.to_owned()
    } else if let Some((head, tail)) = raw.split_once(':') {
        target
            .terms
            .get(head)
            .filter(|binding| binding.prefix)
            .map(|binding| format!("{}{tail}", binding.target))
            .ok_or_else(|| ContextError::InvalidTarget(raw.to_owned()))?
    } else {
        return Err(ContextError::InvalidTarget(raw.to_owned()));
    };
    if prefix {
        if coercion != Coercion::None {
            return Err(ContextError::InvalidForm);
        }
        validate_stem(&expanded)?;
    } else {
        parse_uri(&expanded)?;
    }
    let binding = Binding {
        target: expanded,
        prefix,
        coercion,
        protected,
    };
    if target
        .terms
        .get(key)
        .is_some_and(|old| old.protected && old != &binding)
    {
        return Err(ContextError::ProtectedTermRedefinition(key.to_owned()));
    }
    target.terms.insert(key.to_owned(), binding);
    Ok(())
}

fn parse_uri(raw: &str) -> Result<NodeUri, ContextError> {
    let uri = NodeUri::parse(raw).map_err(|_| ContextError::InvalidTarget(raw.to_owned()))?;
    if !matches!(uri.root(), NodeRoot::Type { .. } | NodeRoot::Value { .. })
        || !uri.steps().is_empty()
    {
        return Err(ContextError::InvalidTarget(raw.to_owned()));
    }
    Ok(uri)
}

fn validate_stem(stem: &str) -> Result<(), ContextError> {
    if !stem.ends_with('/') {
        return Err(ContextError::InvalidTarget(stem.to_owned()));
    }
    parse_uri(&format!("{stem}sample"))
        .map_err(|_| ContextError::InvalidTarget(stem.to_owned()))?;
    Ok(())
}

// A parsed `Value` cannot reveal duplicate object members. Context resources
// enter as bytes, so reject duplicates recursively before constructing one.
struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(UniqueValueVisitor)
    }
}

struct UniqueValueVisitor;

impl<'de> Visitor<'de> for UniqueValueVisitor {
    type Value = UniqueValue;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a context JSON value without duplicate members")
    }

    fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Bool(value)))
    }

    fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(value.into())))
    }

    fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(value.into())))
    }

    fn visit_f64<E: serde::de::Error>(self, value: f64) -> Result<Self::Value, E> {
        serde_json::Number::from_f64(value)
            .map(|number| UniqueValue(Value::Number(number)))
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::String(value.to_owned())))
    }

    fn visit_string<E: serde::de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::String(value)))
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element::<UniqueValue>()? {
            values.push(value.0);
        }
        Ok(UniqueValue(Value::Array(values)))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate context member: {key}"
                )));
            }
            values.insert(key, map.next_value::<UniqueValue>()?.0);
        }
        if values.len() == 1
            && let Some(Value::String(lexeme)) = values.get("$serde_json::private::Number")
        {
            return lexeme
                .parse::<serde_json::Number>()
                .map(|number| UniqueValue(Value::Number(number)))
                .map_err(serde::de::Error::custom);
        }
        Ok(UniqueValue(Value::Object(values)))
    }
}

trait BoolValue {
    fn transpose_bool(self) -> Result<Option<bool>, ContextError>;
}
impl BoolValue for Option<Option<bool>> {
    fn transpose_bool(self) -> Result<Option<bool>, ContextError> {
        self.map(|value| value.ok_or(ContextError::InvalidForm))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::UniqueValue;
    use serde_json::Value;

    #[test]
    fn imported_json_numbers_keep_their_numeric_kind_and_lexeme() {
        for source in ["1", "1.25", "18446744073709551616"] {
            let value = serde_json::from_slice::<UniqueValue>(source.as_bytes())
                .unwrap()
                .0;
            assert!(matches!(value, Value::Number(_)));
            assert_eq!(value.to_string(), source);
        }
    }
}
