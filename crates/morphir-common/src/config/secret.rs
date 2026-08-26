//! References to secrets stored outside a Morphir configuration file.

use serde_json::Value;
use std::path::PathBuf;
use thiserror::Error;

/// An external source for a secret value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretReference {
    /// A secret read from an environment variable.
    Environment { variable: String },
    /// A secret read from a file.
    File { path: PathBuf },
    /// A secret supplied by an external command.
    Command { program: String, args: Vec<String> },
    /// A secret stored in an operating system keyring.
    Keyring { service: String, account: String },
}

/// The structural reason a JSON value cannot represent a [`SecretReference`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SecretReferenceError {
    /// The reference is not an object.
    #[error("secret reference must be an object")]
    ExpectedObject,
    /// The reference object did not have exactly one discriminator.
    #[error("secret reference must contain exactly one discriminator")]
    ExpectedOneDiscriminator,
    /// The reference discriminator is unknown.
    #[error("secret reference discriminator is not supported")]
    UnsupportedDiscriminator,
    /// A required string field was missing, incorrectly typed, or empty.
    #[error("secret reference {field} must be a non-empty string")]
    ExpectedNonEmptyString {
        /// The field with the invalid value.
        field: &'static str,
    },
    /// The command was not a non-empty array of non-empty strings.
    #[error("secret reference command must be a non-empty array of non-empty strings")]
    ExpectedCommand,
    /// The keyring value was not an object with exactly service and account.
    #[error("secret reference keyring must contain exactly service and account")]
    ExpectedKeyring,
}

impl TryFrom<&Value> for SecretReference {
    type Error = SecretReferenceError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        let reference = value
            .as_object()
            .ok_or(SecretReferenceError::ExpectedObject)?;

        if reference.len() != 1 {
            return Err(SecretReferenceError::ExpectedOneDiscriminator);
        }

        let (kind, value) = reference
            .iter()
            .next()
            .ok_or(SecretReferenceError::ExpectedOneDiscriminator)?;

        match kind.as_str() {
            "env" => Ok(Self::Environment {
                variable: required_string(value, "env")?,
            }),
            "file" => Ok(Self::File {
                path: required_string(value, "file")?.into(),
            }),
            "command" => parse_command(value),
            "keyring" => parse_keyring(value),
            _ => Err(SecretReferenceError::UnsupportedDiscriminator),
        }
    }
}

/// Return whether a JSON value is an exact secret-reference shape.
pub fn is_secret_reference(value: &Value) -> bool {
    SecretReference::try_from(value).is_ok()
}

fn required_string(value: &Value, field: &'static str) -> Result<String, SecretReferenceError> {
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or(SecretReferenceError::ExpectedNonEmptyString { field })
}

fn parse_command(value: &Value) -> Result<SecretReference, SecretReferenceError> {
    let command = value
        .as_array()
        .filter(|command| !command.is_empty())
        .ok_or(SecretReferenceError::ExpectedCommand)?;
    let mut parts = command
        .iter()
        .map(|part| required_string(part, "command"))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SecretReferenceError::ExpectedCommand)?;
    let program = parts.remove(0);

    Ok(SecretReference::Command {
        program,
        args: parts,
    })
}

fn parse_keyring(value: &Value) -> Result<SecretReference, SecretReferenceError> {
    let keyring = value
        .as_object()
        .filter(|keyring| keyring.len() == 2)
        .ok_or(SecretReferenceError::ExpectedKeyring)?;
    let service = keyring
        .get("service")
        .ok_or(SecretReferenceError::ExpectedKeyring)
        .and_then(|value| required_string(value, "keyring.service"))?;
    let account = keyring
        .get("account")
        .ok_or(SecretReferenceError::ExpectedKeyring)
        .and_then(|value| required_string(value, "keyring.account"))?;

    Ok(SecretReference::Keyring { service, account })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_all_exact_reference_shapes() {
        assert_eq!(
            SecretReference::try_from(&json!({"env": "GITHUB_TOKEN"})).unwrap(),
            SecretReference::Environment {
                variable: "GITHUB_TOKEN".into()
            }
        );
        assert_eq!(
            SecretReference::try_from(&json!({"file": "secrets/token"})).unwrap(),
            SecretReference::File {
                path: "secrets/token".into()
            }
        );
        assert_eq!(
            SecretReference::try_from(&json!({"command": ["gh", "auth", "token"]})).unwrap(),
            SecretReference::Command {
                program: "gh".into(),
                args: vec!["auth".into(), "token".into()],
            }
        );
        assert_eq!(
            SecretReference::try_from(
                &json!({"keyring": {"service": "github.com", "account": "damre"}})
            )
            .unwrap(),
            SecretReference::Keyring {
                service: "github.com".into(),
                account: "damre".into()
            }
        );
    }

    #[test]
    fn rejects_mixed_extra_empty_and_wrong_typed_reference_shapes() {
        for value in [
            json!({"env": "A", "file": "b"}),
            json!({"command": []}),
            json!({"command": ["gh", 1]}),
            json!({"keyring": {"service": "github.com"}}),
            json!({"keyring": {"service": "", "account": "damre"}}),
            json!({"keyring": {"service": "github.com", "account": "damre", "extra": true}}),
        ] {
            assert!(
                SecretReference::try_from(&value).is_err(),
                "accepted {value}"
            );
            assert!(!is_secret_reference(&value));
        }
    }
}
