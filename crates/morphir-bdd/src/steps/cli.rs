//! CLI process steps: run the suite's program with an isolated home and record its output.
//!
//! Quoting in `When I run {string}`: inside a step text written with double quotes, `\"` groups
//! words (see [`split_command_line`]); it does not embed a literal `"`, since cucumber's
//! `{string}` capture is not unescaped. To pass an argument that itself contains a literal `"`,
//! write the step's `{string}` with single quotes instead, for example
//! `When I run 'morphir x "a b"'`: the double quotes inside it group as usual and arrive intact.

use std::path::{Path, PathBuf};
use std::process::Command;

use cucumber::{then, when};

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

/// Runs `program` with `args` in `dir`, with an isolated environment: `HOME`,
/// `XDG_CONFIG_HOME`, `XDG_DATA_HOME` and `XDG_CACHE_HOME` all point under a fresh `.home`
/// directory inside `dir` (created if it does not exist), and `MORPHIR_NO_BANNER=1` is set. This
/// keeps a scenario from reading or writing a developer's real configuration, data or cache.
///
/// Before applying those overrides, every inherited environment variable whose name starts with
/// `MORPHIR_` is removed from the child's environment. Without this, a developer's own
/// `MORPHIR_*` variables (for example a stray `MORPHIR_BDD_TAGS` or a real `MORPHIR_HOME`) would
/// pass straight through from this test process into the program under test and could defeat the
/// isolation above, or the tag filtering / output directory this crate's own `Suite` reads from
/// the same namespace.
///
/// Returns `Err` naming `program.path` if the process fails to start, for example because the
/// binary is missing. It never reports an empty [`LastOutput`] in that case.
pub fn run_program(
    program: &CliProgram,
    args: &[String],
    dir: &Path,
) -> Result<LastOutput, String> {
    let home = dir.join(".home");
    std::fs::create_dir_all(&home)
        .map_err(|e| format!("cannot create the isolated home {}: {e}", home.display()))?;
    let mut command = Command::new(&program.path);
    command.args(args).current_dir(dir);
    for (name, _) in std::env::vars() {
        if name.starts_with("MORPHIR_") {
            command.env_remove(name);
        }
    }
    let output = command
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("MORPHIR_NO_BANNER", "1")
        .output()
        .map_err(|e| format!("cannot start {}: {e}", program.path.display()))?;
    Ok(LastOutput {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code(),
    })
}

/// `When I run {string}` splits the command line like a shell, checks that its first word names
/// the suite's program, runs it in the scenario's workspace with an isolated home, and records the
/// result as [`LastOutput`].
#[when(expr = "I run {string}")]
fn i_run(world: &mut MorphirWorld, line: String) {
    let program = world.context.get::<CliProgram>().cloned().expect(
        "no CLI program: the suite must call Suite::cli(path) or Suite::cli_named(name, path)",
    );
    let words = split_command_line(&line).unwrap_or_else(|e| panic!("{e}"));
    let (first, args) = words.split_first().expect("a command line names a program");
    if first != &program.name {
        panic!(
            "the command line starts with {first:?}, but the suite's program is named {:?}",
            program.name
        );
    }
    let dir = workspace(world).dir.path().to_owned();
    let output = run_program(&program, args, &dir).unwrap_or_else(|e| panic!("{e}"));
    world.context.insert(output);
}

/// Returns the last command's exit status, panicking if no command has run yet.
fn status(world: &MorphirWorld) -> Option<i32> {
    world
        .context
        .get::<LastOutput>()
        .expect("no command has run")
        .status
}

/// `Then the command should succeed` asserts that the last command exited with status 0.
#[then("the command should succeed")]
fn should_succeed(world: &mut MorphirWorld) {
    let out = world
        .context
        .get::<LastOutput>()
        .expect("no command has run");
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
    assert_ne!(status(world), Some(0), "the command succeeded");
}

/// `Then the exit code should be {int}` asserts the last command's exact exit status.
#[then(expr = "the exit code should be {int}")]
fn exit_code(world: &mut MorphirWorld, code: i32) {
    let out = world
        .context
        .get::<LastOutput>()
        .expect("no command has run");
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

    use super::{CliProgram, run_program, split_command_line};

    #[test]
    fn quotes_group_words() {
        assert_eq!(
            split_command_line(r#"morphir ir migrate "a file.json" --x"#).unwrap(),
            vec!["morphir", "ir", "migrate", "a file.json", "--x"]
        );
        assert!(split_command_line(r#"morphir "open"#).is_err());
    }

    #[test]
    fn run_program_reports_a_missing_binary_by_path() {
        let dir = tempfile::tempdir().expect("create a temporary directory");
        let program = CliProgram {
            name: "morphir".to_owned(),
            path: PathBuf::from("/no/such/morphir-binary"),
        };

        let err = run_program(&program, &[], dir.path()).expect_err("the binary does not exist");

        assert!(
            err.contains("/no/such/morphir-binary"),
            "the error must name the program path: {err}"
        );
    }
}
