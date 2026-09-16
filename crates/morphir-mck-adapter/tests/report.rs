//! Holds a Morphir Compatibility Kit run against the list of cases this binding is allowed to
//! fail.
//!
//! This test does not run the kit — `mise run check:kit` does, and writes its report to
//! `.dev/out/mck/report.json`. What happens here is the adjudication: the set of case ids with
//! any failing record has to equal the `cases` array in `allowed-failing.json`, in both
//! directions. A new failure is a regression; a listed case that has started passing is a stale
//! entry, and leaving it in would hide the next real failure behind it.
//!
//! When the report is absent the test prints a note and passes, so `cargo test --workspace` is
//! still runnable without a kit driver on the machine. The kit task runs the driver first, so in
//! CI the report is always there.
//!
//! The report's shape is `report.schema.json` in the kit (`spec/ir/mck` in finos/morphir),
//! contract version 1: a `records` array of `{ caseId, irVersion, profile, role, fenceIndex,
//! path?, result, durationMs, message? }`, where `result` is one of `pass`, `fail`, `skipped`
//! and `kit-error`. One case has many records — one per fence per path — so a case is failing if
//! any of its records is.
//!
//! A run with nothing failing is not the same as a run that proved anything, so two further
//! things are checked. At least one record has to have passed — a report of nothing but skips
//! would otherwise satisfy an empty allow-list. And every skip has to be one the binding asked
//! for: the driver skips a fence only for a capability the adapter did not declare or for a case
//! the kit marks pending, and nothing else is a legitimate reason for a fence not to have been
//! run.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Where `mise run check:kit` writes the driver's report, relative to the repository root.
const REPORT: &str = ".dev/out/mck/report.json";

/// The list this crate owns: the cases a kit defect is open against.
const ALLOWED: &str = "allowed-failing.json";

/// The only reasons the driver skips a fence, taken from its own `unsupported` and the pending
/// check in `packages/mck/src/driver/run.ts`. Every message it writes for a skipped record is
/// either the exact string `pending` — the kit marked the case not ready — or one of
/// `node|version|layout|profile|path <x> not in capabilities`, which is the driver declining to
/// ask a binding for something the binding said it does not do. A skip with any other message
/// means a fence went unrun for a reason nobody chose, and that is a hole in the gate.
const PENDING: &str = "pending";
const UNDECLARED: &str = "not in capabilities";

#[test]
fn the_kit_fails_exactly_the_cases_the_list_allows() {
    let allowed = read_allowed();

    let report_path = repository_root().join(REPORT);
    let Ok(text) = std::fs::read_to_string(&report_path) else {
        println!(
            "no kit report at {}; run `mise run check:kit` to produce one. \
             Skipping the adjudication.",
            report_path.display()
        );
        return;
    };

    let report: serde_json::Value =
        serde_json::from_str(&text).expect("the kit report is JSON (report.schema.json)");
    let records = report["records"]
        .as_array()
        .expect("the kit report has a records array");
    assert!(
        !records.is_empty(),
        "the kit report at {} has no records at all, which means the driver never ran a case",
        report_path.display()
    );

    // An empty allow-list is only worth something if something was actually decoded. Without
    // this, a run that skipped every fence — a capabilities answer that went wrong, say — would
    // sail through the adjudication below.
    let passed = records
        .iter()
        .filter(|record| record["result"].as_str() == Some("pass"))
        .count();
    assert!(
        passed > 0,
        "the kit report at {} has no passing record, so nothing was proved by this run",
        report_path.display()
    );

    // Every skip has to be one the binding asked for by not declaring a capability, or one the
    // kit asked for by marking the case pending.
    let unexplained: BTreeMap<String, String> = records
        .iter()
        .filter(|record| record["result"].as_str() == Some("skipped"))
        .filter_map(|record| {
            let message = record["message"].as_str().unwrap_or("");
            (message != PENDING && !message.contains(UNDECLARED)).then(|| {
                (
                    record["caseId"].as_str().unwrap_or("?").to_string(),
                    message.to_string(),
                )
            })
        })
        .collect();
    assert!(
        unexplained.is_empty(),
        "a fence was skipped for a reason that is neither an undeclared capability \
         ({UNDECLARED:?}) nor a pending case ({PENDING:?}): {unexplained:?}"
    );

    let failing: BTreeSet<String> = records
        .iter()
        .filter(|record| matches!(record["result"].as_str(), Some("fail") | Some("kit-error")))
        .map(|record| {
            record["caseId"]
                .as_str()
                .expect("every record names its case")
                .to_string()
        })
        .collect();

    let regressions: Vec<&String> = failing.difference(&allowed).collect();
    let stale: Vec<&String> = allowed.difference(&failing).collect();

    assert!(
        regressions.is_empty() && stale.is_empty(),
        "the kit run does not match {ALLOWED}.\n\
         failing but not listed (a regression, fix the codec): {regressions:?}\n\
         listed but now passing (take it out of the list): {stale:?}\n\
         report: {}",
        report_path.display()
    );
}

fn read_allowed() -> BTreeSet<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(ALLOWED);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
    let value: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parsing {}: {error}", path.display()));
    value["cases"]
        .as_array()
        .unwrap_or_else(|| panic!("{} has no cases array", path.display()))
        .iter()
        .map(|case| {
            case.as_str()
                .unwrap_or_else(|| panic!("{} lists case ids as strings", path.display()))
                .to_string()
        })
        .collect()
}

/// The repository root: two levels up from this crate, which lives at `crates/<name>`.
fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the crate sits at crates/<name> under the repository root")
}
