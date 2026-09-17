use super::input::*;
use super::values::*;
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

/// A JSON Pointer and the validation rule violated there.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Violation {
    pub(crate) pointer: String,
    pub(crate) rule: ViolationRule,
}
impl Violation {
    /// The RFC 6901 pointer to the rejected value.
    pub fn pointer(&self) -> &str {
        &self.pointer
    }
    /// The rule that the value violated.
    pub fn rule(&self) -> ViolationRule {
        self.rule
    }
}

/// A stable validation rule name from the resolution contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ViolationRule {
    MalformedJson,
    DuplicateKey,
    UnknownField,
    MissingField,
    InvalidType,
    InvalidValue,
    InvalidName,
    InvalidVersion,
    InvalidInterval,
    InvalidDigest,
    DuplicateIdentity,
    IdentityMismatch,
    MissingRoot,
    DanglingBinding,
    BindingMismatch,
    RequirementMismatch,
    DigestMismatch,
    UnreachableNode,
    Cycle,
    UnsupportedFlatBinding,
}
impl ViolationRule {
    /// The contract spelling used in JSON diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MalformedJson => "malformed-json",
            Self::DuplicateKey => "duplicate-key",
            Self::UnknownField => "unknown-field",
            Self::MissingField => "missing-field",
            Self::InvalidType => "invalid-type",
            Self::InvalidValue => "invalid-value",
            Self::InvalidName => "invalid-name",
            Self::InvalidVersion => "invalid-version",
            Self::InvalidInterval => "invalid-interval",
            Self::InvalidDigest => "invalid-digest",
            Self::DuplicateIdentity => "duplicate-identity",
            Self::IdentityMismatch => "identity-mismatch",
            Self::MissingRoot => "missing-root",
            Self::DanglingBinding => "dangling-binding",
            Self::BindingMismatch => "binding-mismatch",
            Self::RequirementMismatch => "requirement-mismatch",
            Self::DigestMismatch => "digest-mismatch",
            Self::UnreachableNode => "unreachable-node",
            Self::Cycle => "cycle",
            Self::UnsupportedFlatBinding => "unsupported-flat-binding",
        }
    }
}

/// One absent item required to establish a complete resolution input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MissingItem {
    /// No catalog was supplied for a reachable dependency path.
    Catalog {
        #[serde(rename = "packagePath")]
        package_path: PackagePath,
    },
    /// Metadata for an exact release selected by an old lock was absent.
    Release { release: ReleaseId },
}

/// One old out-of-scope pin that a diagnostic search had to relax.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum ChangedPin {
    /// The relaxed search selected a different release at this old path.
    Changed {
        previous: ReleaseId,
        selected: ReleaseId,
    },
    /// The relaxed search no longer needed this old path.
    Removed { previous: ReleaseId },
}

/// One dependency edge in an unfolded diagnostic witness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WitnessBinding {
    pub(crate) ir_package_name: IrPackageName,
    pub(crate) target_occurrence: Vec<IrPackageName>,
}
impl WitnessBinding {
    /// The consumer dependency IR name.
    pub fn ir_package_name(&self) -> &IrPackageName {
        &self.ir_package_name
    }

    /// The occurrence path of the selected child.
    pub fn target_occurrence(&self) -> &[IrPackageName] {
        &self.target_occurrence
    }
}
/// One occurrence in an unfolded diagnostic witness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WitnessNode {
    pub(crate) occurrence: Vec<IrPackageName>,
    pub(crate) release: ReleaseId,
    pub(crate) bindings: Vec<WitnessBinding>,
}
impl WitnessNode {
    /// The root-to-node dependency-name path.
    pub fn occurrence(&self) -> &[IrPackageName] {
        &self.occurrence
    }

    /// The release selected for this occurrence.
    pub fn release(&self) -> &ReleaseId {
        &self.release
    }

    /// The node's dependency edges.
    pub fn bindings(&self) -> &[WitnessBinding] {
        &self.bindings
    }
}
/// A fully unfolded consumer-scoped graph used only as diagnostic evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Witness {
    pub(crate) nodes: Vec<WitnessNode>,
}
impl Witness {
    /// Occurrences in canonical path order.
    pub fn nodes(&self) -> &[WitnessNode] {
        &self.nodes
    }
}

/// A resolver feature required to represent an otherwise feasible witness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RequiredCapability {
    /// Different consumers need distinct versions of one IR package name.
    GraphAwareCoexistence,
}

/// A domain rejection with stable machine-readable evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "kebab-case")]
pub enum ResolutionDiagnostic {
    /// The request failed one of the input-validation phases.
    InvalidInput { violations: Vec<Violation> },
    /// The replayed or update baseline lock failed lock validation.
    InvalidLock { violations: Vec<Violation> },
    /// The supplied metadata snapshot was not complete enough to resolve.
    IncompleteInput { missing: Vec<MissingItem> },
    /// Resolution succeeds only after relaxing an out-of-scope old pin.
    UpdateScopeConflict {
        #[serde(rename = "changedPins")]
        changed_pins: Vec<ChangedPin>,
        witness: Witness,
    },
    /// A feasible graph requires a capability outside `flat-library`.
    UnsupportedCapability {
        #[serde(rename = "requiredCapabilities")]
        required_capabilities: Vec<RequiredCapability>,
        #[serde(rename = "changedPins")]
        changed_pins: Vec<ChangedPin>,
        witness: Witness,
    },
    /// No graph satisfies the canonical reachable requirement problem.
    UnsatisfiableRequirements {
        root: ReleaseRecord,
        catalogs: Vec<Catalog>,
        targets: Vec<UpdateTarget>,
    },
}

/// The outcome of a completed resolution operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionResult {
    /// Resolution produced a complete supported graph.
    Resolved(LockedGraph),
    /// The operation completed and rejected the domain input.
    Rejected(ResolutionDiagnostic),
}
impl Serialize for ResolutionResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        match self {
            Self::Resolved(graph) => {
                map.serialize_entry("ok", &true)?;
                map.serialize_entry("graph", graph)?;
            }
            Self::Rejected(diagnostic) => {
                map.serialize_entry("ok", &false)?;
                map.serialize_entry("diagnostic", diagnostic)?;
            }
        }
        map.end()
    }
}

/// A failure to execute resolution, distinct from a domain rejection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("resolution could not be executed: {message}")]
pub struct ResolutionExecutionError {
    message: String,
}
impl ResolutionExecutionError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// A stable human-readable explanation of the execution failure.
    pub fn message(&self) -> &str {
        &self.message
    }
}
