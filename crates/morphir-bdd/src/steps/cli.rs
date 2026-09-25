//! CLI process steps: run the suite's program with an isolated home and record its output.
//!
//! Quoting in `When I run {string}`: inside a step text written with double quotes, `\"` groups
//! words (see [`split_command_line`]); it does not embed a literal `"`, since cucumber's
//! `{string}` capture is not unescaped. To pass an argument that itself contains a literal `"`,
//! write the step's `{string}` with single quotes instead, for example
//! `When I run 'morphir x "a b"'`: the double quotes inside it group as usual and arrive intact.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use cucumber::{then, when};
use morphir_gherkin::extension::Context;
use tokio::process::Command;

use crate::steps::files::workspace;
use crate::steps::output::LastOutput;
use crate::world::MorphirWorld;

/// The suite's program under test: the name its command lines must start with, and the path to
/// its binary.
///
/// A [`Suite::cli`](crate::suite::Suite::cli) or
/// [`Suite::cli_named`](crate::suite::Suite::cli_named) call adds a processor that inserts one of
/// these into every scenario's context, so `When I run {string}` can find the program to run.
#[derive(Debug, Clone)]
pub struct CliProgram {
    /// The command line's first word must equal this name.
    pub name: String,
    /// The path to the binary to run.
    pub path: PathBuf,
}

/// Splits `line` the way a shell does: words are separated by whitespace, and a pair of double
/// quotes groups a word (allowing embedded whitespace). Returns `Err` if a quote is left unclosed.
///
/// A backslash immediately before a `"` is treated the same as a bare `"`: it opens or closes a
/// group, and the backslash itself is dropped. This is not shell escaping; it matches cucumber's
/// `{string}` parameter, whose capture is not unescaped, so a `\"` written in a `.feature` file to
/// embed a literal quote inside the step text's own outer quotes arrives here still carrying its
/// backslash. Treating it the same as a bare `"` lets a feature quote an argument that contains
/// whitespace, such as a file name.
pub fn split_command_line(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let (mut quoted, mut in_word) = (false, false);
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'"') => {
                chars.next();
                quoted = !quoted;
                in_word = true;
            }
            '"' => {
                quoted = !quoted;
                in_word = true;
            }
            c if c.is_whitespace() && !quoted => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                word.push(c);
                in_word = true;
            }
        }
    }
    if quoted {
        return Err(format!("unclosed quote in `{line}`"));
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

/// A request to run one command line, given to a [`CliRunner`] in place of the default
/// `run_program`.
pub struct CliRequest<'a> {
    /// The suite's program under test.
    pub program: &'a CliProgram,
    /// The command line's words after the program name.
    pub args: &'a [String],
    /// The timeout `When I run {string} with a {int} second timeout` set, if the step that
    /// triggered this request named one.
    pub timeout: Option<Duration>,
    /// The running scenario's context, for a runner that keeps its own directories or other
    /// components there instead of a [`Workspace`](crate::steps::files::Workspace).
    pub context: &'a Context,
}

/// A pluggable runner for `When I run {string}`, installed with a [`CustomCliRunner`] component.
///
/// A runner owns process isolation: the default runner's is described on [`run_program`], and a
/// custom runner is free to isolate a process differently (for example `morphir itest`'s own
/// stricter per-step logs, Windows job objects and timeouts) as long as it still honors
/// [`CliRequest::timeout`] and returns the same [`LastOutput`] shape the base steps read.
pub trait CliRunner: Send + Sync + std::fmt::Debug {
    /// Runs `request.program` with `request.args`, applying this runner's own process isolation
    /// and `request.timeout`, and returns the recorded [`LastOutput`].
    fn run<'a>(
        &'a self,
        request: CliRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LastOutput, String>> + Send + 'a>>;
}

/// A component that installs a custom [`CliRunner`] in place of the default `run_program`. When
/// a scenario's context carries one, `When I run {string}` calls it instead, and does **not**
/// create a [`Workspace`](crate::steps::files::Workspace): a custom runner finds its own
/// directories elsewhere, typically through another component in [`CliRequest::context`].
#[derive(Clone, Debug)]
pub struct CustomCliRunner(pub Arc<dyn CliRunner>);

/// Runs `program` with `args` in `dir`, with an isolated environment: `HOME`, `USERPROFILE`,
/// `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `APPDATA` and `LOCALAPPDATA` all point
/// under a fresh `.home` directory inside `dir` (created if it does not exist), `MORPHIR_HOME` is
/// set to `.home/.morphir`, and `MORPHIR_NO_BANNER=1` is set. This keeps a scenario from reading
/// or writing a developer's real configuration, data or cache.
///
/// The real `morphir` CLI resolves its Morphir home from `MORPHIR_HOME` first and only falls
/// back to the OS home directory (`$HOME` on Unix, `%USERPROFILE%` on Windows) when that variable
/// is unset. Setting `MORPHIR_HOME` directly is what keeps a scenario's run out of a developer's
/// real registries; `HOME` and `USERPROFILE` (plus `APPDATA` and `LOCALAPPDATA`) are set too, on
/// every OS, as a second line of defense for anything that consults the OS home directly instead
/// of `MORPHIR_HOME`. Setting the Windows-only variables on Unix, and vice versa, is harmless: the
/// program under test only reads the ones its own OS gives meaning to.
///
/// Before applying those overrides, every inherited environment variable whose name starts with
/// `MORPHIR_` is removed from the child's environment. Without this, a developer's own
/// `MORPHIR_*` variables (for example a stray `MORPHIR_BDD_TAGS` or a real `MORPHIR_HOME`) would
/// pass straight through from this test process into the program under test and could defeat the
/// isolation above, or the tag filtering / output directory this crate's own `Suite` reads from
/// the same namespace. The strip happens first, so the overrides below always win.
///
/// It runs on `tokio::process`, so a scenario waiting for its program does not block the
/// executor: cucumber keeps running other scenarios meanwhile. It must be awaited inside a Tokio
/// runtime, as a `#[tokio::main]` or `#[tokio::test]` gives.
///
/// If `timeout` is `Some`, the child is killed once that duration elapses (`kill_on_drop(true)`
/// on the underlying `Command`: dropping the timed-out future drops the child handle, which kills
/// it), and this returns `Err` naming `program.path` and the timeout in seconds, with the text
/// `timed out`.
///
/// Returns `Err` naming `program.path` if the process fails to start, for example because the
/// binary is missing. It never reports an empty [`LastOutput`] in that case.
pub async fn run_program(
    program: &CliProgram,
    args: &[String],
    dir: &Path,
    timeout: Option<Duration>,
) -> Result<LastOutput, String> {
    let home = dir.join(".home");
    std::fs::create_dir_all(&home)
        .map_err(|e| format!("cannot create the isolated home {}: {e}", home.display()))?;
    let mut command = Command::new(&program.path);
    command.args(args).current_dir(dir).kill_on_drop(true);
    // `vars_os`, not `vars`: `vars` panics on a variable that is not valid UTF-8. A name that is
    // not UTF-8 cannot start with `MORPHIR_`, so it is skipped.
    for (name, _) in std::env::vars_os() {
        if name.to_str().is_some_and(|n| n.starts_with("MORPHIR_")) {
            command.env_remove(name);
        }
    }
    command
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("APPDATA", home.join("AppData/Roaming"))
        .env("LOCALAPPDATA", home.join("AppData/Local"))
        .env("MORPHIR_HOME", home.join(".morphir"))
        .env("MORPHIR_NO_BANNER", "1");
    let start_err = |e: std::io::Error| format!("cannot start {}: {e}", program.path.display());
    let output = match timeout {
        Some(secs) => match tokio::time::timeout(secs, command.output()).await {
            Ok(result) => result.map_err(start_err)?,
            Err(_elapsed) => {
                return Err(format!(
                    "{} timed out after {} s",
                    program.path.display(),
                    secs.as_secs_f64()
                ));
            }
        },
        None => command.output().await.map_err(start_err)?,
    };
    Ok(LastOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code(),
    })
}

/// Shared behavior behind `When I run {string}` and `When I run {string} with a {int} second
/// timeout`: splits the command line like a shell, checks that its first word names the suite's
/// program, then either calls a [`CustomCliRunner`] the suite installed (never creating a
/// [`Workspace`](crate::steps::files::Workspace)) or runs it directly with [`run_program`] inside
/// the scenario's workspace, with an isolated home. Records the result as [`LastOutput`].
async fn run_line(world: &mut MorphirWorld, line: String, timeout: Option<Duration>) {
    let program = world.context.get::<CliProgram>().cloned().expect(
        "no CLI program: the suite must call `Suite::cli(path)` or `Suite::cli_named(name, path)` \
         first",
    );
    let words = split_command_line(&line).unwrap_or_else(|e| panic!("{e}"));
    let (first, args) = words.split_first().expect("a command line names a program");
    if first != &program.name {
        panic!(
            "the command line starts with {first:?}, but the suite's program is named {:?}",
            program.name
        );
    }
    let output =
        if let Some(CustomCliRunner(runner)) = world.context.get::<CustomCliRunner>().cloned() {
            runner
                .run(CliRequest {
                    program: &program,
                    args,
                    timeout,
                    context: &world.context,
                })
                .await
        } else {
            let dir = workspace(world).dir.path().to_owned();
            run_program(&program, args, &dir, timeout).await
        };
    world
        .context
        .insert(output.unwrap_or_else(|e| panic!("{e}")));
}

/// `When I run {string}` splits the command line like a shell, checks that its first word names
/// the suite's program, runs it in the scenario's workspace with an isolated home, and records the
/// result as [`LastOutput`]. With no timeout given, it runs to completion.
#[when(expr = "I run {string}")]
async fn i_run(world: &mut MorphirWorld, line: String) {
    run_line(world, line, None).await;
}

/// `When I run {string} with a {int} second timeout` runs the command as `When I run {string}`
/// does, but fails it with a timeout error if it has not finished after `secs` seconds. See
/// [`run_program`] for how the default runner enforces this.
#[when(expr = "I run {string} with a {int} second timeout")]
async fn i_run_with_timeout(world: &mut MorphirWorld, line: String, secs: u64) {
    run_line(world, line, Some(Duration::from_secs(secs))).await;
}

/// The message a status step panics with when no command has run in the scenario yet. It names
/// the step that provides the missing [`LastOutput`], in the same style as the output steps.
const NO_COMMAND: &str = "no command has run: add `When I run \"…\"` first";

/// Returns the scenario's [`LastOutput`], panicking with [`NO_COMMAND`] if no command has run yet.
fn last(world: &MorphirWorld) -> &LastOutput {
    world.context.get::<LastOutput>().expect(NO_COMMAND)
}

/// `Then the command should succeed` asserts that the last command exited with status 0.
#[then("the command should succeed")]
fn should_succeed(world: &mut MorphirWorld) {
    let out = last(world);
    assert_eq!(
        out.status,
        Some(0),
        "the command failed:\nstdout:\n{}\nstderr:\n{}",
        out.stdout,
        out.stderr
    );
}

/// `Then the command should fail` asserts that the last command exited with a non-zero status.
#[then("the command should fail")]
fn should_fail(world: &mut MorphirWorld) {
    assert_ne!(last(world).status, Some(0), "the command succeeded");
}

/// `Then the exit code should be {int}` asserts the last command's exact exit status.
#[then(expr = "the exit code should be {int}")]
fn exit_code(world: &mut MorphirWorld, code: i32) {
    let out = last(world);
    let actual = out
        .status
        .map_or_else(|| "killed by a signal".to_owned(), |s| s.to_string());
    assert_eq!(
        out.status,
        Some(code),
        "the command exited with {actual}, not {code}:\nstdout:\n{}\nstderr:\n{}",
        out.stdout,
        out.stderr
    );
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{CliProgram, run_program, split_command_line};

    #[test]
    fn quotes_group_words() {
        assert_eq!(
            split_command_line(r#"morphir ir migrate "a file.json" --x"#).unwrap(),
            vec!["morphir", "ir", "migrate", "a file.json", "--x"]
        );
        assert!(split_command_line(r#"morphir "open"#).is_err());
    }

    #[tokio::test]
    async fn run_program_reports_a_missing_binary_by_path() {
        let dir = tempfile::tempdir().expect("create a temporary directory");
        let program = CliProgram {
            name: "morphir".to_owned(),
            path: PathBuf::from("/no/such/morphir-binary"),
        };

        let err = run_program(&program, &[], dir.path(), None)
            .await
            .expect_err("the binary does not exist");

        assert!(
            err.contains("/no/such/morphir-binary"),
            "the error must name the program path: {err}"
        );
    }

    /// A4: a command that outlives its timeout is killed and reported as a timeout, not left to
    /// run to completion. `/bin/sh -c "sleep 5"` with a 1 s timeout must return `Err` containing
    /// `timed out` well before the 5 s sleep would otherwise finish.
    #[cfg(unix)]
    #[tokio::test]
    async fn run_program_times_out_and_kills_the_child() {
        let dir = tempfile::tempdir().expect("create a temporary directory");
        let program = CliProgram {
            name: "sh".to_owned(),
            path: PathBuf::from("/bin/sh"),
        };
        let args = vec!["-c".to_owned(), "sleep 5".to_owned()];

        let start = std::time::Instant::now();
        let err = run_program(&program, &args, dir.path(), Some(Duration::from_secs(1)))
            .await
            .expect_err("the command must time out");
        let elapsed = start.elapsed();

        assert!(err.contains("timed out"), "{err}");
        assert!(
            elapsed < Duration::from_secs(3),
            "the timeout must cut the run short, took {elapsed:?}"
        );
    }
}
