//! Exact draft compatibility at the claim set read boundary.

use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaimsWire {
    protocol_versions: Vec<String>,
    extension: ExtensionInfo,
    capabilities: Map<String, Value>,
    #[serde(default)]
    requires: Option<ClaimsRequirements>,
    #[serde(default, deserialize_with = "read_critical")]
    critical: Vec<String>,
}

impl TryFrom<Value> for CapabilityClaimSet {
    type Error = String;

    fn try_from(mut value: Value) -> Result<Self, Self::Error> {
        let object = value
            .as_object_mut()
            .ok_or("expected capability claim set")?;
        let old = object.contains_key("statementVersion");
        if old && object.contains_key("claimsVersion") {
            return Err("mixed capability claim set version members".into());
        }
        let member = if old {
            "statementVersion"
        } else {
            "claimsVersion"
        };
        let version: Version =
            serde_json::from_value(object.remove(member).ok_or("missing claimsVersion")?)
                .map_err(|error| format!("invalid claimsVersion: {error}"))?;
        let drafts = ["=0.1.0-draft.1", "=0.1.0-draft.2"];
        let draft = drafts.iter().position(|requirement| {
            VersionReq::parse(requirement).expect("exact claims draft requirement").matches(&version)
        }).ok_or_else(|| format!("unsupported claimsVersion '{version}'; exact drafts: 0.1.0-draft.1, 0.1.0-draft.2"))?;
        if old != (draft == 0) {
            return Err(format!(
                "claimsVersion '{version}' does not match capability claim set members"
            ));
        }
        if old && let Some(Value::Array(paths)) = object.get_mut("critical") {
            for path in paths {
                if path.as_str() == Some("claimsVersion") {
                    return Err(
                        "unknown critical member 'claimsVersion' in draft.1 claim set".into(),
                    );
                }
                if path.as_str() == Some("statementVersion") {
                    *path = Value::String("claimsVersion".into());
                }
            }
        }
        let wire: ClaimsWire = serde_json::from_value(value).map_err(|error| error.to_string())?;
        Ok(Self {
            claims_version: Version::parse(CLAIMS_VERSION).expect("claims version is SemVer"),
            protocol_versions: wire.protocol_versions,
            extension: wire.extension,
            capabilities: wire.capabilities,
            requires: wire.requires,
            critical: wire.critical,
        })
    }
}
