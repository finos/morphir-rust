use super::super::declarations::{Status, custom, reference};
use super::*;
use crate::resolution::PackagePath;
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct Entry {
    pub reference: ObjectReference,
    pub record: RegistryRecord,
    pub status: Status,
}
pub(super) struct Catalog {
    entries: BTreeMap<ReleaseId, Entry>,
}
impl Catalog {
    pub fn selected(&self, release: &ReleaseId) -> Result<&Entry, Error> {
        self.entries.get(release).ok_or(Error::Refused(
            "published root is unavailable for new selection",
        ))
    }
    pub fn active(&self) -> BTreeSet<ReleaseId> {
        self.entries
            .iter()
            .filter(|(_, e)| e.status == Status::Active)
            .map(|(id, _)| id.clone())
            .collect()
    }
    pub fn update_input(
        &self,
        old: &LibraryLock,
        targets: &[resolution::UpdateTarget],
    ) -> Result<Value, Error> {
        for acquisition in old.acquisitions() {
            let entry = self
                .entries
                .get(acquisition.release())
                .ok_or(Error::Refused("old release absent from current repository"))?;
            let statement = old
                .evidence()
                .iter()
                .find(|e| e.id() == acquisition.statement())
                .ok_or(Error::Refused("missing publisher evidence"))?;
            require(
                entry.reference == *acquisition.record()
                    && entry.record.source() == acquisition.source()
                    && entry.record.statement() == statement.reference(),
                "old record acquisition mismatch",
            )?;
        }
        let root = self
            .entries
            .get(old.graph().root())
            .ok_or(Error::Refused("old root absent from current repository"))?
            .record
            .release_record();
        let mut catalogs: BTreeMap<&PackagePath, Vec<_>> = BTreeMap::new();
        for entry in self.entries.values() {
            let record = entry.record.release_record();
            let releases = catalogs.entry(record.release().package_path()).or_default();
            if record.release() != root.release() {
                releases.push(record);
            }
            for dependency in record.dependencies() {
                catalogs.entry(dependency.package_path()).or_default();
            }
        }
        let catalogs: Vec<_> = catalogs
            .into_iter()
            .map(|(path, releases)| json!({"packagePath":path,"releases":releases}))
            .collect();
        Ok(
            json!({"formatVersion":"0.1.0-draft.2","capability":"flat-library","mode":"update","root":root,"catalogs":catalogs,"lock":old.graph(),"targets":targets}),
        )
    }
    pub fn resolution_input(&self, root: &ReleaseId) -> Result<Value, Error> {
        let entry = self.selected(root)?;
        require(
            entry.status == Status::Active,
            "published root is unavailable for new selection",
        )?;
        let root = entry.record.release_record();
        let mut catalogs: BTreeMap<&PackagePath, Vec<_>> = BTreeMap::new();
        for entry in self.entries.values() {
            let record = entry.record.release_record();
            let releases = catalogs.entry(record.release().package_path()).or_default();
            if entry.status == Status::Active && record.release() != root.release() {
                releases.push(record);
            }
            for dependency in record.dependencies() {
                catalogs.entry(dependency.package_path()).or_default();
            }
        }
        let catalogs: Vec<_> = catalogs
            .into_iter()
            .map(|(path, releases)| json!({"packagePath":path,"releases":releases}))
            .collect();
        Ok(
            json!({"formatVersion":"0.1.0-draft.2","capability":"flat-library","mode":"initial","root":root,"catalogs":catalogs}),
        )
    }
}

/// Validate the entire bounded package target view before filtering candidates.
/// TUF extension fields outside custom.morphir remain untouched.
pub(super) fn read(
    backend: &store::Backend,
    registry: &Path,
    policy: &TrustPolicy,
    targets: &Value,
) -> Result<Catalog, Error> {
    let targets_map = targets.as_object().ok_or(Error::Refused("targets map"))?;
    let mut entries = BTreeMap::<ReleaseId, Entry>::new();
    let mut statements = BTreeMap::new();
    for (path, target) in targets_map {
        let reference = reference(path, target)?;
        if path.starts_with("statements/") {
            statements.insert(
                path.as_str(),
                (custom(target, "LibraryReleaseStatement", 3)?, reference),
            );
            continue;
        }
        require(
            path.starts_with("records/"),
            "unsupported package target kind",
        )?;
        require(entries.len() < 4096, "catalog release limit")?;
        let release = custom(target, "LibraryRelease", 4)?;
        let status = Status::parse(target)?;
        let bytes = verify::target(
            registry,
            reference.path(),
            reference.digest(),
            targets,
            &release,
            verify::TargetAuthorization::Declaration,
        )?;
        backend.charge(bytes.len())?;
        let subject = Subject::Object {
            registry: LocalId::parse("local").unwrap(),
            path: reference.path().clone(),
        };
        let record = decode_registry_record(&bytes, &subject)?;
        require(
            record.release_record().release() == &release,
            "target custom release differs from record",
        )?;
        if let Some(previous) = entries.get(&release) {
            require(
                previous.record.release_record() == record.release_record(),
                "conflicting catalog release",
            )?;
            return Err(Error::Refused("duplicate catalog release"));
        }
        entries.insert(
            release,
            Entry {
                reference,
                record,
                status,
            },
        );
    }
    let mut used = BTreeSet::new();
    for (release, entry) in &entries {
        let statement = entry.record.statement();
        let (identity, reference) = statements
            .get(statement.path().as_str())
            .ok_or(Error::Refused("missing statement target"))?;
        require(
            identity == release && reference == statement,
            "statement target differs from record",
        )?;
        require(
            used.insert(statement.path().as_str()),
            "duplicate statement reference",
        )?;
    }
    require(used.len() == statements.len(), "orphan statement target")?;
    // The MVP has no durable revocation ledger. Refuse the whole operation and
    // preserve its marker, even for an unselected revoked record; a later active
    // assertion must not erase that observation. Full transitions are deferred.
    require(
        !entries
            .values()
            .any(|entry| entry.status == Status::Revoked),
        "revocation transition unsupported by MVP; manual intervention required",
    )?;
    entries.retain(|release, _| {
        repository_permits(
            policy,
            policy.repositories()[0].identity(),
            release.package_path(),
        )
    });
    Ok(Catalog { entries })
}
