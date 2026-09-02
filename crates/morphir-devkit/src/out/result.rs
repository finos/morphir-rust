//! Result record written beside a task's `.dest` directory.

use super::TaskId;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Current record schema version.
pub const RESULT_SCHEMA: u32 = 1;

/// How an IR artifact is stored inside `.dest`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IrLayout {
    /// One file holds the whole distribution.
    SingleFile,
    /// A directory tree of logical documents with a `manifest.*` root.
    DocumentTree,
}

/// Where and how an IR-producing task stored its IR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrDescriptor {
    /// Path relative to `.dest`. A file for `single-file`, a directory for
    /// `document-tree`.
    pub path: String,
    /// Storage layout.
    pub layout: IrLayout,
    /// Serialization format: `json` or `yaml`.
    pub format: String,
    /// IR version: `v3` or `v4`.
    pub version: String,
}

/// Result record of one task run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskResult {
    /// Record schema version.
    pub schema: u32,
    /// Task id, for example `generate/scala`.
    pub task: String,
    /// Module path relative to the workspace root. Empty for the root module.
    pub module: String,
    /// Source language for compile records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Task ids this task consumed.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Paths relative to `.dest` that are this task's product.
    #[serde(default)]
    pub value: Vec<String>,
    /// IR storage descriptor for IR-producing tasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ir: Option<IrDescriptor>,
    /// Absolute eject target to the `value` entries copied there last time.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ejected: BTreeMap<String, Vec<String>>,
    /// RFC 3339 UTC timestamp of the successful run.
    pub completed_at: String,
    /// Fields this version does not know. Preserved on read and write.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl TaskResult {
    /// A fresh record for `task` in `module`, stamped with the current time.
    pub fn new(task: &TaskId, module: &Path) -> Self {
        Self {
            schema: RESULT_SCHEMA,
            task: task.as_str().to_owned(),
            module: module.to_string_lossy().replace('\\', "/"),
            language: None,
            inputs: Vec::new(),
            value: Vec::new(),
            ir: None,
            ejected: BTreeMap::new(),
            completed_at: now_rfc3339(),
            extra: BTreeMap::new(),
        }
    }

    /// Read a record. `Ok(None)` when the file does not exist.
    pub fn read(path: &Path) -> anyhow::Result<Option<Self>> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(error).with_context(|| format!("read {}", path.display()));
            }
        };
        let record = serde_json::from_slice(&bytes)
            .with_context(|| format!("decode task result {}", path.display()))?;
        Ok(Some(record))
    }

    /// Write the record through a temporary file and rename, creating parents.
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        let parent = path
            .parent()
            .with_context(|| format!("{} has no parent directory", path.display()))?;
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .with_context(|| format!("create temporary file in {}", parent.display()))?;
        let bytes = serde_json::to_vec_pretty(self).context("encode task result")?;
        temp.write_all(&bytes)
            .and_then(|()| temp.as_file().sync_all())
            .with_context(|| format!("write {}", path.display()))?;
        temp.persist(path)
            .map(|_| ())
            .with_context(|| format!("publish {}", path.display()))
    }
}

/// Current UTC time as RFC 3339 with second precision.
pub fn now_rfc3339() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format_rfc3339(seconds)
}

fn format_rfc3339(unix_seconds: u64) -> String {
    let days = unix_seconds / 86_400;
    let remainder = unix_seconds % 86_400;
    let (hour, minute, second) = (remainder / 3600, (remainder % 3600) / 60, remainder % 60);
    // Civil-from-days, Howard Hinnant's algorithm.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_new_record_has_schema_one_and_a_timestamp() {
        let record = TaskResult::new(&TaskId::compile(), Path::new("packages/orders"));
        assert_eq!(record.schema, RESULT_SCHEMA);
        assert_eq!(record.task, "compile");
        assert_eq!(record.module, "packages/orders");
        assert!(record.completed_at.ends_with('Z'));
        assert_eq!(record.completed_at.len(), 20, "{}", record.completed_at);
    }

    #[test]
    fn records_round_trip_and_keep_unknown_fields() {
        let json = r#"{
          "schema": 1,
          "task": "compile",
          "module": "",
          "language": "gleam",
          "inputs": [],
          "value": ["morphir-ir.json"],
          "ir": {"path": "morphir-ir.json", "layout": "single-file", "format": "json", "version": "v4"},
          "ejected": {"/abs/dist": ["morphir-ir.json"]},
          "completedAt": "2026-09-02T10:00:00Z",
          "inputsHash": "sha256:abc"
        }"#;
        let record: TaskResult = serde_json::from_str(json).unwrap();
        assert_eq!(record.ir.as_ref().unwrap().layout, IrLayout::SingleFile);
        assert_eq!(record.extra["inputsHash"], "sha256:abc");
        let out = serde_json::to_value(&record).unwrap();
        assert_eq!(out["inputsHash"], "sha256:abc");
        assert_eq!(out["ir"]["layout"], "single-file");
        assert_eq!(out["completedAt"], "2026-09-02T10:00:00Z");
    }

    #[test]
    fn document_tree_layout_serializes_with_a_dash() {
        let descriptor = IrDescriptor {
            path: "morphir-ir/".into(),
            layout: IrLayout::DocumentTree,
            format: "yaml".into(),
            version: "v4".into(),
        };
        assert_eq!(
            serde_json::to_value(&descriptor).unwrap()["layout"],
            "document-tree"
        );
    }

    #[test]
    fn read_returns_none_for_a_missing_file_and_write_creates_parents() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested").join("compile.json");
        assert!(TaskResult::read(&path).unwrap().is_none());
        let mut record = TaskResult::new(&TaskId::compile(), Path::new(""));
        record.value.push("morphir-ir.json".into());
        record.write(&path).unwrap();
        let read = TaskResult::read(&path).unwrap().unwrap();
        assert_eq!(read.value, vec!["morphir-ir.json".to_owned()]);
        assert!(
            temp.path()
                .join("nested")
                .read_dir()
                .unwrap()
                .all(|entry| { entry.unwrap().file_name() == "compile.json" }),
            "temp file left behind"
        );
    }

    #[test]
    fn rfc3339_formatter_matches_known_instants() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_rfc3339(1_788_688_800), "2026-09-06T10:00:00Z");
    }
}
