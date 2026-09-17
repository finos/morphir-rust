mod identity;
mod shape;

use super::model::*;
use serde_json::Value;

pub(super) use identity::outer_identities;
pub(super) use shape::outer_shape;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Initial,
    Update,
    Replay,
}

#[derive(Debug, Clone)]
pub(super) struct LocatedRecord {
    pub(super) value: ReleaseRecord,
    pub(super) pointer: String,
    pub(super) suppressed: bool,
}

#[derive(Debug, Clone)]
pub(super) struct LocatedCatalog {
    pub(super) value: Catalog,
    pub(super) records: Vec<LocatedRecord>,
    pub(super) pointer: String,
    pub(super) suppressed: bool,
}

#[derive(Debug, Clone)]
pub(super) struct LocatedTarget {
    pub(super) value: UpdateTarget,
    pub(super) pointer: String,
}

#[derive(Debug, Clone)]
pub(super) struct OuterInput {
    pub(super) mode: Mode,
    pub(super) root: LocatedRecord,
    pub(super) catalogs: Vec<LocatedCatalog>,
    pub(super) releases: Vec<LocatedRecord>,
    pub(super) targets: Vec<LocatedTarget>,
    pub(super) raw_lock: Option<Value>,
}

pub(super) fn normalize(mut violations: Vec<Violation>) -> Vec<Violation> {
    violations.sort_by(|left, right| {
        left.pointer
            .cmp(&right.pointer)
            .then_with(|| left.rule.as_str().cmp(right.rule.as_str()))
    });
    violations.dedup();
    violations
}
