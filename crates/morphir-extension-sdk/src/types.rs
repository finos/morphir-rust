//! Core types for the Morphir extension system
//!
//! These types are shared between the SDK (guest) and daemon (host).

use serde::ser::{Error as _, SerializeMap};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Extension type/capability
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtensionType {
    /// Frontend - parses source into IR
    Frontend,
    /// Backend - generates code from IR
    Backend,
    /// Transform - transforms IR to IR
    Transform,
    /// Validator - analyzes IR and produces diagnostics
    Validator,
    /// Workspace - discovers Morphir projects from a confined file tree
    Workspace,
}

/// Information about an extension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionInfo {
    /// Extension identifier (e.g., "morphir-gleam-binding")
    pub id: String,
    /// Human-readable name
    pub name: String,
    /// Version (semver)
    pub version: String,
    /// Description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Capabilities this extension provides
    pub types: Vec<ExtensionType>,
    /// Author
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Homepage URL
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// License (SPDX identifier)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Minimum SDK version required
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_sdk_version: Option<String>,
}

impl Default for ExtensionInfo {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            version: "0.1.0".to_string(),
            description: None,
            types: Vec::new(),
            author: None,
            homepage: None,
            license: None,
            min_sdk_version: None,
        }
    }
}

/// Source language supported by a frontend extension.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageCapability {
    /// Stable language identifier used in compile requests.
    pub id: String,
    /// File extensions recognized for this language.
    pub file_extensions: Vec<String>,
}

/// Compilation features advertised by a frontend extension.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendCapability {
    /// Source languages accepted by the frontend.
    pub languages: Vec<LanguageCapability>,
    /// Morphir IR versions the frontend can produce.
    pub ir_versions: Vec<String>,
    /// Whether the frontend accepts compile requests.
    pub compile: bool,
    /// Whether the frontend accepts `CompileRequest.baseline` and returns `CompileResult.moduleResults`.
    pub incremental: bool,
    /// Whether the frontend can compile source fragments.
    pub fragments: bool,
    /// Whether one compile request may submit more than one document.
    ///
    /// A frontend that does not declare this compiles exactly one document
    /// per request, and a host must refuse a larger source set before
    /// invoking it rather than let the frontend drop or reject documents.
    /// Written only when true, so a single-document frontend's capabilities
    /// are unchanged on the wire.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multi_document: bool,
}

/// Code-generation features advertised by a backend extension.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendCapability {
    /// Target formats the backend can generate.
    pub targets: Vec<String>,
    /// Morphir IR versions the backend can consume.
    pub ir_versions: Vec<String>,
    /// Whether the backend accepts generate requests.
    pub generate: bool,
}

/// Workspace-discovery features advertised by an extension.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCapability {
    /// Portable workspace discovery protocol versions accepted by the extension.
    pub protocol_versions: Vec<u32>,
    /// Whether the extension accepts workspace discovery requests.
    pub discover: bool,
}

/// Extension capabilities for runtime negotiation
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ExtensionCapabilities {
    /// Frontend compilation features, when provided by the extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontend: Option<FrontendCapability>,
    /// Backend code-generation features, when provided by the extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<BackendCapability>,
    /// Workspace discovery features, when provided by the extension.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceCapability>,
    /// Supports streaming/incremental processing
    #[serde(default)]
    pub streaming: bool,
    /// Supports incremental compilation
    #[serde(default)]
    pub incremental: bool,
    /// Supports cancellation
    #[serde(default)]
    pub cancellation: bool,
    /// Supports progress reporting
    #[serde(default)]
    pub progress: bool,
    /// Additional capability values reserved for protocol extensions.
    ///
    /// Keys that duplicate a known capability field are rejected during serialization.
    #[serde(default, flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

impl Serialize for ExtensionCapabilities {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        const RESERVED_KEYS: [&str; 7] = [
            "frontend",
            "backend",
            "workspace",
            "streaming",
            "incremental",
            "cancellation",
            "progress",
        ];
        if let Some(key) = self
            .extra
            .keys()
            .find(|key| RESERVED_KEYS.contains(&key.as_str()))
        {
            return Err(S::Error::custom(format!(
                "extra contains reserved capability key '{key}'"
            )));
        }

        let mut map = serializer.serialize_map(Some(
            4 + usize::from(self.frontend.is_some())
                + usize::from(self.backend.is_some())
                + usize::from(self.workspace.is_some())
                + self.extra.len(),
        ))?;
        if let Some(frontend) = &self.frontend {
            map.serialize_entry("frontend", frontend)?;
        }
        if let Some(backend) = &self.backend {
            map.serialize_entry("backend", backend)?;
        }
        if let Some(workspace) = &self.workspace {
            map.serialize_entry("workspace", workspace)?;
        }
        map.serialize_entry("streaming", &self.streaming)?;
        map.serialize_entry("incremental", &self.incremental)?;
        map.serialize_entry("cancellation", &self.cancellation)?;
        map.serialize_entry("progress", &self.progress)?;
        for (key, value) in &self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// Resource limits for extension execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Maximum memory in bytes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_memory_bytes: Option<u64>,
    /// Maximum execution time in milliseconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_time_ms: Option<u64>,
    /// Maximum fuel (instruction count)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fuel: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_memory_bytes: Some(256 * 1024 * 1024), // 256 MB
            max_time_ms: Some(30_000),                 // 30 seconds
            max_fuel: Some(100_000_000),               // 100M instructions
        }
    }
}

/// Source document supplied to a frontend compiler.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDocument {
    /// URI that identifies the document.
    pub uri: String,
    /// Language identifier understood by the frontend.
    pub language_id: String,
    /// Monotonically increasing document version.
    pub version: u64,
    /// Complete source text for this document version.
    pub text: String,
}

/// Package metadata for a frontend compilation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompilePackage {
    /// Morphir package name.
    pub name: String,
    /// Exact public module list. Omission exposes all modules; an empty list exposes none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exposed_modules: Option<Vec<String>>,
}

/// A package distribution available to a frontend compilation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileDependency {
    /// Name of the dependency package.
    pub package_name: String,
    /// Morphir IR version used by the distribution.
    pub ir_version: String,
    /// Serialized Morphir distribution.
    pub distribution: serde_json::Value,
}

/// Legacy options-bag keys that used to carry a compilation's source root.
///
/// A caller that sends one of these *alongside* the current `sources` envelope
/// would otherwise lose its root silently and get different module names for
/// the same files, so there they are rejected rather than ignored. They are
/// only meaningful as part of the legacy envelope described on
/// [`SourceEnvelope`]. See [`SourceSet::root`].
const LEGACY_SOURCE_ROOT_KEYS: [&str; 2] = ["sourceRootUri", "sourceRoot"];

pub(crate) fn reject_legacy_source_root_keys(
    extra: &HashMap<String, serde_json::Value>,
) -> Result<(), String> {
    if let Some(key) = extra
        .keys()
        .find(|key| LEGACY_SOURCE_ROOT_KEYS.contains(&key.as_str()))
    {
        return Err(format!(
            "'{key}' is no longer a compile option; supply the root as sources.root"
        ));
    }
    Ok(())
}

/// Which envelope a compile request stated its sources in.
///
/// # Transitional — delete the legacy variant
///
/// The current envelope is [`CompileRequest::sources`]: a [`SourceSet`] that
/// carries the documents *and* the root their module identities resolve
/// against. Hosts released before that change send the documents at the
/// request's top level and the root as one of [`LEGACY_SOURCE_ROOT_KEYS`] in
/// the options bag. This enum exists only so those hosts keep working until a
/// morphir release ships a host that speaks `sources`.
///
/// Deleting it then means deleting this enum, [`take_legacy_source_root`],
/// `CompileOptionsWire` and the `legacy_compile_envelope_tests` module, after
/// which [`CompileRequest`] derives `Deserialize` again.
/// [`LEGACY_SOURCE_ROOT_KEYS`] and [`reject_legacy_source_root_keys`] stay:
/// once nothing accepts the legacy envelope, the keys are simply wrong
/// everywhere, which is what those two already say.
///
/// A request is *wholly* one envelope or *wholly* the other, never a mixture.
/// Both envelopes name a source root, and a request that supplied two of them
/// would have no honest answer for which one module names resolve against —
/// the exact silent renaming that moving the root into [`SourceSet`] exists to
/// prevent. So a modern request keeps hard-rejecting the legacy option keys,
/// and a request carrying both `sources` and a top-level `documents` is
/// refused rather than resolved by a precedence rule.
///
/// This is deliberately not a `#[serde(untagged)]` enum: an untagged enum
/// reports a failure to match as "data did not match any variant", which
/// cannot distinguish "you sent both envelopes" from "you sent neither" or
/// from a malformed document inside one of them. The two optional fields are
/// deserialized separately and the choice between them is made — and
/// diagnosed — here.
enum SourceEnvelope {
    /// The current envelope: documents and root travel together.
    Modern(SourceSet),
    /// The pre-`sources` envelope: documents at the top level, with the root,
    /// if any, in the options bag.
    Legacy(Vec<SourceDocument>),
}

impl SourceEnvelope {
    /// Message for a request that states its sources both ways at once.
    const MIXED: &'static str = "a compile request carries both 'sources' and a legacy top-level \
                                 'documents'; they are two different envelopes, each naming its \
                                 own source root, so which root module names resolve against is \
                                 ambiguous — send 'sources' alone";

    /// Reduce the envelope to the one shape the rest of the SDK knows about.
    ///
    /// The legacy root key is *moved* out of `extra`, not copied: no consumer
    /// downstream of deserialization ever learns that a legacy request
    /// existed, and `CompileOptions`' serializer — which still refuses a
    /// legacy key in `extra` — can re-emit the normalized request.
    fn normalize(
        self,
        extra: &mut HashMap<String, serde_json::Value>,
    ) -> Result<SourceSet, String> {
        match self {
            Self::Modern(sources) => {
                reject_legacy_source_root_keys(extra)?;
                Ok(sources)
            }
            Self::Legacy(documents) => Ok(SourceSet {
                root: take_legacy_source_root(extra)?,
                documents,
            }),
        }
    }
}

/// Remove the legacy envelope's source root from an options bag.
///
/// `null` reads as "no root", matching how `"root": null` deserializes on
/// [`SourceSet`], rather than as the type error the pre-`sources` accessor
/// raised. Any other non-string value is a type error, as it was then.
///
/// Both legacy keys are accepted because both were tolerated before: the SDK
/// read the root from `sourceRootUri`, while `sourceRoot` was allowed through
/// per-frontend option allowlists. A request carrying both is fine while they
/// agree and an error when they do not — two disagreeing roots are the same
/// ambiguity a mixed envelope is refused for.
fn take_legacy_source_root(
    extra: &mut HashMap<String, serde_json::Value>,
) -> Result<Option<String>, String> {
    let mut found: Option<(&str, Option<String>)> = None;
    for key in LEGACY_SOURCE_ROOT_KEYS {
        let Some(value) = extra.remove(key) else {
            continue;
        };
        let root = match value {
            serde_json::Value::String(root) => Some(root),
            serde_json::Value::Null => None,
            _ => return Err(format!("'{key}' must be a string")),
        };
        match &found {
            Some((first, first_root)) if *first_root != root => {
                return Err(format!(
                    "a legacy compile request supplies both '{first}' and '{key}', and they name \
                     different source roots; module names resolve against exactly one root"
                ));
            }
            _ => found = Some((key, root)),
        }
    }
    Ok(found.and_then(|(_, root)| root))
}

/// Options that control frontend compilation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CompileOptions {
    /// Emit type information without value bodies when supported.
    pub types_only: bool,
    /// Morphir IR version the frontend should produce.
    pub ir_version: String,
    /// Frontend-specific compilation options.
    ///
    /// Keys that duplicate `typesOnly` or `irVersion`, or either legacy source
    /// root key (`sourceRootUri`, `sourceRoot`), are rejected during
    /// serialization: a root belongs in [`SourceSet::root`], and this SDK
    /// never writes the legacy envelope.
    ///
    /// Deserializing options *on their own* rejects the legacy keys too, since
    /// options outside a request carry no envelope that could make them legal.
    /// Within a request the envelope decides: a request in the transitional
    /// legacy envelope states its root with one of those keys, and it is moved
    /// into [`SourceSet::root`] before any of this is populated.
    pub extra: HashMap<String, serde_json::Value>,
}

impl Serialize for CompileOptions {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        const RESERVED_KEYS: [&str; 4] = ["typesOnly", "irVersion", "sourceRootUri", "sourceRoot"];
        if let Some(key) = self
            .extra
            .keys()
            .find(|key| RESERVED_KEYS.contains(&key.as_str()))
        {
            return Err(S::Error::custom(format!(
                "extra contains reserved compile option key '{key}'"
            )));
        }

        let mut map = serializer.serialize_map(Some(2 + self.extra.len()))?;
        map.serialize_entry("typesOnly", &self.types_only)?;
        map.serialize_entry("irVersion", &self.ir_version)?;
        for (key, value) in &self.extra {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// The options bag exactly as it arrives, before any envelope rule is applied.
///
/// [`CompileOptions`]' own `Deserialize` rejects the legacy source-root keys,
/// which is right for options deserialized on their own but wrong inside a
/// legacy request, where the root key is the request's root. A
/// [`CompileRequest`] therefore deserializes this helper and applies the rule
/// its envelope calls for. It can be folded back into
/// `CompileOptions::deserialize` once the legacy envelope goes.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompileOptionsWire {
    types_only: bool,
    ir_version: String,
    #[serde(default, flatten)]
    extra: HashMap<String, serde_json::Value>,
}

impl<'de> Deserialize<'de> for CompileOptions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = CompileOptionsWire::deserialize(deserializer)?;
        reject_legacy_source_root_keys(&wire.extra).map_err(serde::de::Error::custom)?;
        Ok(CompileOptions {
            types_only: wire.types_only,
            ir_version: wire.ir_version,
            extra: wire.extra,
        })
    }
}

/// The documents a compilation submits, together with the root their module
/// identities are derived against.
///
/// The root belongs to the set rather than to the request: replacing or
/// combining document sets while a root sits elsewhere silently renames
/// modules, because a module's name is a function of its path relative to the
/// root.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSet {
    /// The root module identities resolve against, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// The documents to compile.
    pub documents: Vec<SourceDocument>,
}

/// Request to compile source documents into Morphir IR.
///
/// Serialization always writes the current envelope: `sources` carrying the
/// documents and their root. Deserialization also accepts, transitionally, the
/// envelope hosts released before `sources` send — top-level `documents` with
/// the root in `options.extra` — and normalizes it into this shape. A request
/// is wholly one or wholly the other; stating both is an error. See
/// `SourceEnvelope` in this module for the rule and for what to delete when
/// the legacy envelope goes.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileRequest {
    /// Language identifier shared by the submitted documents.
    pub language_id: String,
    /// Source documents to compile, together with the root their module
    /// identities are derived against.
    ///
    /// A legacy request states the same thing as a top-level `documents` list
    /// plus a root in `options.extra`; it is normalized into this field during
    /// deserialization, so nothing downstream sees the difference.
    pub sources: SourceSet,
    /// Package metadata for the compilation unit.
    pub package: CompilePackage,
    /// Package distributions available to the compilation.
    #[serde(default)]
    pub dependencies: Vec<CompileDependency>,
    /// Options that control the produced Morphir IR.
    pub options: CompileOptions,
    /// Baseline from a prior compilation, for incremental frontends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<CompileBaseline>,
}

impl<'de> Deserialize<'de> for CompileRequest {
    /// Accept either envelope and produce the one request shape.
    ///
    /// This is a hand-written impl only because of the legacy envelope; it is
    /// otherwise exactly what `#[derive(Deserialize)]` produced, down to the
    /// missing-field error naming `sources` for a request that states no
    /// sources at all. Deleting `SourceEnvelope` restores the derive.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct CompileRequestWire {
            language_id: String,
            #[serde(default)]
            sources: Option<SourceSet>,
            /// The legacy envelope's documents. Absent from every request a
            /// current host writes.
            #[serde(default)]
            documents: Option<Vec<SourceDocument>>,
            package: CompilePackage,
            #[serde(default)]
            dependencies: Vec<CompileDependency>,
            options: CompileOptionsWire,
            #[serde(default)]
            baseline: Option<CompileBaseline>,
        }

        let wire = CompileRequestWire::deserialize(deserializer)?;
        let envelope = match (wire.sources, wire.documents) {
            (Some(_), Some(_)) => {
                return Err(serde::de::Error::custom(SourceEnvelope::MIXED));
            }
            (Some(sources), None) => SourceEnvelope::Modern(sources),
            (None, Some(documents)) => SourceEnvelope::Legacy(documents),
            (None, None) => return Err(serde::de::Error::missing_field("sources")),
        };

        let mut extra = wire.options.extra;
        let sources = envelope
            .normalize(&mut extra)
            .map_err(serde::de::Error::custom)?;

        Ok(CompileRequest {
            language_id: wire.language_id,
            sources,
            package: wire.package,
            dependencies: wire.dependencies,
            options: CompileOptions {
                types_only: wire.options.types_only,
                ir_version: wire.options.ir_version,
                extra,
            },
            baseline: wire.baseline,
        })
    }
}

/// Result of compilation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileResult {
    /// Whether compilation succeeded
    pub success: bool,
    /// Morphir IR version of the compiled output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir_version: Option<String>,
    /// Compiled IR (JSON)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir: Option<serde_json::Value>,
    /// Diagnostics
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
    /// Module names produced by the compilation.
    #[serde(default)]
    pub modules: Vec<String>,
    /// Per-module incremental compilation results, when the frontend supports them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub module_results: Vec<ModuleResult>,
    /// Digest of the compile context these results were produced under, when the
    /// frontend computes one. A host stores it next to the module results it
    /// keeps and echoes it back as [`CompileBaseline::context_digest`]; a run
    /// under a different context cannot reuse them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_digest: Option<String>,
}

/// A module baseline captured from a prior compilation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaselineModule {
    /// Module name.
    pub name: String,
    /// URI of the module's source document.
    pub uri: String,
    /// Digest of the module's source text.
    pub source_digest: String,
    /// Digest of the module's resolved public interface.
    pub interface_digest: String,
    /// Names of modules this module depends on.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Previously compiled IR for this module.
    pub ir: serde_json::Value,
}

/// Baseline supplied by the host for incremental compilation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileBaseline {
    /// Modules known from a prior compilation.
    #[serde(default)]
    pub modules: Vec<BaselineModule>,
    /// Digest of the compile context the baseline was built under, as the
    /// producing run reported it in [`CompileResult::context_digest`].
    ///
    /// Everything a module's compiled form depends on besides its own source —
    /// the IR version, the frontend's configuration, and the dependency
    /// distributions supplied with the request — is folded into this one value.
    /// A run whose context digest differs, or a baseline that carries none,
    /// describes resolution against something else and cannot be reused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_digest: Option<String>,
}

/// Outcome of compiling (or reusing) a single module in an incremental compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleStatus {
    /// The module was recompiled.
    Compiled,
    /// The module was unchanged and reused from the baseline.
    Unchanged,
    /// The module failed to compile.
    Failed,
    /// The module was blocked by a failure in a dependency.
    Blocked,
}

/// Per-module result of an incremental compilation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleResult {
    /// Module name.
    pub name: String,
    /// URI of the module's source document.
    pub uri: String,
    /// Outcome for this module.
    pub status: ModuleStatus,
    /// Digest of the module's source text, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_digest: Option<String>,
    /// Digest of the module's resolved public interface, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface_digest: Option<String>,
    /// Names of modules this module depends on.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Compiled IR for this module, when produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir: Option<serde_json::Value>,
    /// Diagnostics for this module.
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// Request to generate code
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateRequest {
    /// Input IR (JSON)
    pub ir: serde_json::Value,
    /// Target selected by the host for this generation call.
    ///
    /// The host states the exact target ID it negotiated during provider
    /// selection. A backend that advertises more than one target dispatches on
    /// this value and never guesses a default.
    pub target: String,
    /// Generation options
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Result of code generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateResult {
    /// Whether generation succeeded
    pub success: bool,
    /// Generated artifacts
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    /// Diagnostics
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// Request to validate IR
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ValidateRequest {
    /// Input IR (JSON)
    pub ir: serde_json::Value,
    /// Validation options
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Result of validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateResult {
    /// Whether validation passed
    pub valid: bool,
    /// Diagnostics
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// Request to transform IR
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransformRequest {
    /// Input IR (JSON)
    pub ir: serde_json::Value,
    /// Transformation options
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Result of transformation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformResult {
    /// Whether transformation succeeded
    pub success: bool,
    /// Transformed IR (JSON)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir: Option<serde_json::Value>,
    /// Diagnostics
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

/// A diagnostic message
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Severity level
    pub severity: DiagnosticSeverity,
    /// Error/warning code
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Human-readable message
    pub message: String,
    /// Source location
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<SourceLocation>,
    /// Related information
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedInformation>,
}

/// Diagnostic severity level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSeverity {
    /// Error - compilation fails
    Error,
    /// Warning - may indicate problems
    Warning,
    /// Information - neutral message
    Info,
    /// Hint - suggestion for improvement
    Hint,
}

/// Source code location identified by URI and zero-based range.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceLocation {
    /// URI of the source document.
    pub uri: String,
    /// Zero-based range within the source document.
    pub range: SourceRange,
}

/// Half-open range between two zero-based source positions.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRange {
    /// Inclusive start position.
    pub start: SourcePosition,
    /// Exclusive end position.
    pub end: SourcePosition,
}

/// Zero-based line and character position in a source document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePosition {
    /// Zero-based line number.
    pub line: u32,
    /// Zero-based UTF-16 code-unit offset within the line, following LSP conventions.
    pub character: u32,
}

impl SourcePosition {
    /// Construct a position from a zero-based line number and the source text before the position.
    ///
    /// `source_line_prefix` must contain only text from the specified line before the position.
    ///
    /// # Panics
    ///
    /// Panics if the prefix contains more UTF-16 code units than fit in a [`u32`].
    pub fn from_line_prefix(line: u32, source_line_prefix: &str) -> Self {
        Self {
            line,
            character: u32::try_from(source_line_prefix.encode_utf16().count())
                .expect("source line prefix exceeds the supported UTF-16 offset"),
        }
    }
}

/// Related diagnostic information
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelatedInformation {
    /// Location of related information
    pub location: SourceLocation,
    /// Message
    pub message: String,
}

/// A generated artifact
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    /// Output path (relative)
    pub path: String,
    /// Content (text or base64 for binary)
    pub content: String,
    /// Whether content is base64-encoded binary
    #[serde(default)]
    pub binary: bool,
}

/// Workspace information provided by host
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    /// Workspace root path
    pub root: String,
    /// Output directory path
    pub output_dir: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_request_matches_mep_0_1() {
        let expected = serde_json::json!({
            "languageId": "elm",
            "sources": {
                "documents": [{
                    "uri": "file:///work/Example.elm",
                    "languageId": "elm",
                    "version": 1,
                    "text": "module Example exposing (add)\n"
                }]
            },
            "package": {
                "name": "local/example",
                "exposedModules": ["Example"]
            },
            "dependencies": [],
            "options": {
                "typesOnly": false,
                "irVersion": "3"
            }
        });
        let request = CompileRequest {
            language_id: "elm".into(),
            sources: SourceSet {
                root: None,
                documents: vec![SourceDocument {
                    uri: "file:///work/Example.elm".into(),
                    language_id: "elm".into(),
                    version: 1,
                    text: "module Example exposing (add)\n".into(),
                }],
            },
            package: CompilePackage {
                name: "local/example".into(),
                exposed_modules: Some(vec!["Example".into()]),
            },
            dependencies: vec![],
            options: CompileOptions {
                types_only: false,
                ir_version: "3".into(),
                extra: HashMap::new(),
            },
            baseline: None,
        };

        assert_eq!(serde_json::to_value(request).unwrap(), expected);
        let decoded: CompileRequest = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
    }

    #[test]
    fn compile_request_serializes_dependencies_and_vendor_options() {
        let expected = serde_json::json!({
            "languageId": "elm",
            "sources": {
                "documents": []
            },
            "package": {
                "name": "local/example",
                "exposedModules": []
            },
            "dependencies": [{
                "packageName": "morphir/sdk",
                "irVersion": "3",
                "distribution": {"modules": {}}
            }],
            "options": {
                "typesOnly": true,
                "irVersion": "3",
                "vendorOptimization": {"level": 2}
            }
        });
        let request = CompileRequest {
            language_id: "elm".into(),
            sources: SourceSet::default(),
            package: CompilePackage {
                name: "local/example".into(),
                exposed_modules: Some(vec![]),
            },
            dependencies: vec![CompileDependency {
                package_name: "morphir/sdk".into(),
                ir_version: "3".into(),
                distribution: serde_json::json!({"modules": {}}),
            }],
            options: CompileOptions {
                types_only: true,
                ir_version: "3".into(),
                extra: HashMap::from([(
                    "vendorOptimization".into(),
                    serde_json::json!({"level": 2}),
                )]),
            },
            baseline: None,
        };

        assert_eq!(serde_json::to_value(request).unwrap(), expected);
        let decoded: CompileRequest = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
    }

    #[test]
    fn compile_options_reject_ir_version_extra_collision() {
        let options = CompileOptions {
            types_only: false,
            ir_version: "3".into(),
            extra: HashMap::from([("irVersion".into(), serde_json::json!("4"))]),
        };

        let error = serde_json::to_value(options).unwrap_err();
        assert!(error.to_string().contains("reserved compile option key"));
    }

    #[test]
    fn compile_options_reject_types_only_extra_collision() {
        let options = CompileOptions {
            types_only: false,
            ir_version: "3".into(),
            extra: HashMap::from([("typesOnly".into(), serde_json::json!(true))]),
        };

        let error = serde_json::to_value(options).unwrap_err();
        assert!(error.to_string().contains("reserved compile option key"));
    }

    /// A root supplied through the old options bag alongside the current
    /// `sources` envelope is an error, not a silently ignored key: the request
    /// would then state a root twice and the two could disagree. Options
    /// deserialized on their own have no envelope that could make the key
    /// legal, so they reject it too. The one place it is accepted is the
    /// legacy envelope — see `legacy_compile_envelope_tests`.
    #[test]
    fn a_legacy_source_root_uri_option_is_rejected() {
        let error = serde_json::from_value::<CompileOptions>(serde_json::json!({
            "typesOnly": false,
            "irVersion": "4",
            "sourceRootUri": "file:///project/src",
        }))
        .unwrap_err();
        assert!(error.to_string().contains("sourceRootUri"), "{error}");

        // Rejected even when a valid root already travels with the documents:
        // the legacy key is not a fallback for a missing `sources.root`.
        let error = serde_json::from_value::<CompileRequest>(serde_json::json!({
            "languageId": "elm",
            "sources": {"root": "file:///project/src", "documents": []},
            "package": {"name": "local/example"},
            "options": {
                "typesOnly": false,
                "irVersion": "4",
                "sourceRootUri": "file:///project/src",
            },
        }))
        .unwrap_err();
        assert!(error.to_string().contains("sourceRootUri"), "{error}");
    }

    /// The Gleam-local fallback key is rejected too.
    #[test]
    fn a_legacy_source_root_option_is_rejected() {
        let error = serde_json::from_value::<CompileOptions>(serde_json::json!({
            "typesOnly": false,
            "irVersion": "4",
            "sourceRoot": "src",
        }))
        .unwrap_err();
        assert!(error.to_string().contains("sourceRoot"), "{error}");

        // Rejected even when a valid root already travels with the documents.
        let error = serde_json::from_value::<CompileRequest>(serde_json::json!({
            "languageId": "gleam",
            "sources": {"root": "file:///project/src", "documents": []},
            "package": {"name": "local/example"},
            "options": {
                "typesOnly": false,
                "irVersion": "4",
                "sourceRoot": "src",
            },
        }))
        .unwrap_err();
        assert!(error.to_string().contains("sourceRoot"), "{error}");
    }

    /// The root travels with the documents it applies to.
    #[test]
    fn a_source_set_carries_its_root() {
        let sources: SourceSet = serde_json::from_value(serde_json::json!({
            "root": "file:///project/src",
            "documents": [{
                "uri": "file:///project/src/domain/models.py",
                "languageId": "python",
                "version": 1,
                "text": ""
            }]
        }))
        .unwrap();
        assert_eq!(sources.root.as_deref(), Some("file:///project/src"));
        assert_eq!(sources.documents.len(), 1);

        // The root is optional: a set with no root deserializes with `None`.
        let rootless: SourceSet = serde_json::from_value(serde_json::json!({
            "documents": []
        }))
        .unwrap();
        assert_eq!(rootless.root, None);
    }

    /// Before the root was typed, `CompileOptions::source_root()` rejected an
    /// explicit `"sourceRootUri": null` with "must be a string" — a type
    /// error. `root` is now `Option<String>`, so `"root": null` deserializes
    /// as ordinary serde practice for an absent option, not a type error.
    /// Pinning this so the change is a recorded decision, not an accident.
    #[test]
    fn an_explicit_null_root_deserializes_as_no_root() {
        let sources: SourceSet = serde_json::from_value(serde_json::json!({
            "root": null,
            "documents": []
        }))
        .unwrap();
        assert_eq!(sources.root, None);
    }

    #[test]
    fn compile_result_matches_mep_0_1() {
        let expected = serde_json::json!({
            "success": true,
            "irVersion": "3",
            "ir": {"formatVersion": 3},
            "diagnostics": [{
                "severity": "warning",
                "code": "unused-value",
                "message": "Value is not exposed",
                "location": {
                    "uri": "file:///work/Example.elm",
                    "range": {
                        "start": {"line": 2, "character": 4},
                        "end": {"line": 2, "character": 7}
                    }
                }
            }],
            "modules": ["Example"]
        });

        let result = CompileResult {
            success: true,
            ir_version: Some("3".into()),
            ir: Some(serde_json::json!({"formatVersion": 3})),
            diagnostics: vec![Diagnostic {
                severity: DiagnosticSeverity::Warning,
                code: Some("unused-value".into()),
                message: "Value is not exposed".into(),
                location: Some(SourceLocation {
                    uri: "file:///work/Example.elm".into(),
                    range: SourceRange {
                        start: SourcePosition {
                            line: 2,
                            character: 4,
                        },
                        end: SourcePosition {
                            line: 2,
                            character: 7,
                        },
                    },
                }),
                related: vec![],
            }],
            modules: vec!["Example".into()],
            module_results: vec![],
            context_digest: None,
        };

        assert_eq!(serde_json::to_value(&result).unwrap(), expected);
        let decoded: CompileResult = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);

        let minimal: CompileResult = serde_json::from_value(serde_json::json!({
            "success": false
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(minimal).unwrap(),
            serde_json::json!({
                "success": false,
                "diagnostics": [],
                "modules": []
            })
        );
    }

    #[test]
    fn extension_capabilities_support_optional_frontend_contract() {
        let capabilities = ExtensionCapabilities {
            frontend: Some(FrontendCapability {
                languages: vec![LanguageCapability {
                    id: "elm".into(),
                    file_extensions: vec![".elm".into()],
                }],
                ir_versions: vec!["3".into()],
                compile: true,
                incremental: false,
                fragments: false,
                multi_document: false,
            }),
            ..ExtensionCapabilities::default()
        };
        let expected = serde_json::json!({
            "streaming": false,
            "incremental": false,
            "cancellation": false,
            "progress": false,
            "frontend": {
                "languages": [{"id": "elm", "fileExtensions": [".elm"]}],
                "irVersions": ["3"],
                "compile": true,
                "incremental": false,
                "fragments": false
            }
        });

        assert_eq!(serde_json::to_value(&capabilities).unwrap(), expected);
        let decoded: ExtensionCapabilities = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);

        let legacy = serde_json::json!({
            "streaming": true,
            "incremental": false,
            "cancellation": true,
            "progress": false,
            "vendorFeature": true
        });
        let decoded: ExtensionCapabilities = serde_json::from_value(legacy.clone()).unwrap();
        assert!(decoded.frontend.is_none());
        assert_eq!(serde_json::to_value(decoded).unwrap(), legacy);
    }

    #[test]
    fn a_multi_document_frontend_says_so_and_silence_means_one_document() {
        let frontend = FrontendCapability {
            compile: true,
            multi_document: true,
            ..FrontendCapability::default()
        };

        let wire = serde_json::to_value(&frontend).unwrap();
        assert_eq!(wire["multiDocument"], true);

        let mut silent = wire;
        silent.as_object_mut().unwrap().remove("multiDocument");
        let decoded: FrontendCapability = serde_json::from_value(silent).unwrap();
        assert!(!decoded.multi_document);
    }

    #[test]
    fn backend_capability_round_trips() {
        let capabilities = ExtensionCapabilities {
            backend: Some(BackendCapability {
                targets: vec!["avro".into()],
                ir_versions: vec!["3".into(), "4".into()],
                generate: true,
            }),
            ..ExtensionCapabilities::default()
        };
        let json = serde_json::to_value(&capabilities).unwrap();

        assert_eq!(json["backend"]["targets"], serde_json::json!(["avro"]));
        assert_eq!(json["backend"]["irVersions"], serde_json::json!(["3", "4"]));
        assert_eq!(json["backend"]["generate"], true);
        assert_eq!(
            serde_json::from_value::<ExtensionCapabilities>(json)
                .unwrap()
                .backend
                .unwrap()
                .targets,
            ["avro"]
        );
    }

    #[test]
    fn extra_cannot_replace_the_typed_backend_capability() {
        let capabilities = ExtensionCapabilities {
            extra: HashMap::from([("backend".into(), serde_json::json!({}))]),
            ..ExtensionCapabilities::default()
        };

        assert!(serde_json::to_value(capabilities).is_err());
    }

    #[test]
    fn extension_capabilities_preserve_unknown_structured_values() {
        let expected = serde_json::json!({
            "streaming": false,
            "incremental": false,
            "cancellation": false,
            "progress": false,
            "vendorFrontend": {
                "modes": ["batch", "watch"],
                "limits": {"documents": 100}
            }
        });

        let decoded: ExtensionCapabilities = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
    }

    #[test]
    fn extension_capabilities_reject_reserved_extra_keys() {
        for key in [
            "frontend",
            "streaming",
            "incremental",
            "cancellation",
            "progress",
        ] {
            let capabilities = ExtensionCapabilities {
                extra: HashMap::from([(key.into(), serde_json::json!({"override": true}))]),
                ..ExtensionCapabilities::default()
            };

            let error = serde_json::to_value(capabilities).unwrap_err();
            assert!(error.to_string().contains("reserved capability key"));
        }
    }

    #[test]
    fn source_position_counts_utf16_code_units() {
        let position = SourcePosition::from_line_prefix(4, "a😀");

        assert_eq!(position.line, 4);
        assert_eq!(position.character, 3);
    }
}

#[cfg(test)]
mod generate_request_tests {
    use super::*;

    #[test]
    fn decodes_a_request_that_states_its_target() {
        let request: GenerateRequest = serde_json::from_value(serde_json::json!({
            "ir": {"formatVersion": 4},
            "target": "json-schema",
            "options": {"unsupported": "warn-and-skip"}
        }))
        .expect("a request stating its target decodes");

        assert_eq!(request.target, "json-schema");
        assert_eq!(request.ir["formatVersion"], 4);
        assert_eq!(
            request.options.get("unsupported"),
            Some(&serde_json::json!("warn-and-skip"))
        );
    }

    #[test]
    fn rejects_a_request_with_no_target() {
        let error = serde_json::from_value::<GenerateRequest>(serde_json::json!({
            "ir": {"formatVersion": 4},
            "options": {}
        }))
        .expect_err("the host always states the selected target");

        assert!(error.to_string().contains("target"), "{error}");
    }

    #[test]
    fn defaults_options_to_an_empty_map() {
        let request: GenerateRequest = serde_json::from_value(serde_json::json!({
            "ir": {},
            "target": "openapi"
        }))
        .expect("options remain optional");

        assert!(request.options.is_empty());
    }
}

#[cfg(test)]
mod incremental_tests {
    use super::*;

    #[test]
    fn request_without_baseline_serializes_as_before() {
        let request = CompileRequest {
            language_id: "elm".into(),
            sources: SourceSet::default(),
            package: CompilePackage {
                name: "local/example".into(),
                exposed_modules: None,
            },
            dependencies: vec![],
            options: CompileOptions {
                types_only: true,
                ir_version: "3".into(),
                extra: Default::default(),
            },
            baseline: None,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert!(json.get("baseline").is_none());
        let back: CompileRequest = serde_json::from_value(json).unwrap();
        assert_eq!(back, request);
    }

    #[test]
    fn baseline_round_trips() {
        let json = serde_json::json!({
            "modules": [{
                "name": "My.Types", "uri": "file:///My/Types.elm",
                "sourceDigest": "sha256:aa", "interfaceDigest": "sha256:bb",
                "dependsOn": ["My.Other"], "ir": {"types": []}
            }]
        });
        let baseline: CompileBaseline = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(baseline.modules[0].depends_on, vec!["My.Other".to_string()]);
        assert_eq!(serde_json::to_value(&baseline).unwrap(), json);
    }

    /// `contextDigest` is optional in both directions: a baseline written
    /// before the field existed still decodes, and one that carries it comes
    /// back out byte-identical.
    #[test]
    fn baseline_context_digest_is_optional_and_round_trips() {
        let without = serde_json::json!({"modules": []});
        let baseline: CompileBaseline = serde_json::from_value(without.clone()).unwrap();
        assert_eq!(baseline.context_digest, None);
        assert_eq!(serde_json::to_value(&baseline).unwrap(), without);

        let with = serde_json::json!({"modules": [], "contextDigest": "sha256:cc"});
        let baseline: CompileBaseline = serde_json::from_value(with.clone()).unwrap();
        assert_eq!(baseline.context_digest.as_deref(), Some("sha256:cc"));
        assert_eq!(serde_json::to_value(&baseline).unwrap(), with);
    }

    /// A result's `contextDigest` is what a host stores and echoes back: it is
    /// omitted when the frontend computes none, and survives a round trip.
    #[test]
    fn result_context_digest_is_optional_and_round_trips() {
        let json = serde_json::json!({
            "success": true,
            "diagnostics": [],
            "modules": [],
            "contextDigest": "sha256:dd"
        });
        let result: CompileResult = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(result.context_digest.as_deref(), Some("sha256:dd"));
        assert_eq!(serde_json::to_value(&result).unwrap(), json);

        let without: CompileResult =
            serde_json::from_value(serde_json::json!({"success": true})).unwrap();
        assert_eq!(without.context_digest, None);
        assert!(
            serde_json::to_value(&without)
                .unwrap()
                .get("contextDigest")
                .is_none()
        );
    }

    #[test]
    fn module_results_are_omitted_when_empty_and_statuses_are_lowercase() {
        let result = CompileResult {
            success: true,
            ir_version: Some("3".into()),
            ir: None,
            diagnostics: vec![],
            modules: vec![],
            module_results: vec![],
            context_digest: None,
        };
        assert!(
            serde_json::to_value(&result)
                .unwrap()
                .get("moduleResults")
                .is_none()
        );
        let with = CompileResult {
            module_results: vec![ModuleResult {
                name: "A".into(),
                uri: "file:///A.elm".into(),
                status: ModuleStatus::Unchanged,
                source_digest: Some("sha256:aa".into()),
                interface_digest: None,
                depends_on: vec![],
                ir: None,
                diagnostics: vec![],
            }],
            ..result
        };
        let json = serde_json::to_value(&with).unwrap();
        assert_eq!(json["moduleResults"][0]["status"], "unchanged");
        assert!(json["moduleResults"][0].get("ir").is_none());
        assert!(json["moduleResults"][0].get("interfaceDigest").is_none());
    }

    #[test]
    fn old_shape_result_still_decodes() {
        let json = serde_json::json!({"success": false, "diagnostics": [], "modules": []});
        let result: CompileResult = serde_json::from_value(json).unwrap();
        assert!(result.module_results.is_empty());
    }
}

/// The transitional legacy compile envelope: top-level `documents` with the
/// root in the options bag, as hosts released before [`CompileRequest::sources`]
/// send it.
///
/// This whole module is deleted together with [`SourceEnvelope::Legacy`] once a
/// released host speaks `sources`. Until then these tests pin the property the
/// design turns on: a request is wholly legacy or wholly modern, and the two
/// envelopes can never be mixed into one request that names its root twice.
#[cfg(test)]
mod legacy_compile_envelope_tests {
    use super::*;

    fn a_document() -> serde_json::Value {
        serde_json::json!({
            "uri": "file:///project/src/domain/models.py",
            "languageId": "python",
            "version": 1,
            "text": "",
        })
    }

    /// The legacy envelope is a different *spelling* of the same request, not
    /// a different request: both parse to one identical `CompileRequest`.
    #[test]
    fn a_legacy_request_parses_to_exactly_the_modern_request() {
        let legacy: CompileRequest = serde_json::from_value(serde_json::json!({
            "languageId": "python",
            "documents": [a_document()],
            "package": {"name": "acme/example"},
            "options": {
                "typesOnly": false,
                "irVersion": "4",
                "sourceRootUri": "file:///project/src",
                "outputDir": "compiled",
            },
        }))
        .expect("a host released before `sources` keeps working");

        let modern: CompileRequest = serde_json::from_value(serde_json::json!({
            "languageId": "python",
            "sources": {
                "root": "file:///project/src",
                "documents": [a_document()],
            },
            "package": {"name": "acme/example"},
            "options": {
                "typesOnly": false,
                "irVersion": "4",
                "outputDir": "compiled",
            },
        }))
        .expect("the current envelope parses");

        assert_eq!(legacy, modern);
    }

    /// Normalization *moves* the root: no frontend, option allowlist or
    /// serializer downstream ever sees a legacy key, so none of them needs to
    /// know the legacy envelope exists. The normalized request also
    /// re-serializes, which it could not if the key had been left in `extra` —
    /// `CompileOptions`' serializer still refuses one there.
    #[test]
    fn the_legacy_root_key_does_not_survive_normalization() {
        for key in LEGACY_SOURCE_ROOT_KEYS {
            let request: CompileRequest = serde_json::from_value(serde_json::json!({
                "languageId": "python",
                "documents": [a_document()],
                "package": {"name": "acme/example"},
                "options": {"typesOnly": false, "irVersion": "4", key: "file:///project/src"},
            }))
            .unwrap_or_else(|error| panic!("'{key}' names the legacy root: {error}"));

            assert_eq!(request.sources.root.as_deref(), Some("file:///project/src"));
            assert!(request.options.extra.is_empty(), "'{key}' stayed in extra");
            assert_eq!(
                serde_json::to_value(&request).unwrap()["sources"]["root"],
                serde_json::json!("file:///project/src"),
            );
        }
    }

    /// A legacy host that sends no root at all is a rootless compilation, not
    /// an error: single-document compiles never needed one.
    #[test]
    fn a_legacy_request_without_a_root_has_no_root() {
        let request: CompileRequest = serde_json::from_value(serde_json::json!({
            "languageId": "python",
            "documents": [a_document()],
            "package": {"name": "acme/example"},
            "options": {"typesOnly": false, "irVersion": "4"},
        }))
        .expect("the legacy root key was always optional");

        assert_eq!(request.sources.root, None);
        assert_eq!(request.sources.documents.len(), 1);
    }

    /// Mixing the envelopes is the failure this design exists to prevent: two
    /// source roots, no honest answer for which one module names resolve
    /// against. It is refused rather than settled by a precedence rule, and
    /// the error says why.
    #[test]
    fn a_request_stating_both_envelopes_is_rejected() {
        let error = serde_json::from_value::<CompileRequest>(serde_json::json!({
            "languageId": "python",
            "sources": {"root": "file:///project/src", "documents": []},
            "documents": [a_document()],
            "package": {"name": "acme/example"},
            "options": {"typesOnly": false, "irVersion": "4"},
        }))
        .expect_err("a request is wholly legacy or wholly modern");

        let error = error.to_string();
        assert!(error.contains("both 'sources'"), "{error}");
        assert!(error.contains("documents"), "{error}");
        assert!(error.contains("ambiguous"), "{error}");
    }

    /// Accepting the legacy envelope did not make the legacy key legal in a
    /// *modern* request; that rejection is unchanged, message and all.
    #[test]
    fn a_modern_request_still_rejects_a_legacy_root_key() {
        for key in LEGACY_SOURCE_ROOT_KEYS {
            let error = serde_json::from_value::<CompileRequest>(serde_json::json!({
                "languageId": "python",
                "sources": {"root": "file:///project/src", "documents": []},
                "package": {"name": "acme/example"},
                "options": {"typesOnly": false, "irVersion": "4", key: "file:///elsewhere"},
            }))
            .expect_err("a modern request names its root once, in sources.root")
            .to_string();

            assert!(
                error.contains(&format!(
                    "'{key}' is no longer a compile option; supply the root as sources.root"
                )),
                "{error}"
            );
        }
    }

    /// A request that states no sources at all fails the way it always did,
    /// naming the field a caller is expected to send.
    #[test]
    fn a_request_with_neither_envelope_reports_the_missing_field() {
        let error = serde_json::from_value::<CompileRequest>(serde_json::json!({
            "languageId": "python",
            "package": {"name": "acme/example"},
            "options": {"typesOnly": false, "irVersion": "4"},
        }))
        .expect_err("sources is required");

        assert!(
            error.to_string().contains("missing field `sources`"),
            "{error}"
        );
    }

    /// Both legacy keys in one request agree or the request is refused: two
    /// roots that disagree are the same ambiguity as two envelopes.
    #[test]
    fn two_legacy_root_keys_must_agree() {
        let request: CompileRequest = serde_json::from_value(serde_json::json!({
            "languageId": "gleam",
            "documents": [],
            "package": {"name": "acme/example"},
            "options": {
                "typesOnly": false,
                "irVersion": "4",
                "sourceRootUri": "file:///project/src",
                "sourceRoot": "file:///project/src",
            },
        }))
        .expect("keys that say the same thing say one thing");
        assert_eq!(request.sources.root.as_deref(), Some("file:///project/src"));

        let error = serde_json::from_value::<CompileRequest>(serde_json::json!({
            "languageId": "gleam",
            "documents": [],
            "package": {"name": "acme/example"},
            "options": {
                "typesOnly": false,
                "irVersion": "4",
                "sourceRootUri": "file:///project/src",
                "sourceRoot": "file:///project/lib",
            },
        }))
        .expect_err("disagreeing roots have no resolution");
        assert!(
            error.to_string().contains("different source roots"),
            "{error}"
        );
    }

    /// `null` reads as "no root", the way `sources.root` does. Anything else
    /// is a type error rather than a root nobody can use.
    #[test]
    fn a_legacy_root_key_must_be_a_string_or_null() {
        let request: CompileRequest = serde_json::from_value(serde_json::json!({
            "languageId": "python",
            "documents": [],
            "package": {"name": "acme/example"},
            "options": {"typesOnly": false, "irVersion": "4", "sourceRootUri": null},
        }))
        .expect("an absent option may be spelled null");
        assert_eq!(request.sources.root, None);

        let error = serde_json::from_value::<CompileRequest>(serde_json::json!({
            "languageId": "python",
            "documents": [],
            "package": {"name": "acme/example"},
            "options": {"typesOnly": false, "irVersion": "4", "sourceRootUri": ["src"]},
        }))
        .expect_err("a root is a single location");
        assert!(
            error
                .to_string()
                .contains("'sourceRootUri' must be a string"),
            "{error}"
        );
    }

    /// The legacy envelope carries everything else a request carries; only
    /// where the documents and the root sit differs.
    #[test]
    fn a_legacy_request_keeps_its_dependencies_and_baseline() {
        let request: CompileRequest = serde_json::from_value(serde_json::json!({
            "languageId": "python",
            "documents": [a_document()],
            "package": {"name": "acme/example", "exposedModules": ["domain.models"]},
            "dependencies": [{
                "packageName": "morphir/sdk",
                "irVersion": "4",
                "distribution": {"modules": {}},
            }],
            "options": {"typesOnly": true, "irVersion": "4", "sourceRootUri": "file:///project/src"},
            "baseline": {"modules": [], "contextDigest": "sha256:aa"},
        }))
        .expect("a legacy request is a whole request");

        assert_eq!(request.dependencies.len(), 1);
        assert_eq!(
            request
                .baseline
                .expect("baseline")
                .context_digest
                .as_deref(),
            Some("sha256:aa")
        );
        assert!(request.options.types_only);
        assert_eq!(
            request.package.exposed_modules.as_deref(),
            Some(&["domain.models".to_string()][..])
        );
    }
}
