use super::{
    SecretReference, SecretResolutionContext, SecretResolutionError, SecretResolver, SecretString,
};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeyringReadFailure;

trait KeyringReader: Send + Sync {
    fn read(&self, service: &str, account: &str) -> Result<String, KeyringReadFailure>;
}

#[derive(Debug, Default, Clone, Copy)]
struct NativeKeyringReader;

impl KeyringReader for NativeKeyringReader {
    fn read(&self, service: &str, account: &str) -> Result<String, KeyringReadFailure> {
        let entry = keyring::Entry::new(service, account).map_err(|_| KeyringReadFailure)?;
        entry.get_password().map_err(|_| KeyringReadFailure)
    }
}

/// Resolver for secret sources provided by the local operating system.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemSecretResolver;

impl SystemSecretResolver {
    fn resolve_with_keyring_reader(
        &self,
        reference: &SecretReference,
        context: SecretResolutionContext<'_>,
        keyring_reader: &dyn KeyringReader,
    ) -> Result<SecretString, SecretResolutionError> {
        match reference {
            SecretReference::Environment { variable } => {
                read_process_environment_value(variable, context.config_key, |name| {
                    std::env::var_os(name)
                })
                .map(SecretString::from)
            }
            SecretReference::File { path } => {
                read_secret_file(path, context.declaring_file).map(SecretString::from)
            }
            SecretReference::Command { program, args } => {
                run_secret_command(program, args, context)
            }
            SecretReference::Keyring { service, account } => keyring_reader
                .read(service, account)
                .map_err(|_| SecretResolutionError::KeyringLookupFailed {
                    config_key: context.config_key.to_owned(),
                    service: service.to_owned(),
                    account: account.to_owned(),
                })
                .and_then(|value| require_non_empty(value, "keyring"))
                .map(SecretString::from),
        }
    }
}

impl SecretResolver for SystemSecretResolver {
    fn resolve(
        &self,
        reference: &SecretReference,
        context: SecretResolutionContext<'_>,
    ) -> Result<SecretString, SecretResolutionError> {
        self.resolve_with_keyring_reader(reference, context, &NativeKeyringReader)
    }
}

fn read_process_environment_value(
    variable: &str,
    config_key: &str,
    lookup: impl FnOnce(&str) -> Option<OsString>,
) -> Result<String, SecretResolutionError> {
    if variable.contains(['=', '\0']) {
        return Err(SecretResolutionError::InvalidEnvironmentName {
            config_key: config_key.to_owned(),
        });
    }

    read_environment_value(variable, lookup(variable))
}

fn read_environment_value(
    variable: &str,
    value: Option<OsString>,
) -> Result<String, SecretResolutionError> {
    let value = value.ok_or_else(|| SecretResolutionError::EnvironmentMissing {
        variable: variable.to_owned(),
    })?;
    let value = value
        .into_string()
        .map_err(|_| SecretResolutionError::EnvironmentNotUnicode {
            variable: variable.to_owned(),
        })?;
    require_non_empty(value, "environment")
}

fn read_secret_file(
    path: &Path,
    declaring_file: Option<&Path>,
) -> Result<String, SecretResolutionError> {
    read_secret_file_with_home(path, declaring_file, dirs::home_dir().as_deref())
}

fn read_secret_file_with_home(
    path: &Path,
    declaring_file: Option<&Path>,
    home: Option<&Path>,
) -> Result<String, SecretResolutionError> {
    let path = resolve_secret_file_path(path, declaring_file, home)?;
    let bytes = fs::read(&path).map_err(|error| SecretResolutionError::FileRead {
        path: path.clone(),
        kind: error.kind(),
    })?;
    let mut value =
        String::from_utf8(bytes).map_err(|_| SecretResolutionError::FileNotUnicode { path })?;
    strip_one_line_ending(&mut value);
    require_non_empty(value, "file")
}

fn resolve_secret_file_path(
    path: &Path,
    declaring_file: Option<&Path>,
    home: Option<&Path>,
) -> Result<PathBuf, SecretResolutionError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    if path == Path::new("~") {
        return home
            .map(Path::to_path_buf)
            .ok_or(SecretResolutionError::HomeDirectoryUnavailable);
    }

    if let Ok(relative_to_home) = path.strip_prefix("~") {
        let home = home.ok_or(SecretResolutionError::HomeDirectoryUnavailable)?;
        return Ok(home.join(relative_to_home));
    }

    if path
        .components()
        .next()
        .is_some_and(|component| component.as_os_str().to_string_lossy().starts_with('~'))
    {
        return Err(SecretResolutionError::UnsupportedHomePath {
            path: path.to_path_buf(),
        });
    }

    let declaring_file =
        declaring_file.ok_or_else(|| SecretResolutionError::RelativeFileWithoutDeclaringFile {
            path: path.to_path_buf(),
        })?;
    let declaring_directory = declaring_file.parent().unwrap_or_else(|| Path::new(""));
    Ok(declaring_directory.join(path))
}

fn strip_one_line_ending(value: &mut String) {
    if value.ends_with("\r\n") {
        value.truncate(value.len() - 2);
    } else if value.ends_with('\n') {
        value.pop();
    }
}

fn require_non_empty(
    value: String,
    backend: &'static str,
) -> Result<String, SecretResolutionError> {
    if value.is_empty() {
        Err(SecretResolutionError::EmptySecret { backend })
    } else {
        Ok(value)
    }
}

fn run_secret_command(
    program: &str,
    args: &[String],
    context: SecretResolutionContext<'_>,
) -> Result<SecretString, SecretResolutionError> {
    let cwd = command_working_directory(context.declaring_file).map_err(|kind| {
        SecretResolutionError::CommandCurrentDirectory {
            config_key: context.config_key.to_owned(),
            kind,
        }
    })?;
    let executable = command_program(Path::new(program), &cwd);
    let output = Command::new(executable)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| SecretResolutionError::CommandSpawn {
            config_key: context.config_key.to_owned(),
            program: program.to_owned(),
            kind: error.kind(),
        })?;

    if !output.status.success() {
        return Err(SecretResolutionError::CommandFailed {
            config_key: context.config_key.to_owned(),
            program: program.to_owned(),
            status_code: output.status.code(),
        });
    }

    let mut value = String::from_utf8(output.stdout).map_err(|_| {
        SecretResolutionError::CommandOutputNotUnicode {
            config_key: context.config_key.to_owned(),
            program: program.to_owned(),
        }
    })?;
    strip_one_line_ending(&mut value);
    require_non_empty(value, "command").map(SecretString::from)
}

fn command_program(program: &Path, working_directory: &Path) -> PathBuf {
    if program.is_relative() && program.components().count() > 1 {
        working_directory.join(program)
    } else {
        program.to_path_buf()
    }
}

fn command_working_directory(declaring_file: Option<&Path>) -> Result<PathBuf, std::io::ErrorKind> {
    match declaring_file {
        Some(path) => Ok(path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()),
        None => std::env::current_dir().map_err(|error| error.kind()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::secret::ExposeSecret;
    use std::io::{Read, Write};
    use std::path::PathBuf;
    use std::sync::Mutex;

    struct ReturningKeyringReader {
        password: &'static str,
        calls: Mutex<Vec<(String, String)>>,
    }

    impl ReturningKeyringReader {
        fn new(password: &'static str) -> Self {
            Self {
                password,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, String)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl KeyringReader for ReturningKeyringReader {
        fn read(&self, service: &str, account: &str) -> Result<String, KeyringReadFailure> {
            self.calls
                .lock()
                .unwrap()
                .push((service.to_owned(), account.to_owned()));
            Ok(self.password.to_owned())
        }
    }

    struct FailingKeyringReader {
        private_diagnostic: &'static str,
    }

    impl KeyringReader for FailingKeyringReader {
        fn read(&self, _service: &str, _account: &str) -> Result<String, KeyringReadFailure> {
            let _ = self.private_diagnostic;
            Err(KeyringReadFailure)
        }
    }

    fn write_file(path: &Path, value: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, value).unwrap();
    }

    #[test]
    fn environment_requires_present_non_empty_unicode_text() {
        assert_eq!(
            read_environment_value("TOKEN", Some(OsString::from("env-token"))).unwrap(),
            "env-token"
        );
        assert!(matches!(
            read_environment_value("TOKEN", None),
            Err(SecretResolutionError::EnvironmentMissing { .. })
        ));
        assert!(matches!(
            read_environment_value("TOKEN", Some(OsString::new())),
            Err(SecretResolutionError::EmptySecret {
                backend: "environment"
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn environment_rejects_non_unicode_text() {
        use std::os::unix::ffi::OsStringExt;
        let value = OsString::from_vec(vec![0xff]);
        assert!(matches!(
            read_environment_value("TOKEN", Some(value)),
            Err(SecretResolutionError::EnvironmentNotUnicode { .. })
        ));
    }

    #[test]
    fn relative_file_uses_declaring_file_directory_and_removes_one_line_ending() {
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join(".config/morphir/config.user.toml");
        let secret_path = config.parent().unwrap().join("secrets/token");
        write_file(&secret_path, "file-token\r\n");

        let actual = read_secret_file(Path::new("secrets/token"), Some(&config)).unwrap();

        assert_eq!(actual, "file-token");
    }

    #[test]
    fn absolute_file_is_used_without_a_declaring_file() {
        let root = tempfile::tempdir().unwrap();
        let secret_path = root.path().join("token");
        write_file(&secret_path, "absolute-token\n");

        let actual = read_secret_file(&secret_path, None).unwrap();

        assert_eq!(actual, "absolute-token");
    }

    #[test]
    fn tilde_and_tilde_slash_expand_from_an_injected_home() {
        let root = tempfile::tempdir().unwrap();
        let home_file = root.path().join("home-secret");
        let home_directory = root.path().join("home-directory");
        let nested = home_directory.join("secrets/token");
        write_file(&home_file, "home-token");
        write_file(&nested, "nested-token");

        assert_eq!(
            read_secret_file_with_home(Path::new("~"), None, Some(&home_file)).unwrap(),
            "home-token"
        );
        assert_eq!(
            read_secret_file_with_home(Path::new("~/secrets/token"), None, Some(&home_directory),)
                .unwrap(),
            "nested-token"
        );
    }

    #[test]
    fn unsupported_named_home_is_rejected() {
        let error = read_secret_file_with_home(
            Path::new("~another-user/token"),
            None,
            Some(Path::new("/home/current")),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::UnsupportedHomePath { .. }
        ));
    }

    #[test]
    fn relative_file_requires_a_declaring_file() {
        let error = read_secret_file(Path::new("secrets/token"), None).unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::RelativeFileWithoutDeclaringFile { .. }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn file_rejects_non_utf8_content() {
        let root = tempfile::tempdir().unwrap();
        let secret_path = root.path().join("token");
        fs::write(&secret_path, [0xff]).unwrap();

        let error = read_secret_file(&secret_path, None).unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::FileNotUnicode { .. }
        ));
    }

    #[test]
    fn file_rejects_empty_content() {
        let root = tempfile::tempdir().unwrap();
        let secret_path = root.path().join("token");
        fs::write(&secret_path, []).unwrap();

        let error = read_secret_file(&secret_path, None).unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::EmptySecret { backend: "file" }
        ));
    }

    #[test]
    fn file_preserves_spaces_and_removes_exactly_one_line_ending() {
        let root = tempfile::tempdir().unwrap();
        let secret_path = root.path().join("token");
        write_file(&secret_path, "  file-token  \n\n");

        let actual = read_secret_file(&secret_path, None).unwrap();

        assert_eq!(actual, "  file-token  \n");
    }

    #[test]
    fn missing_home_is_a_typed_error() {
        let error = read_secret_file_with_home(Path::new("~/token"), None, None).unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::HomeDirectoryUnavailable
        ));
    }

    #[test]
    fn file_read_error_does_not_contain_file_contents() {
        let missing = PathBuf::from("/definitely/not/a/secret/file");
        let error = read_secret_file(&missing, None).unwrap_err();

        assert!(matches!(error, SecretResolutionError::FileRead { .. }));
    }

    fn test_executable() -> PathBuf {
        std::env::current_exe().unwrap()
    }

    fn listed_test_args(name: &str) -> Vec<String> {
        vec![
            name.to_owned(),
            "--exact".into(),
            "--list".into(),
            "--format".into(),
            "terse".into(),
        ]
    }

    fn invoked_test_args(name: &str, ignored: bool) -> Vec<String> {
        let mut args = vec![name.to_owned(), "--exact".into(), "--nocapture".into()];
        if ignored {
            args.push("--ignored".into());
        }
        args
    }

    fn context(declaring_file: Option<&Path>) -> SecretResolutionContext<'_> {
        SecretResolutionContext {
            config_key: "registry.token",
            declaring_file,
        }
    }

    fn a_keyring_reference() -> SecretReference {
        SecretReference::Keyring {
            service: "morphir.registry".to_owned(),
            account: "build-user".to_owned(),
        }
    }

    #[test]
    fn keyring_dispatch_passes_identifiers_and_protects_the_password() {
        let reader = ReturningKeyringReader::new("native-keyring-password");

        let secret = SystemSecretResolver
            .resolve_with_keyring_reader(&a_keyring_reference(), context(None), &reader)
            .unwrap();

        assert_eq!(
            reader.calls(),
            vec![("morphir.registry".to_owned(), "build-user".to_owned())]
        );
        assert_eq!(secret.expose_secret(), "native-keyring-password");
        assert!(!format!("{secret:?}").contains("native-keyring-password"));
    }

    #[test]
    fn keyring_dispatch_rejects_an_empty_password() {
        let reader = ReturningKeyringReader::new("");

        let error = SystemSecretResolver
            .resolve_with_keyring_reader(&a_keyring_reference(), context(None), &reader)
            .unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::EmptySecret { backend: "keyring" }
        ));
    }

    #[test]
    fn keyring_dispatch_maps_failures_without_private_diagnostics() {
        let private_sentinel = "private-keyring-error-sentinel";
        let reader = FailingKeyringReader {
            private_diagnostic: private_sentinel,
        };

        let error = SystemSecretResolver
            .resolve_with_keyring_reader(&a_keyring_reference(), context(None), &reader)
            .unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::KeyringLookupFailed {
                ref config_key,
                ref service,
                ref account,
            } if config_key == "registry.token"
                && service == "morphir.registry"
                && account == "build-user"
        ));
        assert!(!format!("{error}").contains(private_sentinel));
        assert!(!format!("{error:?}").contains(private_sentinel));
    }

    #[test]
    fn direct_command_captures_stdout_and_removes_one_line_ending() {
        let program = test_executable();
        let args = listed_test_args("config::secret::system::tests::secret_process_listing_token");

        let secret = run_secret_command(&program.to_string_lossy(), &args, context(None)).unwrap();

        assert_eq!(
            secret.expose_secret(),
            "config::secret::system::tests::secret_process_listing_token: test"
        );
    }

    #[test]
    fn direct_command_uses_the_declaring_file_directory() {
        let root = tempfile::tempdir().unwrap();
        let config_directory = root.path().join("config");
        fs::create_dir_all(&config_directory).unwrap();
        let config_file = config_directory.join("morphir.user.toml");
        let helper_name = format!("secret-helper{}", std::env::consts::EXE_SUFFIX);
        fs::copy(test_executable(), config_directory.join(&helper_name)).unwrap();
        let relative_program = Path::new(".").join(helper_name);
        let args = listed_test_args("config::secret::system::tests::secret_process_listing_token");

        let secret = run_secret_command(
            &relative_program.to_string_lossy(),
            &args,
            context(Some(&config_file)),
        )
        .unwrap();

        assert_eq!(
            secret.expose_secret(),
            "config::secret::system::tests::secret_process_listing_token: test"
        );
    }

    #[test]
    fn relative_path_like_programs_resolve_against_the_command_directory() {
        let directory = Path::new("config/project");

        assert_eq!(
            command_program(Path::new("./helper"), directory),
            directory.join("./helper")
        );
        assert_eq!(
            command_program(Path::new("../bin/helper"), directory),
            directory.join("../bin/helper")
        );
        assert_eq!(
            command_program(Path::new("helper"), directory),
            Path::new("helper")
        );
    }

    #[test]
    fn direct_command_inherits_environment_and_closes_stdin() {
        let program = test_executable();
        let args = invoked_test_args(
            "config::secret::system::tests::secret_process_checks_environment_and_stdin",
            false,
        );

        assert!(run_secret_command(&program.to_string_lossy(), &args, context(None)).is_ok());
    }

    #[test]
    fn direct_command_rejects_empty_stdout() {
        let program = test_executable();
        let args = listed_test_args("no-test-has-this-name");

        let error =
            run_secret_command(&program.to_string_lossy(), &args, context(None)).unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::EmptySecret { backend: "command" }
        ));
    }

    #[test]
    fn direct_command_reports_missing_executable_without_backend_text() {
        let program = "definitely-not-a-morphir-secret-helper";

        let error = run_secret_command(program, &[], context(None)).unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::CommandSpawn {
                ref config_key,
                ref program,
                kind: std::io::ErrorKind::NotFound,
            } if config_key == "registry.token"
                && program == "definitely-not-a-morphir-secret-helper"
        ));
    }

    #[test]
    fn direct_command_failure_drops_captured_stdout_and_stderr() {
        let stdout_sentinel = "stdout-secret-sentinel";
        let stderr_sentinel = "stderr-secret-sentinel";
        let program = test_executable();
        let args = invoked_test_args(
            "config::secret::system::tests::secret_process_emits_secrets_then_fails",
            true,
        );

        let error =
            run_secret_command(&program.to_string_lossy(), &args, context(None)).unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::CommandFailed {
                ref config_key,
                status_code: Some(_),
                ..
            } if config_key == "registry.token"
        ));
        let display = format!("{error}");
        let debug = format!("{error:?}");
        assert!(!display.contains(stdout_sentinel));
        assert!(!display.contains(stderr_sentinel));
        assert!(!debug.contains(stdout_sentinel));
        assert!(!debug.contains(stderr_sentinel));
    }

    #[cfg(unix)]
    #[test]
    fn direct_command_rejects_non_utf8_stdout() {
        let program = test_executable();
        let args = invoked_test_args(
            "config::secret::system::tests::secret_process_emits_non_utf8",
            true,
        );

        let error =
            run_secret_command(&program.to_string_lossy(), &args, context(None)).unwrap_err();

        assert!(matches!(
            error,
            SecretResolutionError::CommandOutputNotUnicode { .. }
        ));
    }

    #[test]
    fn command_without_an_origin_uses_the_process_current_directory() {
        assert_eq!(
            command_working_directory(None).unwrap(),
            std::env::current_dir().unwrap()
        );
    }

    #[test]
    fn secret_process_listing_token() {}

    #[test]
    fn secret_process_checks_environment_and_stdin() {
        assert!(std::env::var_os("PATH").is_some());
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).unwrap();
        assert!(input.is_empty());
    }

    #[test]
    #[ignore]
    fn secret_process_emits_secrets_then_fails() {
        std::io::stdout()
            .write_all(b"stdout-secret-sentinel")
            .unwrap();
        std::io::stderr()
            .write_all(b"stderr-secret-sentinel")
            .unwrap();
        std::io::stdout().flush().unwrap();
        std::io::stderr().flush().unwrap();
        panic!("intentional helper failure");
    }

    #[cfg(unix)]
    #[test]
    #[ignore]
    fn secret_process_emits_non_utf8() {
        std::io::stdout().write_all(&[0xff]).unwrap();
        std::io::stdout().flush().unwrap();
    }
}
