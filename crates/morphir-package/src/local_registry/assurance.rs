//! Fresh-host restore assurance preflight. Qualification is a supplied attestation,
//! not verification of a filesystem provider. Parsed receipts are data, not authority.
use serde::{Deserialize, Deserializer, Serialize, de::Error};
use serde_json::Value;
use std::future::Future;

/// The assurance profile, separate from the package contract version.
pub const RESTORE_ASSURANCE_PROFILE: &str = "restore-filesystem-assurance";
/// Experimental assurance wire version.
pub const RESTORE_ASSURANCE_PROFILE_VERSION: &str = "0.1.0-draft.1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Profile {
    #[serde(rename = "restore-filesystem-assurance")]
    RestoreFilesystemAssurance,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Version {
    #[serde(rename = "0.1.0-draft.1")]
    Draft1,
}
/// An explicitly requested assurance mode; neither implies the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestoreAssuranceMode {
    /// Portable profile.
    Portable,
    /// Separately qualified hardened profile.
    Hardened,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct Text(String);
impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = String::deserialize(d)?;
        if value.is_empty() {
            Err(D::Error::custom("expected nonempty string"))
        } else {
            Ok(Self(value))
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct Nonempty<T>(Vec<T>);
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Nonempty<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let values = Vec::<T>::deserialize(d)?;
        if values.is_empty() {
            Err(D::Error::custom("expected nonempty list"))
        } else {
            Ok(Self(values))
        }
    }
}
/// Validated explicit profile/version/mode selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RestoreAssuranceSelection {
    profile: Profile,
    profile_version: Version,
    mode: RestoreAssuranceMode,
}
impl RestoreAssuranceSelection {
    /// The explicitly requested mode.
    pub fn mode(&self) -> RestoreAssuranceMode {
        self.mode
    }
}
/// Identity supplied by the trusted host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreAssuranceProviderIdentity {
    id: Text,
    version: Text,
}
impl RestoreAssuranceProviderIdentity {
    /// Host provider name.
    pub fn id(&self) -> &str {
        &self.id.0
    }
    /// Host provider version.
    pub fn version(&self) -> &str {
        &self.version.0
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Platform {
    name: Text,
    version: Text,
}
/// Environment described by a host attestation; no platform support is inferred.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreAssuranceEnvironment {
    runtime: Platform,
    os: Platform,
    architecture: Text,
    filesystem: Text,
    assumptions: Nonempty<Text>,
}
/// One mode's qualification claim supplied by the trusted host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreAssuranceQualification {
    mode: RestoreAssuranceMode,
    environment: RestoreAssuranceEnvironment,
    evidence: Nonempty<Text>,
}
impl RestoreAssuranceQualification {
    /// Mode named by this attestation.
    pub fn mode(&self) -> RestoreAssuranceMode {
        self.mode
    }
    /// Immutable environment snapshot.
    pub fn environment(&self) -> &RestoreAssuranceEnvironment {
        &self.environment
    }
    /// Supplied evidence references, not independently verified here.
    pub fn evidence(&self) -> impl ExactSizeIterator<Item = &str> {
        self.evidence.0.iter().map(|t| t.0.as_str())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
struct Qualifications(Nonempty<RestoreAssuranceQualification>);
impl<'de> Deserialize<'de> for Qualifications {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let entries = Nonempty::<RestoreAssuranceQualification>::deserialize(d)?;
        let modes = entries
            .0
            .iter()
            .map(|q| q.mode)
            .collect::<std::collections::BTreeSet<_>>();
        if modes.len() != entries.0.len() {
            Err(D::Error::custom("duplicate restore assurance mode"))
        } else {
            Ok(Self(entries))
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum QualificationState {
    Unqualified {},
    Qualified { entries: Qualifications },
}
/// A closed snapshot of fresh provider identity and qualification claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreAssuranceProvider {
    identity: RestoreAssuranceProviderIdentity,
    qualification: QualificationState,
}
/// Paired, structurally validated preflight inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreAssuranceRequest {
    selection: RestoreAssuranceSelection,
    provider: RestoreAssuranceProvider,
}
impl RestoreAssuranceRequest {
    /// Explicit requested selection.
    pub fn selection(&self) -> &RestoreAssuranceSelection {
        &self.selection
    }
    /// Fresh host attestation.
    pub fn provider(&self) -> &RestoreAssuranceProvider {
        &self.provider
    }
}
/// Matched selection and qualification with private fields preserving mode agreement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ContextWire")]
pub struct RestoreAssuranceContext {
    selection: RestoreAssuranceSelection,
    provider: RestoreAssuranceProviderIdentity,
    qualification: RestoreAssuranceQualification,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextWire {
    selection: RestoreAssuranceSelection,
    provider: RestoreAssuranceProviderIdentity,
    qualification: RestoreAssuranceQualification,
}
impl TryFrom<ContextWire> for RestoreAssuranceContext {
    type Error = &'static str;
    fn try_from(value: ContextWire) -> Result<Self, Self::Error> {
        if value.selection.mode != value.qualification.mode {
            return Err("restore assurance receipt mode mismatch");
        }
        Ok(Self {
            selection: value.selection,
            provider: value.provider,
            qualification: value.qualification,
        })
    }
}
impl RestoreAssuranceContext {
    /// Matched, explicitly selected mode.
    pub fn selection(&self) -> &RestoreAssuranceSelection {
        &self.selection
    }
    /// Provider identity captured for this call.
    pub fn provider(&self) -> &RestoreAssuranceProviderIdentity {
        &self.provider
    }
    /// Matching host qualification captured for this call.
    pub fn qualification(&self) -> &RestoreAssuranceQualification {
        &self.qualification
    }
}
/// Why preflight rejected before package access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestoreAssuranceRejection {
    /// The provider has no qualification claim.
    ProviderUnqualified,
    /// The provider has no claim for the explicit requested mode.
    ModeUnavailable,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum SelectedKind {
    #[serde(rename = "selected")]
    Selected,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum RejectedKind {
    #[serde(rename = "rejected")]
    Rejected,
}
/// A selected preflight record. Parsing it does not authorize a callback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectedAssuranceReceipt {
    kind: SelectedKind,
    context: RestoreAssuranceContext,
}
/// A rejected preflight record, which cannot be used as an executed receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RejectedAssuranceReceipt {
    kind: RejectedKind,
    selection: RestoreAssuranceSelection,
    provider: RestoreAssuranceProviderIdentity,
    reason: RestoreAssuranceRejection,
}
/// Serializable preflight data only; neither branch grants future access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RestoreAssuranceReceipt {
    /// Selection succeeded using fresh host inputs.
    Selected(SelectedAssuranceReceipt),
    /// Selection rejected before access.
    Rejected(RejectedAssuranceReceipt),
}
/// A callback result, with an executed receipt distinct from rejection.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum AssuranceExecution<T> {
    /// The callback was invoked once with a matched snapshot.
    Executed {
        /// Selected preflight record.
        receipt: SelectedAssuranceReceipt,
        /// Unmodified callback result.
        value: T,
    },
    /// No callback access occurred.
    Rejected {
        /// Rejection preflight record.
        receipt: RejectedAssuranceReceipt,
    },
}
impl<T> AssuranceExecution<T> {
    /// Obtain the preflight report without interpreting it as authorization.
    pub fn receipt(&self) -> RestoreAssuranceReceipt {
        match self {
            Self::Executed { receipt, .. } => RestoreAssuranceReceipt::Selected(receipt.clone()),
            Self::Rejected { receipt } => RestoreAssuranceReceipt::Rejected(receipt.clone()),
        }
    }
}
/// Structural input errors are separate from unchanged callback errors.
#[derive(Debug)]
pub enum AssuranceExecutionError<E> {
    /// Invalid preflight inputs; callback was not invoked.
    InvalidInput(serde_json::Error),
    /// Original callback error with no retry or fallback.
    Callback(E),
}
impl<E: std::fmt::Display> std::fmt::Display for AssuranceExecutionError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(e) => write!(f, "invalid assurance input: {e}"),
            Self::Callback(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for AssuranceExecutionError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(match self {
            Self::InvalidInput(e) => e,
            Self::Callback(e) => e,
        })
    }
}
/// Parse closed selection data and preserve only the supported prerelease version.
pub fn parse_restore_assurance_selection(
    value: &Value,
) -> Result<RestoreAssuranceSelection, serde_json::Error> {
    serde_json::from_value(value.clone())
}
/// Parse trusted host data without verifying evidence or qualifying any provider.
pub fn parse_restore_assurance_provider(
    value: &Value,
) -> Result<RestoreAssuranceProvider, serde_json::Error> {
    serde_json::from_value(value.clone())
}
/// Parse the closed request envelope.
pub fn parse_restore_assurance_request(
    value: &Value,
) -> Result<RestoreAssuranceRequest, serde_json::Error> {
    serde_json::from_value(value.clone())
}
/// Parse a receipt as data; future calls still require fresh provider inputs.
pub fn parse_restore_assurance_receipt(
    value: &Value,
) -> Result<RestoreAssuranceReceipt, serde_json::Error> {
    serde_json::from_value(value.clone())
}
fn preflight(
    selection: &Value,
    provider: &Value,
) -> Result<RestoreAssuranceReceipt, serde_json::Error> {
    let selection = parse_restore_assurance_selection(selection)?;
    let provider = parse_restore_assurance_provider(provider)?;
    let reason = match provider.qualification {
        QualificationState::Unqualified {} => RestoreAssuranceRejection::ProviderUnqualified,
        QualificationState::Qualified { entries } => {
            if let Some(qualification) = entries
                .0
                .0
                .into_iter()
                .find(|entry| entry.mode == selection.mode)
            {
                return Ok(RestoreAssuranceReceipt::Selected(
                    SelectedAssuranceReceipt {
                        kind: SelectedKind::Selected,
                        context: RestoreAssuranceContext {
                            selection,
                            provider: provider.identity,
                            qualification,
                        },
                    },
                ));
            }
            RestoreAssuranceRejection::ModeUnavailable
        }
    };
    Ok(RestoreAssuranceReceipt::Rejected(
        RejectedAssuranceReceipt {
            kind: RejectedKind::Rejected,
            selection,
            provider: provider.identity,
            reason,
        },
    ))
}
/// Select fresh inputs, reject before access or invoke the callback exactly once.
///
/// ```
/// use morphir_package::local_registry::assurance::with_restore_assurance;
/// use serde_json::json;
/// let selected=json!({"profile":"restore-filesystem-assurance","profileVersion":"0.1.0-draft.1","mode":"portable"});
/// let host=json!({"identity":{"id":"host","version":"1"},"qualification":{"kind":"unqualified"}});
/// let result=with_restore_assurance(&selected,&host, |_| -> Result<(),()> {panic!("no access")}).unwrap();
/// assert_eq!(serde_json::to_value(result).unwrap()["kind"], "rejected");
/// ```
pub fn with_restore_assurance<T, E>(
    selection: &Value,
    provider: &Value,
    callback: impl FnOnce(&RestoreAssuranceContext) -> Result<T, E>,
) -> Result<AssuranceExecution<T>, AssuranceExecutionError<E>> {
    match preflight(selection, provider).map_err(AssuranceExecutionError::InvalidInput)? {
        RestoreAssuranceReceipt::Rejected(receipt) => Ok(AssuranceExecution::Rejected { receipt }),
        RestoreAssuranceReceipt::Selected(receipt) => {
            let value = callback(&receipt.context).map_err(AssuranceExecutionError::Callback)?;
            Ok(AssuranceExecution::Executed { receipt, value })
        }
    }
}
/// Snapshot inputs immediately, then run an asynchronous callback once when polled.
/// The future owns its preflight data; caller mutation cannot change the selection.
/// Errors propagate without retry, downgrade or replacement.
pub fn with_restore_assurance_async<
    T,
    E,
    Fut: Future<Output = Result<T, E>>,
    F: FnOnce(RestoreAssuranceContext) -> Fut,
>(
    selection: &Value,
    provider: &Value,
    callback: F,
) -> impl Future<Output = Result<AssuranceExecution<T>, AssuranceExecutionError<E>>> + use<T, E, Fut, F>
{
    let receipt = preflight(selection, provider);
    async move {
        match receipt.map_err(AssuranceExecutionError::InvalidInput)? {
            RestoreAssuranceReceipt::Rejected(receipt) => {
                Ok(AssuranceExecution::Rejected { receipt })
            }
            RestoreAssuranceReceipt::Selected(receipt) => {
                let value = callback(receipt.context.clone())
                    .await
                    .map_err(AssuranceExecutionError::Callback)?;
                Ok(AssuranceExecution::Executed { receipt, value })
            }
        }
    }
}
