use super::{
    Draft, Error, Outcome, Predecessor, Proposal, PublicationResult, digest, filesystem::Directory,
    verify,
};
use crate::{
    authoring::AuthoredLibrary,
    local_registry::{
        tuf::{self, ProfileMetadata},
        *,
    },
};
use package_tough::schema::Version;
use serde_json::{Value, json};
use std::{fs::File, io, path::Path};

/// One exclusive local APFS publisher session. Dropping it releases the OS lock.
/// Signing keys never enter this type.
pub struct Registry {
    pub(super) root: Directory,
    state: Directory,
    pub(super) metadata: Directory,
    pub(super) policy: TrustPolicy,
    pub(super) authority: ProfileMetadata,
    _lock: File,
    operation: std::sync::Mutex<()>,
}
pub(super) struct View {
    pub(super) predecessor: Predecessor,
    pub(super) targets: Value,
}
impl Registry {
    /// Explicitly initialize absent registry and protected state directories.
    /// Both parents must exist on local APFS. Existing directories are never reset.
    pub fn initialize(
        registry: &Path,
        state: &Path,
        policy: &[u8],
        root: &[u8],
    ) -> Result<(), Error> {
        let policy = verify::policy(policy)?;
        let authority = verify::root(&policy, root, jiff::Timestamp::now())?;
        let registry = Directory::create(registry)?;
        let state = Directory::create(state)?;
        let _lock = registry.lock()?;
        registry.mkdir("metadata")?;
        registry.mkdir("bundles")?;
        registry.mkdir("targets")?;
        registry.mkdir(".staging")?;
        let metadata = registry.child("metadata")?;
        metadata.install(&format!("{}.root.json", verify::version(&authority)), root)?;
        let binding =
            json!({"registry":registry.identity()?,"state":state.identity()?,"root":digest(root)});
        let binding =
            serde_json::to_vec(&binding).map_err(|_| Error::Invalid("publisher state"))?;
        state.install("identity.json", &binding)?;
        registry.install(".publisher-state", &binding)?;
        Ok(())
    }
    /// Open established state, acquire the stable writer lock and recheck authority.
    pub fn open(registry: &Path, state: &Path, policy: &[u8]) -> Result<Self, Error> {
        let root = Directory::open(registry)?;
        let state = Directory::open(state)?;
        let lock = root.lock()?;
        let policy = verify::policy(policy)?;
        let metadata = root.child("metadata")?;
        let authority_bytes = metadata.read(
            &format!(
                "{}.root.json",
                policy.repositories()[0].bootstrap_root().version()
            ),
            1_048_576,
        )?;
        let authority = verify::root(&policy, &authority_bytes, jiff::Timestamp::now())?;
        let binding = state.read("identity.json", 4096)?;
        if root.read(".publisher-state", 4096)? != binding {
            return Err(Error::Invalid("publisher state binding"));
        }
        let binding: Value = serde_json::from_slice(&binding)
            .map_err(|_| Error::Invalid("publisher state binding"))?;
        if binding
            != json!({"registry":root.identity()?,"state":state.identity()?,"root":digest(&authority_bytes)})
        {
            return Err(Error::Invalid("publisher state binding"));
        }
        let registry = Self {
            root,
            state,
            metadata,
            policy,
            authority,
            _lock: lock,
            operation: std::sync::Mutex::new(()),
        };
        registry.floors()?;
        registry.current(jiff::Timestamp::now())?;
        Ok(registry)
    }
    /// Prepare a successor without reserving versions. Signing is a separate step.
    pub fn prepare(
        &self,
        library: &AuthoredLibrary,
        record: &[u8],
        envelope: &[u8],
        expires: &str,
    ) -> Result<Draft, Error> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| Error::Invalid("publisher operation poisoned"))?;
        let record_model = verify::release(library, record, envelope, &self.policy)?;
        let now = jiff::Timestamp::now();
        let expires_time = expires
            .parse::<jiff::Timestamp>()
            .map_err(|_| Error::Invalid("expiry"))?;
        if expires_time <= now {
            return Err(Error::Invalid("metadata-expired"));
        }
        let view = self.current(now)?;
        let existing = self.existing(&view, &record_model, record, envelope)?;
        let floors = self.floors()?;
        let mut targets = view.targets;
        for (path, pin) in verify::targets(&record_model, record, envelope)
            .as_object()
            .expect("object")
        {
            if existing {
                continue;
            }
            if let Some(previous) = targets.get(path)
                && previous != pin
            {
                return Err(Error::ReleaseConflict);
            }
            targets[path] = pin.clone();
        }
        let next = |v: &Option<Version>| {
            v.as_ref()
                .map_or_else(|| Version::new(1).expect("one"), Version::successor)
        };
        Ok(Draft {
            predecessor: view.predecessor,
            targets: json!({"_type":"targets","spec_version":"1.0.36",
            "version":next(&floors[0]),"expires":expires,"targets":targets}),
            snapshot_version: next(&floors[1]),
            timestamp_version: next(&floors[2]),
        })
    }
    /// Verify and durably install exact caller-signed bytes against an exact base.
    pub fn publish(
        &self,
        library: &AuthoredLibrary,
        record: &[u8],
        envelope: &[u8],
        predecessor: &Predecessor,
        proposal: &Proposal,
    ) -> Result<PublicationResult, Error> {
        let _operation = self
            .operation
            .lock()
            .map_err(|_| Error::Invalid("publisher operation poisoned"))?;
        let record_model = verify::release(library, record, envelope, &self.policy)?;
        let now = jiff::Timestamp::now();
        let view = self.current(now)?;
        if &view.predecessor != predecessor {
            return Err(Error::Conflict {
                expected: predecessor.clone(),
                observed: view.predecessor,
            });
        }
        if self.existing(&view, &record_model, record, envelope)? {
            let Predecessor::Timestamp { digest: timestamp } = view.predecessor else {
                return Err(Error::Invalid("release in empty view"));
            };
            return Ok(PublicationResult {
                outcome: Outcome::Idempotent,
                timestamp,
            });
        }
        let roles = self.proposal(proposal, &view, &record_model, record, envelope, now)?;
        let versions = roles.each_ref().map(verify::version);
        let floors = self.floors()?;
        for (i, role) in ["targets", "snapshot", "timestamp"].into_iter().enumerate() {
            if floors[i]
                .as_ref()
                .is_some_and(|floor| &versions[i] <= floor)
            {
                return Err(Error::Rollback {
                    role,
                    floor: floors[i].as_ref().expect("checked floor").to_string(),
                    received: versions[i].to_string(),
                });
            }
        }
        for (i, role) in ["targets", "snapshot", "timestamp"].into_iter().enumerate() {
            RegistryPath::parse(&format!("metadata/{}.{role}.json", versions[i]))
                .map_err(|_| Error::Invalid("metadata physical path limit"))?;
        }
        let reservation_bytes =
            serde_json::to_vec(&versions).map_err(|_| Error::Invalid("reservation"))?;
        let transaction = digest(&reservation_bytes)[7..].to_owned();
        self.state.install(
            &format!("reservation-{transaction}.json"),
            &reservation_bytes,
        )?;
        #[cfg(test)]
        super::checkpoint(super::FaultPoint::Reserved)?;
        #[cfg(test)]
        super::crash_tests::after_reservation();
        self.install_bundle(library, &transaction)?;
        let staging = self.root.child(".staging")?;
        let stage_name = format!("objects-{transaction}");
        staging.mkdir(&stage_name)?;
        let staging = staging.child(&stage_name)?;
        self.root.install_path(
            &verify::physical(&verify::record_path(&record_model), record)?,
            record,
            &staging,
        )?;
        self.root.install_path(
            &verify::physical(record_model.statement().path().as_str(), envelope)?,
            envelope,
            &staging,
        )?;
        for (i, role) in ["targets", "snapshot", "timestamp"].into_iter().enumerate() {
            self.install_exact(&format!("{}.{}.json", versions[i], role), roles[i].bytes())?;
        }
        let prepared = format!(".timestamp-{transaction}");
        self.metadata.install(&prepared, &proposal.timestamp)?;
        #[cfg(test)]
        super::checkpoint(super::FaultPoint::BeforeTimestamp)?;
        self.metadata.replace(&prepared, "timestamp.json")?;
        #[cfg(test)]
        super::checkpoint(super::FaultPoint::AfterTimestamp)?;
        #[cfg(test)]
        super::checkpoint(super::FaultPoint::BeforeFinalFlush)?;
        self.metadata
            .flush()
            .map_err(|_| Error::CommitOutcomeUncertain)?;
        self.state
            .install(
                &format!("commit-{}.json", versions[2]),
                digest(&proposal.timestamp).as_bytes(),
            )
            .map_err(|_| Error::CommitOutcomeUncertain)?;
        Ok(PublicationResult {
            outcome: Outcome::Committed,
            timestamp: digest(&proposal.timestamp),
        })
    }
    fn install_exact(&self, name: &str, bytes: &[u8]) -> Result<(), Error> {
        match self.metadata.install(name, bytes) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                if self.metadata.read(name, 16 * 1024 * 1024)? == bytes {
                    Ok(())
                } else {
                    Err(Error::Invalid("immutable-object-conflict"))
                }
            }
            Err(e) => Err(e.into()),
        }
    }
    fn install_bundle(&self, library: &AuthoredLibrary, transaction: &str) -> Result<(), Error> {
        let staging = self.root.child(".staging")?;
        let bundles = self.root.child("bundles")?;
        let content = library.metadata().content_digest().to_string();
        let name = &content[7..];
        match bundles.child(name) {
            Ok(existing) => return verify_bundle(&existing, library),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let stage = format!("bundle-{transaction}");
        staging.mkdir(&stage)?;
        let directory = staging.child(&stage)?;
        directory.install("manifest.json", library.manifest_bytes())?;
        directory.install("ir.json", library.ir_bytes())?;
        directory.flush()?;
        staging.promote(&stage, &bundles, name)?;
        Ok(())
    }
    fn proposal(
        &self,
        proposal: &Proposal,
        view: &View,
        record: &RegistryRecord,
        record_bytes: &[u8],
        envelope: &[u8],
        now: jiff::Timestamp,
    ) -> Result<[ProfileMetadata; 3], Error> {
        let targets = verify::role(&self.authority, &proposal.targets, TufRole::Targets, now)?;
        let snapshot = verify::role(&self.authority, &proposal.snapshot, TufRole::Snapshot, now)?;
        let timestamp = verify::role(
            &self.authority,
            &proposal.timestamp,
            TufRole::Timestamp,
            now,
        )?;
        tuf::verify_link(&timestamp, &snapshot)?;
        tuf::verify_link(&snapshot, &targets)?;
        let mut expected = view.targets.clone();
        for (path, pin) in verify::targets(record, record_bytes, envelope)
            .as_object()
            .expect("object")
        {
            if expected.get(path).is_some() {
                return Err(Error::Invalid("target-link-mismatch"));
            }
            expected[path] = pin.clone();
        }
        if targets.document()["signed"]["targets"] != expected {
            return Err(Error::Invalid("cross-document-mismatch"));
        }
        Ok([targets, snapshot, timestamp])
    }
    fn floors(&self) -> Result<[Option<Version>; 3], Error> {
        let mut floors: [Option<Version>; 3] = [None, None, None];
        for name in self.state.names()? {
            if name == "identity.json" || name.starts_with("commit-") {
                continue;
            }
            let bytes = self.state.read(&name, 4096)?;
            let versions: [Version; 3] = serde_json::from_slice(&bytes)
                .map_err(|_| Error::Invalid("corrupt publisher reservation"))?;
            if name != format!("reservation-{}.json", &digest(&bytes)[7..]) {
                return Err(Error::Invalid("corrupt publisher reservation"));
            }
            for (i, version) in versions.into_iter().enumerate() {
                floors[i] = floors[i].take().max(Some(version));
            }
        }
        for name in self.metadata.names()? {
            for (i, role) in ["targets", "snapshot", "timestamp"].into_iter().enumerate() {
                if let Some(version) = name.strip_suffix(&format!(".{role}.json")) {
                    let version = version
                        .parse::<Version>()
                        .map_err(|_| Error::Invalid("invalid numbered metadata"))?;
                    floors[i] = floors[i].take().max(Some(version));
                }
            }
        }
        if let Some((version, _)) = self.committed()? {
            floors[2] = floors[2].take().max(Some(version));
        }
        Ok(floors)
    }
    pub(super) fn committed(&self) -> Result<Option<(Version, String)>, Error> {
        let mut highest = None;
        for name in self.state.names()? {
            let Some(version) = name.strip_prefix("commit-") else {
                continue;
            };
            let version = version
                .strip_suffix(".json")
                .and_then(|v| v.parse::<Version>().ok())
                .ok_or(Error::Invalid("corrupt commit receipt"))?;
            let bytes = self.state.read(&name, 71)?;
            let digest = std::str::from_utf8(&bytes)
                .map_err(|_| Error::Invalid("corrupt commit receipt"))?;
            Digest::parse(digest).map_err(|_| Error::Invalid("corrupt commit receipt"))?;
            if highest
                .as_ref()
                .is_none_or(|(previous, _)| version > *previous)
            {
                highest = Some((version, digest.to_owned()));
            }
        }
        Ok(highest)
    }
}
fn verify_bundle(directory: &Directory, library: &AuthoredLibrary) -> Result<(), Error> {
    if directory.names()? != vec!["ir.json".to_owned(), "manifest.json".to_owned()] {
        return Err(Error::Invalid("immutable-object-conflict"));
    }
    if directory.read("manifest.json", 1_048_576)? != library.manifest_bytes()
        || directory.read("ir.json", 64 * 1024 * 1024)? != library.ir_bytes()
    {
        return Err(Error::Invalid("immutable-object-conflict"));
    }
    Ok(())
}
