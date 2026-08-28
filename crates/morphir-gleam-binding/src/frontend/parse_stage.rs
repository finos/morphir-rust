//! Capability-confined, transactional parse-stage output.
//!
//! `cap-std` and `cap-fs-ext` provide maintained cross-platform directory
//! handles and no-follow operations. Staging beneath the opened output root
//! keeps atomic renames on one filesystem. Existing files are copied to
//! transaction-local backups before a staged file atomically replaces them.

use super::ast::ModuleIR;
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use std::ffi::{OsStr, OsString};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TRANSACTION_ID: AtomicU64 = AtomicU64::new(0);

/// One parsed module and the source document responsible for it.
pub(crate) struct ParseStageModule<'a> {
    pub(crate) module_name: &'a str,
    pub(crate) uri: &'a str,
    pub(crate) module: &'a ModuleIR,
}

/// An emission failure and, when applicable, its responsible source document.
#[derive(Debug)]
pub(crate) struct EmitFailure {
    pub(crate) message: String,
    pub(crate) uri: Option<String>,
}

/// The state of parse-stage output after an emission attempt.
#[derive(Debug)]
pub(crate) enum EmitParseStageOutcome {
    /// All requested outputs were atomically installed.
    Committed {
        /// Failure to remove transaction artifacts after the successful commit.
        cleanup_warning: Option<String>,
    },
    /// No requested output remains partially updated.
    RolledBack { failure: EmitFailure },
    /// Rollback failed; transaction backups were retained at `recovery_path`.
    RecoveryRequired {
        failure: EmitFailure,
        recovery_path: PathBuf,
    },
}

struct StagedModule {
    destination: PathBuf,
    staged: PathBuf,
    contents: Vec<u8>,
    uri: String,
}

struct Destination {
    parent: Dir,
    leaf: OsString,
    staged: PathBuf,
    backup: PathBuf,
    uri: String,
}

struct CommitRecord {
    destination_index: usize,
    backup: Option<PathBuf>,
    installed: bool,
}

struct Transaction {
    directory: Dir,
    relative_path: PathBuf,
}

#[derive(Default)]
struct Faults {
    fail_commit_at: Option<usize>,
    fail_rollback_at: Option<usize>,
    fail_cleanup: bool,
}

enum RunFailure {
    RolledBack(EmitFailure),
    RecoveryRequired(EmitFailure),
}

/// Serialize and publish the full submitted parse-stage tree as one transaction.
pub(crate) fn emit_parse_stage(
    output_dir: &Path,
    modules: &[ParseStageModule<'_>],
) -> EmitParseStageOutcome {
    if modules.is_empty() {
        return EmitParseStageOutcome::Committed {
            cleanup_warning: None,
        };
    }
    emit_parse_stage_with_faults(output_dir, modules, &Faults::default())
}

fn emit_parse_stage_with_faults(
    output_dir: &Path,
    modules: &[ParseStageModule<'_>],
    faults: &Faults,
) -> EmitParseStageOutcome {
    let staged_modules = match serialize_modules(modules) {
        Ok(modules) => modules,
        Err(failure) => return EmitParseStageOutcome::RolledBack { failure },
    };
    if let Err(error) = std::fs::create_dir_all(output_dir) {
        return rolled_back(error, None);
    }
    // Ambient authority is intentionally limited to selecting the configured
    // output root. Every operation beneath it is descriptor-relative.
    let root = match Dir::open_ambient_dir(output_dir, cap_std::ambient_authority()) {
        Ok(root) => root,
        Err(error) => return rolled_back(error, None),
    };
    let transaction = match create_transaction(&root) {
        Ok(transaction) => transaction,
        Err(error) => return rolled_back(error, None),
    };
    let recovery_path = output_dir.join(&transaction.relative_path);

    match run_transaction(&root, &transaction.directory, &staged_modules, faults) {
        Ok(()) => {
            let cleanup_warning = cleanup_transaction(transaction.directory, faults)
                .err()
                .map(|error| {
                    format!(
                        "Parse-stage output was committed, but transaction cleanup failed at '{}': {error}",
                        recovery_path.display()
                    )
                });
            EmitParseStageOutcome::Committed { cleanup_warning }
        }
        Err(RunFailure::RolledBack(failure)) => {
            let cleanup = cleanup_transaction(transaction.directory, faults);
            let failure = match cleanup {
                Ok(()) => failure,
                Err(error) => EmitFailure {
                    message: format!(
                        "{}; additionally failed to clean parse-stage transaction '{}': {error}",
                        failure.message,
                        recovery_path.display()
                    ),
                    uri: None,
                },
            };
            EmitParseStageOutcome::RolledBack { failure }
        }
        Err(RunFailure::RecoveryRequired(failure)) => EmitParseStageOutcome::RecoveryRequired {
            failure,
            recovery_path,
        },
    }
}

fn serialize_modules(modules: &[ParseStageModule<'_>]) -> Result<Vec<StagedModule>, EmitFailure> {
    modules
        .iter()
        .map(|module| {
            serde_json::to_vec_pretty(module.module)
                .map(|contents| StagedModule {
                    destination: json_path("parse", module.module_name),
                    staged: json_path("staged", module.module_name),
                    contents,
                    uri: module.uri.to_owned(),
                })
                .map_err(|error| failure(error, Some(module.uri)))
        })
        .collect()
}

fn run_transaction(
    root: &Dir,
    transaction: &Dir,
    modules: &[StagedModule],
    faults: &Faults,
) -> Result<(), RunFailure> {
    transaction
        .create_dir("staged")
        .and_then(|()| transaction.create_dir("backups"))
        .map_err(|error| RunFailure::RolledBack(failure(error, None)))?;
    for module in modules {
        stage_module(transaction, module)
            .map_err(|error| RunFailure::RolledBack(failure(error, Some(&module.uri))))?;
    }
    for module in modules {
        preflight_destination(root, &module.destination)
            .map_err(|error| RunFailure::RolledBack(failure(error, Some(&module.uri))))?;
    }

    let mut created_directories = Vec::new();
    let destinations =
        prepare_destinations(root, modules, &mut created_directories).map_err(|failure| {
            remove_created_directories(root, &created_directories);
            RunFailure::RolledBack(failure)
        })?;
    let commit_result = commit_destinations(transaction, &destinations, faults);
    drop(destinations);
    if commit_result.is_err() {
        remove_created_directories(root, &created_directories);
    }
    commit_result
}

fn json_path(prefix: &str, module_name: &str) -> PathBuf {
    let mut path = PathBuf::from(prefix).join(module_name);
    path.set_extension("json");
    path
}

fn create_transaction(root: &Dir) -> anyhow::Result<Transaction> {
    for _ in 0..100 {
        let id = NEXT_TRANSACTION_ID.fetch_add(1, Ordering::Relaxed);
        let relative_path =
            PathBuf::from(format!(".morphir-parse-stage-{}-{id}", std::process::id()));
        match root.create_dir(&relative_path) {
            Ok(()) => {
                return Ok(Transaction {
                    directory: root.open_dir_nofollow(&relative_path)?,
                    relative_path,
                });
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    anyhow::bail!("Unable to allocate a parse-stage transaction directory")
}

fn cleanup_transaction(transaction: Dir, faults: &Faults) -> anyhow::Result<()> {
    if faults.fail_cleanup {
        anyhow::bail!("injected transaction cleanup failure");
    }
    transaction.remove_open_dir_all()?;
    Ok(())
}

fn stage_module(transaction: &Dir, module: &StagedModule) -> anyhow::Result<()> {
    let parent_path = module
        .staged
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Staged parse output has no parent"))?;
    let mut created = Vec::new();
    let parent = ensure_directory_path(transaction, parent_path, &mut created)?;
    let leaf = module
        .staged
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Staged parse output has no file name"))?;
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let mut file = parent.open_with(leaf, &options)?;
    file.write_all(&module.contents)?;
    file.sync_all()?;
    Ok(())
}

fn preflight_destination(root: &Dir, destination: &Path) -> anyhow::Result<()> {
    let parent_path = destination
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Parse-stage destination has no parent"))?;
    let Some(parent) = open_existing_directory_path(root, parent_path)? else {
        return Ok(());
    };
    let leaf = destination
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Parse-stage destination has no file name"))?;
    validate_leaf(&parent, leaf).map(|_| ())
}

fn prepare_destinations(
    root: &Dir,
    modules: &[StagedModule],
    created_directories: &mut Vec<PathBuf>,
) -> Result<Vec<Destination>, EmitFailure> {
    modules
        .iter()
        .map(|module| {
            let result = (|| {
                let parent_path = module
                    .destination
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("Parse-stage destination has no parent"))?;
                let parent = ensure_directory_path(root, parent_path, created_directories)?;
                let leaf = module
                    .destination
                    .file_name()
                    .ok_or_else(|| anyhow::anyhow!("Parse-stage destination has no file name"))?
                    .to_owned();
                Ok(Destination {
                    parent,
                    leaf,
                    staged: module.staged.clone(),
                    backup: PathBuf::from("backups").join(&module.destination),
                    uri: module.uri.clone(),
                })
            })();
            result.map_err(|error: anyhow::Error| failure(error, Some(&module.uri)))
        })
        .collect()
}

fn commit_destinations(
    transaction: &Dir,
    destinations: &[Destination],
    faults: &Faults,
) -> Result<(), RunFailure> {
    let mut records = Vec::with_capacity(destinations.len());
    for (index, destination) in destinations.iter().enumerate() {
        let leaf_state = match validate_leaf(&destination.parent, &destination.leaf) {
            Ok(state) => state,
            Err(error) => {
                return Err(rollback_after_error(
                    failure(error, Some(&destination.uri)),
                    transaction,
                    destinations,
                    &records,
                    faults,
                ));
            }
        };
        let backup = match leaf_state {
            LeafState::Absent => None,
            LeafState::Regular => {
                let backup_metadata =
                    match hard_link_destination_to_backup(transaction, destination) {
                        Ok(metadata) => metadata,
                        Err(error) => {
                            return Err(rollback_after_error(
                                failure(error, Some(&destination.uri)),
                                transaction,
                                destinations,
                                &records,
                                faults,
                            ));
                        }
                    };
                if let Err(error) =
                    transaction.set_permissions(&destination.staged, backup_metadata.permissions())
                {
                    return Err(rollback_after_error(
                        failure(error, Some(&destination.uri)),
                        transaction,
                        destinations,
                        &records,
                        faults,
                    ));
                }
                Some(destination.backup.clone())
            }
        };
        records.push(CommitRecord {
            destination_index: index,
            backup,
            installed: false,
        });

        let install_result = if faults.fail_commit_at == Some(index) {
            Err(anyhow::anyhow!("injected parse-stage commit failure"))
        } else {
            transaction
                .rename(&destination.staged, &destination.parent, &destination.leaf)
                .map_err(anyhow::Error::from)
        };
        if let Err(error) = install_result {
            return Err(rollback_after_error(
                failure(error, Some(&destination.uri)),
                transaction,
                destinations,
                &records,
                faults,
            ));
        }
        records.last_mut().expect("commit record exists").installed = true;
    }
    Ok(())
}

fn hard_link_destination_to_backup(
    transaction: &Dir,
    destination: &Destination,
) -> anyhow::Result<cap_std::fs::Metadata> {
    let backup_parent_path = destination
        .backup
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Parse-stage backup has no parent"))?;
    let backup_leaf = destination
        .backup
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Parse-stage backup has no file name"))?;
    let mut created = Vec::new();
    let backup_parent = ensure_directory_path(transaction, backup_parent_path, &mut created)?;
    destination
        .parent
        .hard_link(&destination.leaf, &backup_parent, backup_leaf)?;
    let metadata = backup_parent.symlink_metadata(backup_leaf)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        anyhow::bail!("Parse-stage backup is not a regular file");
    }
    Ok(metadata)
}

fn rollback_after_error(
    error: EmitFailure,
    transaction: &Dir,
    destinations: &[Destination],
    records: &[CommitRecord],
    faults: &Faults,
) -> RunFailure {
    match rollback(transaction, destinations, records, faults) {
        Ok(()) => RunFailure::RolledBack(error),
        Err(rollback_error) => RunFailure::RecoveryRequired(EmitFailure {
            message: format!(
                "{}; parse-stage rollback failed: {rollback_error}",
                error.message
            ),
            uri: None,
        }),
    }
}

fn rollback(
    transaction: &Dir,
    destinations: &[Destination],
    records: &[CommitRecord],
    faults: &Faults,
) -> anyhow::Result<()> {
    let mut errors = Vec::new();
    for record in records.iter().rev() {
        let destination = &destinations[record.destination_index];
        if faults.fail_rollback_at == Some(record.destination_index) {
            errors.push("injected parse-stage rollback failure".to_owned());
            continue;
        }
        if !record.installed {
            continue;
        }
        let result = if let Some(backup) = &record.backup {
            transaction.rename(backup, &destination.parent, &destination.leaf)
        } else {
            destination.parent.remove_file(&destination.leaf)
        };
        if let Err(error) = result {
            errors.push(error.to_string());
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(errors.join("; "))
    }
}

fn rolled_back(error: impl std::fmt::Display, uri: Option<&str>) -> EmitParseStageOutcome {
    EmitParseStageOutcome::RolledBack {
        failure: failure(error, uri),
    }
}

fn failure(error: impl std::fmt::Display, uri: Option<&str>) -> EmitFailure {
    EmitFailure {
        message: error.to_string(),
        uri: uri.map(str::to_owned),
    }
}

fn validate_leaf(parent: &Dir, leaf: &OsStr) -> anyhow::Result<LeafState> {
    match parent.symlink_metadata(leaf) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            anyhow::bail!("Parse-stage destination is a symbolic link")
        }
        Ok(metadata) if metadata.is_file() => Ok(LeafState::Regular),
        Ok(_) => anyhow::bail!("Parse-stage destination is not a regular file"),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(LeafState::Absent),
        Err(error) => Err(error.into()),
    }
}

enum LeafState {
    Absent,
    Regular,
}

fn open_existing_directory_path(root: &Dir, path: &Path) -> anyhow::Result<Option<Dir>> {
    let mut current = root.try_clone()?;
    for component in normal_components(path)? {
        match current.symlink_metadata(component) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!("Parse-stage directory is a symbolic link")
            }
            Ok(metadata) if metadata.is_dir() => {
                current = current.open_dir_nofollow(component)?;
            }
            Ok(_) => anyhow::bail!("Parse-stage directory component is not a directory"),
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some(current))
}

fn ensure_directory_path(
    root: &Dir,
    path: &Path,
    created_directories: &mut Vec<PathBuf>,
) -> anyhow::Result<Dir> {
    let mut current = root.try_clone()?;
    let mut relative = PathBuf::new();
    for component in normal_components(path)? {
        relative.push(component);
        current = match current.open_dir_nofollow(component) {
            Ok(directory) => directory,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                current.create_dir(component)?;
                created_directories.push(relative.clone());
                current.open_dir_nofollow(component)?
            }
            Err(error) => return Err(error.into()),
        };
    }
    Ok(current)
}

fn normal_components(path: &Path) -> anyhow::Result<Vec<&OsStr>> {
    path.components()
        .map(|component| match component {
            Component::Normal(segment) => Ok(segment),
            _ => Err(anyhow::anyhow!(
                "Parse-stage paths must contain only normal relative components"
            )),
        })
        .collect()
}

fn remove_created_directories(root: &Dir, created_directories: &[PathBuf]) {
    for directory in created_directories.iter().rev() {
        let _ = root.remove_dir(directory);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::parse_gleam;

    fn parsed_module(source: &str) -> ModuleIR {
        parse_gleam("main.gleam", source).expect("parse test module")
    }

    #[test]
    fn hard_linking_a_backup_keeps_the_live_destination_present_until_atomic_replace() {
        let output = tempfile::tempdir().expect("create output");
        std::fs::create_dir(output.path().join("parse")).expect("create parse directory");
        std::fs::write(output.path().join("parse/main.json"), "original")
            .expect("write live destination");
        let root = Dir::open_ambient_dir(output.path(), cap_std::ambient_authority())
            .expect("open output root");
        let transaction = create_transaction(&root).expect("create transaction");
        transaction
            .directory
            .create_dir("backups")
            .expect("create backups");
        let destination = Destination {
            parent: root.open_dir_nofollow("parse").expect("open parse"),
            leaf: OsString::from("main.json"),
            staged: PathBuf::from("staged/main.json"),
            backup: PathBuf::from("backups/0.json"),
            uri: "file:///workspace/src/main.gleam".into(),
        };

        hard_link_destination_to_backup(&transaction.directory, &destination)
            .expect("hard-link backup");

        assert_eq!(
            std::fs::read_to_string(output.path().join("parse/main.json"))
                .expect("live destination remains readable"),
            "original"
        );
        assert_eq!(
            transaction
                .directory
                .read_to_string("backups/0.json")
                .expect("read backup"),
            "original"
        );
        transaction
            .directory
            .remove_open_dir_all()
            .expect("clean transaction");
    }

    #[test]
    fn rollback_failure_retains_backups_and_reports_recovery_path_without_uri() {
        let output = tempfile::tempdir().expect("create output");
        std::fs::create_dir(output.path().join("parse")).expect("create parse directory");
        std::fs::write(output.path().join("parse/first.json"), "original")
            .expect("write live destination");
        let first = parsed_module("pub fn changed() { 1 }");
        let second = parsed_module("");
        let modules = [
            ParseStageModule {
                module_name: "first",
                uri: "file:///workspace/src/first.gleam",
                module: &first,
            },
            ParseStageModule {
                module_name: "second",
                uri: "file:///workspace/src/second.gleam",
                module: &second,
            },
        ];
        let faults = Faults {
            fail_commit_at: Some(1),
            fail_rollback_at: Some(0),
            fail_cleanup: false,
        };

        let outcome = emit_parse_stage_with_faults(output.path(), &modules, &faults);

        let recovery_path = match outcome {
            EmitParseStageOutcome::RecoveryRequired {
                failure,
                recovery_path,
            } => {
                assert!(failure.message.contains("rollback failed"));
                assert!(failure.uri.is_none());
                recovery_path
            }
            other => panic!("expected recovery-required outcome, got {other:?}"),
        };
        assert!(recovery_path.join("backups/parse/first.json").is_file());
        std::fs::remove_dir_all(recovery_path).expect("clean retained test transaction");
    }

    #[test]
    fn cleanup_failure_after_commit_is_a_warning_outcome() {
        let output = tempfile::tempdir().expect("create output");
        let module = parsed_module("");
        let modules = [ParseStageModule {
            module_name: "main",
            uri: "file:///workspace/src/main.gleam",
            module: &module,
        }];
        let faults = Faults {
            fail_cleanup: true,
            ..Faults::default()
        };

        let outcome = emit_parse_stage_with_faults(output.path(), &modules, &faults);

        match outcome {
            EmitParseStageOutcome::Committed {
                cleanup_warning: Some(warning),
            } => assert!(warning.contains("output was committed")),
            other => panic!("expected committed warning outcome, got {other:?}"),
        }
        assert!(output.path().join("parse/main.json").is_file());
        for entry in std::fs::read_dir(output.path()).expect("read output") {
            let entry = entry.expect("read entry");
            if entry.file_name().to_string_lossy().starts_with(".morphir") {
                std::fs::remove_dir_all(entry.path()).expect("clean retained test transaction");
            }
        }
    }

    #[test]
    fn transaction_cleanup_failure_after_rollback_omits_source_uri() {
        let output = tempfile::tempdir().expect("create output");
        let module = parsed_module("");
        let modules = [ParseStageModule {
            module_name: "main",
            uri: "file:///workspace/src/main.gleam",
            module: &module,
        }];
        let faults = Faults {
            fail_commit_at: Some(0),
            fail_cleanup: true,
            ..Faults::default()
        };

        let outcome = emit_parse_stage_with_faults(output.path(), &modules, &faults);

        match outcome {
            EmitParseStageOutcome::RolledBack { failure } => {
                assert!(failure.message.contains("failed to clean"));
                assert!(failure.uri.is_none());
            }
            other => panic!("expected rolled-back outcome, got {other:?}"),
        }
        for entry in std::fs::read_dir(output.path()).expect("read output") {
            let entry = entry.expect("read entry");
            if entry.file_name().to_string_lossy().starts_with(".morphir") {
                std::fs::remove_dir_all(entry.path()).expect("clean retained test transaction");
            }
        }
    }
}
