//! Protected, on-demand resolution of secret configuration values.

mod system;

use crate::config::EffectiveConfig;
use serde_json::Value;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub use morphir_common::config::{ExposeSecret, SecretReference, SecretString};
pub use system::SystemSecretResolver;

/// Information about the configuration value being resolved.
#[derive(Debug, Clone, Copy)]
pub struct SecretResolutionContext<'a> {
    /// Dotted configuration key requested by the caller.
    pub config_key: &'a str,
    /// File that supplied the winning value, when the value came from a file.
    pub declaring_file: Option<&'a Path>,
}

/// Resolves an external secret reference into protected text.
pub trait SecretResolver {
    /// Resolve one reference without traversing or resolving any other value.
    fn resolve(
        &self,
        reference: &SecretReference,
        context: SecretResolutionContext<'_>,
    ) -> Result<SecretString, SecretResolutionError>;
}

/// A failure to locate, classify, or resolve one secret value.
#[derive(Debug, Error)]
pub enum SecretResolutionError {
    /// The requested dotted key was empty or contained an empty segment.
    #[error("secret configuration key `{config_key}` is not a valid dotted key")]
    InvalidConfigKey {
        /// Dotted key supplied by the caller.
        config_key: String,
    },
    /// The requested dotted key was not present in the effective configuration.
    #[error("secret configuration key `{config_key}` was not found")]
    MissingConfigKey {
        /// Dotted key supplied by the caller.
        config_key: String,
    },
    /// The selected value was neither non-empty literal text nor an exact reference.
    #[error("secret configuration key `{config_key}` is invalid: {reason}")]
    InvalidSecretValue {
        /// Dotted key supplied by the caller.
        config_key: String,
        /// Static structural classification that never contains supplied values.
        reason: &'static str,
    },
    /// The selected reference kind has no backend in this resolver.
    #[error("secret reference kind `{reference_kind}` is not supported for `{config_key}`")]
    UnsupportedReference {
        /// Dotted configuration key being resolved.
        config_key: String,
        /// Static reference-kind label.
        reference_kind: &'static str,
    },
    /// The referenced environment variable was not present.
    #[error("environment variable `{variable}` is not set")]
    EnvironmentMissing {
        /// Environment variable named by the reference.
        variable: String,
    },
    /// The referenced environment value was not Unicode text.
    #[error("environment variable `{variable}` is not Unicode text")]
    EnvironmentNotUnicode {
        /// Environment variable named by the reference.
        variable: String,
    },
    /// The referenced environment name cannot be passed to the operating system.
    #[error("environment variable name for `{config_key}` is invalid")]
    InvalidEnvironmentName {
        /// Dotted configuration key being resolved.
        config_key: String,
    },
    /// A backend returned no secret text.
    #[error("{backend} secret source returned empty text")]
    EmptySecret {
        /// Static source-kind label.
        backend: &'static str,
    },
    /// A relative file reference had no declaring file to anchor it.
    #[error("relative secret file requires a declaring configuration file")]
    RelativeFileWithoutDeclaringFile {
        /// Relative path named by the reference.
        path: PathBuf,
    },
    /// The current user's home directory could not be determined.
    #[error("home directory is unavailable for secret file expansion")]
    HomeDirectoryUnavailable,
    /// A named-user home path such as `~alice` is unsupported.
    #[error("named-user home paths are not supported for secret files")]
    UnsupportedHomePath {
        /// Unsupported path named by the reference.
        path: PathBuf,
    },
    /// The selected secret file could not be read.
    #[error("secret file `{path}` could not be read ({kind:?})")]
    FileRead {
        /// Resolved file path.
        path: PathBuf,
        /// Stable I/O failure classification, without an arbitrary error string.
        kind: ErrorKind,
    },
    /// The selected secret file did not contain UTF-8 text.
    #[error("secret file `{path}` does not contain UTF-8 text")]
    FileNotUnicode {
        /// Resolved file path.
        path: PathBuf,
    },
    /// The process current directory could not be read for a command reference.
    #[error("current directory is unavailable for secret command `{config_key}` ({kind:?})")]
    CommandCurrentDirectory {
        /// Dotted configuration key being resolved.
        config_key: String,
        /// Stable I/O failure classification.
        kind: ErrorKind,
    },
    /// The referenced command could not be started.
    #[error("secret command `{program}` for `{config_key}` could not be started ({kind:?})")]
    CommandSpawn {
        /// Dotted configuration key being resolved.
        config_key: String,
        /// Executable named by the reference.
        program: String,
        /// Stable I/O failure classification, without an arbitrary error string.
        kind: ErrorKind,
    },
    /// The referenced command exited unsuccessfully.
    #[error("secret command `{program}` for `{config_key}` failed with status {status_code:?}")]
    CommandFailed {
        /// Dotted configuration key being resolved.
        config_key: String,
        /// Executable named by the reference.
        program: String,
        /// Platform exit code, when the platform supplied one.
        status_code: Option<i32>,
    },
    /// The referenced command emitted stdout that was not UTF-8 text.
    #[error("secret command `{program}` for `{config_key}` did not emit UTF-8 text")]
    CommandOutputNotUnicode {
        /// Dotted configuration key being resolved.
        config_key: String,
        /// Executable named by the reference.
        program: String,
    },
    /// The selected native credential could not be read.
    #[error(
        "native keyring credential for service `{service}` and account `{account}` could not be read for `{config_key}`"
    )]
    KeyringLookupFailed {
        /// Dotted configuration key being resolved.
        config_key: String,
        /// Keyring service named by the reference.
        service: String,
        /// Keyring account named by the reference.
        account: String,
    },
}

impl EffectiveConfig {
    /// Resolve one secret value using the operating-system-backed resolver.
    #[allow(clippy::default_constructed_unit_structs)]
    pub fn resolve_secret(&self, key: &str) -> Result<SecretString, SecretResolutionError> {
        self.resolve_secret_with(key, &SystemSecretResolver::default())
    }

    /// Resolve one secret value using an explicitly supplied backend.
    ///
    /// The resolver receives only the requested reference. Literal values do
    /// not call it, and the returned text remains protected until explicitly
    /// exposed at its final use site.
    ///
    /// ```
    /// use morphir_devkit::{
    ///     ConfigLoadOptions, ExposeSecret, SecretReference, SecretResolutionContext,
    ///     SecretResolutionError, SecretResolver, SecretString, load_effective_config,
    /// };
    ///
    /// struct FixtureResolver;
    ///
    /// impl SecretResolver for FixtureResolver {
    ///     fn resolve(
    ///         &self,
    ///         reference: &SecretReference,
    ///         context: SecretResolutionContext<'_>,
    ///     ) -> Result<SecretString, SecretResolutionError> {
    ///         match reference {
    ///             SecretReference::Environment { variable } if variable == "REGISTRY_TOKEN" => {
    ///                 Ok(SecretString::from("injected-token".to_owned()))
    ///             }
    ///             _ => Err(SecretResolutionError::UnsupportedReference {
    ///                 config_key: context.config_key.to_owned(),
    ///                 reference_kind: "fixture",
    ///             }),
    ///         }
    ///     }
    /// }
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let directory = tempfile::tempdir()?;
    /// let config_path = directory.path().join("morphir.toml");
    /// std::fs::write(
    ///     &config_path,
    ///     "[registry]\nliteral = \"literal-token\"\ninjected = { env = \"REGISTRY_TOKEN\" }\n",
    /// )?;
    /// let effective = load_effective_config(
    ///     Some(&config_path),
    ///     &ConfigLoadOptions::project_only(),
    /// )?;
    ///
    /// let literal = effective.resolve_secret("registry.literal")?;
    /// let injected = effective.resolve_secret_with("registry.injected", &FixtureResolver)?;
    /// assert!(
    ///     literal.expose_secret() == "literal-token",
    ///     "literal secret did not match"
    /// );
    /// assert!(
    ///     injected.expose_secret() == "injected-token",
    ///     "injected secret did not match"
    /// );
    /// # Ok(())
    /// # }
    /// ```
    pub fn resolve_secret_with(
        &self,
        key: &str,
        resolver: &dyn SecretResolver,
    ) -> Result<SecretString, SecretResolutionError> {
        let segments = dotted_key_segments(key)?;
        let value = lookup_value(&self.value, &segments).ok_or_else(|| {
            SecretResolutionError::MissingConfigKey {
                config_key: key.to_owned(),
            }
        })?;

        match value {
            Value::String(value) if !value.is_empty() => Ok(SecretString::from(value.to_owned())),
            Value::String(_) => Err(invalid_secret_value(key, "empty string")),
            Value::Object(object) => match SecretReference::try_from(value) {
                Ok(reference) => resolver.resolve(
                    &reference,
                    SecretResolutionContext {
                        config_key: key,
                        declaring_file: self
                            .origin_for_key(key)
                            .and_then(|origin| origin.path.as_deref()),
                    },
                ),
                Err(_) if object.keys().any(|key| is_reference_discriminator(key)) => {
                    Err(invalid_secret_value(key, "malformed secret reference"))
                }
                Err(_) => Err(invalid_secret_value(
                    key,
                    "object is not an exact secret reference",
                )),
            },
            _ => Err(invalid_secret_value(
                key,
                "value is not non-empty text or an exact secret reference",
            )),
        }
    }
}

fn dotted_key_segments(key: &str) -> Result<Vec<&str>, SecretResolutionError> {
    let segments = key.split('.').collect::<Vec<_>>();
    if key.is_empty() || segments.iter().any(|segment| segment.is_empty()) {
        return Err(SecretResolutionError::InvalidConfigKey {
            config_key: key.to_owned(),
        });
    }
    Ok(segments)
}

fn lookup_value<'a>(value: &'a Value, segments: &[&str]) -> Option<&'a Value> {
    segments
        .iter()
        .try_fold(value, |current, segment| current.as_object()?.get(*segment))
}

fn invalid_secret_value(config_key: &str, reason: &'static str) -> SecretResolutionError {
    SecretResolutionError::InvalidSecretValue {
        config_key: config_key.to_owned(),
        reason,
    }
}

fn is_reference_discriminator(key: &str) -> bool {
    matches!(key, "env" | "file" | "command" | "keyring")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::provenance::{ConfigOrigin, ProvenanceState};
    use crate::config::{ConfigSourceKind, EffectiveConfig};
    use morphir_common::config::{ExposeSecret, SecretReference, SecretString};
    use serde_json::{Value, json};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    #[derive(Clone)]
    struct RecordedCall {
        reference: SecretReference,
        config_key: String,
        declaring_file: Option<PathBuf>,
    }

    struct RecordingResolver {
        calls: RefCell<Vec<RecordedCall>>,
        result: Option<&'static str>,
    }

    impl RecordingResolver {
        fn returning(value: &'static str) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                result: Some(value),
            }
        }

        fn rejecting_calls() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                result: None,
            }
        }

        fn calls(&self) -> Vec<RecordedCall> {
            self.calls.borrow().clone()
        }
    }

    impl SecretResolver for RecordingResolver {
        fn resolve(
            &self,
            reference: &SecretReference,
            context: SecretResolutionContext<'_>,
        ) -> Result<SecretString, SecretResolutionError> {
            self.calls.borrow_mut().push(RecordedCall {
                reference: reference.clone(),
                config_key: context.config_key.to_owned(),
                declaring_file: context.declaring_file.map(Path::to_path_buf),
            });

            self.result
                .map(|value| SecretString::from(value.to_owned()))
                .ok_or_else(|| SecretResolutionError::UnsupportedReference {
                    config_key: context.config_key.to_owned(),
                    reference_kind: "test",
                })
        }
    }

    fn an_effective_config(value: Value, declaring_file: &str) -> EffectiveConfig {
        let mut state = ProvenanceState::default();
        state.merge(
            &value,
            ConfigOrigin {
                kind: ConfigSourceKind::UserOverride,
                path: Some(PathBuf::from(declaring_file)),
            },
        );
        let (value, provenance) = state.into_parts();

        EffectiveConfig {
            value,
            sources: Vec::new(),
            workspace_root: None,
            member_root: None,
            ignored_member_out_dir: Vec::new(),
            warnings: Vec::new(),
            provenance,
        }
    }

    fn an_effective_config_with_user_command_reference() -> EffectiveConfig {
        an_effective_config(
            json!({"registry": {"token": {"command": ["gh", "auth", "token"]}}}),
            "/work/morphir.user.toml",
        )
    }

    fn an_effective_config_with_literal(value: &str) -> EffectiveConfig {
        an_effective_config(
            json!({"registry": {"token": value}}),
            "/work/morphir.user.toml",
        )
    }

    #[test]
    fn resolves_only_the_requested_reference_with_its_origin() {
        let effective = an_effective_config_with_user_command_reference();
        let resolver = RecordingResolver::returning("resolved-token");

        let secret = effective
            .resolve_secret_with("registry.token", &resolver)
            .unwrap();

        assert_eq!(secret.expose_secret(), "resolved-token");
        let calls = resolver.calls();
        assert_eq!(calls.len(), 1);
        assert!(
            matches!(
                &calls[0].reference,
                SecretReference::Command { program, args }
                    if program == "gh" && args == &["auth", "token"]
            ),
            "resolver did not receive the expected command structure"
        );
        assert_eq!(calls[0].config_key, "registry.token");
        assert_eq!(
            calls[0].declaring_file,
            Some(PathBuf::from("/work/morphir.user.toml"))
        );
    }

    #[test]
    fn literal_secret_is_protected_without_calling_a_backend() {
        let effective = an_effective_config_with_literal("literal-token");
        let resolver = RecordingResolver::rejecting_calls();

        let secret = effective
            .resolve_secret_with("registry.token", &resolver)
            .unwrap();

        assert_eq!(secret.expose_secret(), "literal-token");
        assert!(!format!("{secret:?}").contains("literal-token"));
        assert!(resolver.calls().is_empty());
    }

    #[test]
    fn missing_path_is_a_typed_error() {
        let effective = an_effective_config_with_literal("literal-token");

        let error = effective
            .resolve_secret_with("registry.missing", &RecordingResolver::rejecting_calls())
            .unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::MissingConfigKey { ref config_key }
                if config_key == "registry.missing"
        ));
    }

    #[test]
    fn empty_literal_is_rejected_without_exposing_the_value() {
        let effective = an_effective_config_with_literal("");

        let error = effective
            .resolve_secret_with("registry.token", &RecordingResolver::rejecting_calls())
            .unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::InvalidSecretValue {
                ref config_key,
                reason: "empty string",
            } if config_key == "registry.token"
        ));
    }

    #[test]
    fn ordinary_object_is_rejected_without_backend_dispatch() {
        let effective = an_effective_config(
            json!({"registry": {"token": {"nested": "value"}}}),
            "/work/morphir.user.toml",
        );
        let resolver = RecordingResolver::rejecting_calls();

        let error = effective
            .resolve_secret_with("registry.token", &resolver)
            .unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::InvalidSecretValue {
                reason: "object is not an exact secret reference",
                ..
            }
        ));
        assert!(resolver.calls().is_empty());
    }

    #[test]
    fn malformed_reference_payload_is_classified_without_rendering_input() {
        let sentinel = "must-not-appear-in-errors";
        let effective = an_effective_config(
            json!({"registry": {"token": {"env": "", "private": sentinel}}}),
            "/work/morphir.user.toml",
        );

        let error = effective
            .resolve_secret_with("registry.token", &RecordingResolver::rejecting_calls())
            .unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::InvalidSecretValue {
                reason: "malformed secret reference",
                ..
            }
        ));
        assert!(!format!("{error}").contains(sentinel));
        assert!(!format!("{error:?}").contains(sentinel));
    }

    #[test]
    fn dotted_key_must_have_only_non_empty_segments() {
        let effective = an_effective_config_with_literal("literal-token");

        for key in ["", ".registry", "registry..token", "registry.token."] {
            assert!(matches!(
                effective.resolve_secret_with(key, &RecordingResolver::rejecting_calls()),
                Err(SecretResolutionError::InvalidConfigKey { .. })
            ));
        }
    }

    #[test]
    fn environment_reference_with_equals_returns_a_redacted_error() {
        let invalid_name = "INVALID=must-not-appear";
        let effective = an_effective_config(
            json!({"registry": {"token": {"env": invalid_name}}}),
            "/work/morphir.user.toml",
        );

        let error = effective.resolve_secret("registry.token").unwrap_err();

        assert_eq!(
            error.to_string(),
            "environment variable name for `registry.token` is invalid"
        );
        assert!(!format!("{error:?}").contains(invalid_name));
    }

    #[test]
    fn environment_reference_with_nul_returns_a_redacted_error() {
        let invalid_name = "INVALID\0must-not-appear";
        let effective = an_effective_config(
            json!({"registry": {"token": {"env": invalid_name}}}),
            "/work/morphir.user.toml",
        );

        let error = effective.resolve_secret("registry.token").unwrap_err();

        assert_eq!(
            error.to_string(),
            "environment variable name for `registry.token` is invalid"
        );
        assert!(!format!("{error:?}").contains(invalid_name));
    }
}
