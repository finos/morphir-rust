//! Redaction of sensitive values before a configuration value is displayed.
//!
//! The effective configuration can carry credentials, for example from
//! `MORPHIR_REGISTRY_TOKEN` or `MORPHIR_REGISTRY_PASSWORD`. Tools that print
//! the configuration should pass it through [`redact_secrets`] first.

use serde_json::Value;

/// Replacement for a sensitive value.
pub const REDACTED: &str = "<redacted>";

const SENSITIVE_KEY_FRAGMENTS: &[&str] = &[
    "token",
    "password",
    "passwd",
    "secret",
    "credential",
    "api_key",
    "apikey",
    "private_key",
    "access_key",
];

/// Whether a configuration key names a value that must not be displayed.
///
/// Matching is case-insensitive and treats `-` like `_`, so `registry_token`,
/// `RegistryToken`, and `api-key` are all sensitive.
///
/// ```
/// use morphir_common::config::redact::is_sensitive_key;
///
/// assert!(is_sensitive_key("registry_token"));
/// assert!(is_sensitive_key("api-key"));
/// assert!(!is_sensitive_key("authors"));
/// assert!(!is_sensitive_key("username"));
/// ```
pub fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace('-', "_");
    SENSITIVE_KEY_FRAGMENTS
        .iter()
        .any(|fragment| normalized.contains(fragment))
}

/// Return a copy of `value` with every sensitive entry replaced by [`REDACTED`].
///
/// The whole subtree under a sensitive key is replaced, whatever its type.
///
/// ```
/// use morphir_common::config::redact::redact_secrets;
/// use serde_json::json;
///
/// let value = json!({
///     "registry_token": "ghp_abc",
///     "registry": {"password": "hunter2", "username": "alice"},
///     "toolchain": {"elm": {"env": {"NPM_TOKEN": "x"}}}
/// });
///
/// assert_eq!(
///     redact_secrets(&value),
///     json!({
///         "registry_token": "<redacted>",
///         "registry": {"password": "<redacted>", "username": "alice"},
///         "toolchain": {"elm": {"env": {"NPM_TOKEN": "<redacted>"}}}
///     })
/// );
/// ```
pub fn redact_secrets(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let redacted = if is_sensitive_key(key) {
                        Value::String(REDACTED.to_string())
                    } else {
                        redact_secrets(value)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redact_secrets).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn matches_common_credential_key_shapes() {
        for key in [
            "token",
            "MORPHIR_REGISTRY_TOKEN",
            "registry_password",
            "passwd",
            "client-secret",
            "credentials",
            "apiKey",
            "private_key",
            "aws_access_key_id",
        ] {
            assert!(is_sensitive_key(key), "{key} should be sensitive");
        }
        for key in ["username", "authors", "key_bindings", "entry_point"] {
            assert!(!is_sensitive_key(key), "{key} should not be sensitive");
        }
        // Substring matching errs on the side of redaction.
        assert!(is_sensitive_key("tokenizer_mode"));
    }

    #[test]
    fn redacts_nested_values_and_arrays_without_mutating_input() {
        let value = json!({
            "registry": {"token": {"value": "abc", "expires": 1}},
            "list": [{"password": "x"}, {"name": "ok"}],
            "plain": "visible"
        });
        let before = value.clone();

        let redacted = redact_secrets(&value);

        assert_eq!(
            redacted,
            json!({
                "registry": {"token": REDACTED},
                "list": [{"password": REDACTED}, {"name": "ok"}],
                "plain": "visible"
            })
        );
        assert_eq!(value, before);
    }

    #[test]
    fn leaves_scalars_untouched() {
        assert_eq!(redact_secrets(&json!("token")), json!("token"));
        assert_eq!(redact_secrets(&json!(42)), json!(42));
    }
}
