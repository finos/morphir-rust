use crate::ir::{Diagnostic, DiagnosticCode, DiagnosticError};
use crate::naming::{name::Name, path::Path};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// FQName represents a Fully Qualified Name (PackagePath + ModulePath + LocalName).
///
/// The wire form is the canonical string `package/path:module/path#local-name`; the reader also
/// accepts the legacy three-element array `[package, module, local]`, where the two paths are
/// legacy arrays of legacy names. `schemars` is told the schema is a string because that is what
/// the writer emits and what every schema consumer expects.
#[derive(Debug, Clone, PartialEq, Eq, Hash, JsonSchema)]
#[schemars(with = "String")]
pub struct FQName {
    pub package_path: Path,
    pub module_path: Path,
    pub local_name: Name,
}

impl FQName {
    pub fn new(package_path: Path, module_path: Path, local_name: Name) -> Self {
        Self {
            package_path,
            module_path,
            local_name,
        }
    }

    /// Parse FQName from classic format: `pkg:mod:local`
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() != 3 {
            return None;
        }
        let pkg_params = parts[0];
        let mod_params = parts[1];
        let local_name = parts[2];
        // The empty string does not name anything, so `a:b:` is not a fully qualified name.
        if local_name.is_empty() {
            return None;
        }

        Some(Self::new(
            Path::new(pkg_params),
            Path::new(mod_params),
            Name::from(local_name),
        ))
    }

    /// Convert to V4 canonical string format: `package/path:module/path#local-name`
    pub fn to_canonical_string(&self) -> String {
        format!(
            "{}:{}#{}",
            self.package_path, self.module_path, self.local_name
        )
    }

    /// Parse from V4 canonical string format: `package/path:module/path#local-name`
    pub fn from_canonical_string(s: &str) -> Result<Self, String> {
        // Split on ':' first, then '#' for the local name
        let colon_pos = s
            .find(':')
            .ok_or_else(|| format!("missing ':' in FQName: {}", s))?;
        let package_str = &s[..colon_pos];
        let rest = &s[colon_pos + 1..];

        let hash_pos = rest
            .find('#')
            .ok_or_else(|| format!("missing '#' in FQName: {}", s))?;
        let module_str = &rest[..hash_pos];
        let local_str = &rest[hash_pos + 1..];

        Ok(Self::new(
            Path::from_canonical_string(package_str)?,
            Path::from_canonical_string(module_str)?,
            Name::from_canonical_string(local_str)?,
        ))
    }
}

impl std::fmt::Display for FQName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_canonical_string())
    }
}

impl From<FQName> for String {
    fn from(fqname: FQName) -> String {
        fqname.to_canonical_string()
    }
}

impl TryFrom<String> for FQName {
    type Error = String;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        FQName::from_canonical_string(&s)
    }
}

impl Serialize for FQName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_canonical_string())
    }
}

/// Reads a fully qualified name from the canonical string or the legacy three-element array.
///
/// This is written out rather than derived through `#[serde(try_from = "String")]` for two
/// reasons. A derived `try_from` only ever sees a string, so the legacy array
/// `[[["morphir"], ["s","d","k"]], [["list"]], ["map"]]` — the spelling every classic document
/// uses — could not be read at all. And its error is a bare `String`, so the code and the cursor
/// are lost by the time serde hands it back; the refusals below carry a [`Diagnostic`] through
/// [`DiagnosticError`] instead.
impl<'de> Deserialize<'de> for FQName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de;

        fn carry<E: de::Error>(code: DiagnosticCode, message: impl Into<String>) -> E {
            E::custom(DiagnosticError(Diagnostic::normalization(
                code, "/", message,
            )))
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        match value {
            serde_json::Value::String(text) => FQName::from_canonical_string(&text)
                .map_err(|error| carry(DiagnosticCode::InvalidFqname, error)),
            serde_json::Value::Array(items) => {
                let [package, module, local]: [serde_json::Value; 3] =
                    items.try_into().map_err(|items: Vec<_>| {
                        carry::<D::Error>(
                            DiagnosticCode::InvalidFqname,
                            format!(
                                "a legacy fully qualified name is a package, a module and a local \
                                 name, not {} elements",
                                items.len()
                            ),
                        )
                    })?;
                Ok(FQName::new(
                    serde_json::from_value(package).map_err(de::Error::custom)?,
                    serde_json::from_value(module).map_err(de::Error::custom)?,
                    serde_json::from_value(local).map_err(de::Error::custom)?,
                ))
            }
            _ => Err(carry(
                DiagnosticCode::InvalidType,
                "a fully qualified name is a canonical string or a legacy three-element array",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fqname_parsing() {
        let fq = FQName::parse("org/pkg:mod/sub:Func").unwrap();
        assert_eq!(fq.package_path.to_string(), "org/pkg");
        assert_eq!(fq.module_path.to_string(), "mod/sub");
        assert_eq!(fq.local_name.to_kebab_case(), "func");
    }

    #[test]
    fn test_fqname_roundtrip() {
        let fq = FQName::parse("my/pkg:my/mod:myFunc").unwrap();
        let s = fq.to_string();
        assert_eq!(s, "my/pkg:my/mod#my-func");
    }

    /// The legacy nested array is the spelling every classic document uses for a fully qualified
    /// name, so the reader has to understand it. A derived `try_from = "String"` only ever sees a
    /// string and refuses this outright.
    #[test]
    fn a_legacy_nested_array_reads_as_a_fully_qualified_name() {
        let fq: FQName = serde_json::from_str(
            r#"[[["acme"], ["b", "i"]], [["widget", "kit"]], ["make", "one"]]"#,
        )
        .expect("a legacy fully qualified name");
        assert_eq!(fq.to_canonical_string(), "acme/BI:widget-kit#make-one");
    }

    #[test]
    fn the_canonical_string_round_trips_through_serde() {
        let text = "\"acme/BI:widget-kit#make-one\"";
        let fq: FQName = serde_json::from_str(text).expect("a canonical fully qualified name");
        assert_eq!(serde_json::to_string(&fq).unwrap(), text);
    }

    /// A refusal has to reach the caller as a code and a cursor, not as prose: reporting through
    /// `serde::de::Error::custom(String)` leaves the caller nothing to answer with but
    /// `invalid_type`.
    #[test]
    fn a_string_that_is_not_a_fully_qualified_name_carries_a_diagnostic() {
        let error = serde_json::from_str::<FQName>("\"acme/BI\"").unwrap_err();
        let diagnostic = Diagnostic::from_serde_error(&error).expect("a carried diagnostic");
        assert_eq!(diagnostic.code, DiagnosticCode::InvalidFqname);
        assert_eq!(diagnostic.cursor, "/");
    }

    #[test]
    fn an_array_of_the_wrong_length_carries_a_diagnostic() {
        let error = serde_json::from_str::<FQName>(r#"[[["acme"]], [["widget"]]]"#).unwrap_err();
        let diagnostic = Diagnostic::from_serde_error(&error).expect("a carried diagnostic");
        assert_eq!(diagnostic.code, DiagnosticCode::InvalidFqname);
    }
}
