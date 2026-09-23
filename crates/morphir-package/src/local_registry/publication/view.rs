use super::{Error, Predecessor, Registry, digest, registry::View, verify};
use crate::local_registry::{tuf, *};
use serde_json::{Value, json};
use std::io;
impl Registry {
    pub(super) fn current(&self, now: jiff::Timestamp) -> Result<View, Error> {
        verify::fresh(&self.authority, now)?;
        let committed = self.committed()?;
        let timestamp = match self.metadata.read("timestamp.json", 1_048_576) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if committed.is_some() {
                    return Err(Error::Invalid("missing committed timestamp"));
                }
                return Ok(View {
                    predecessor: Predecessor::Absent,
                    targets: json!({}),
                });
            }
            Err(e) => return Err(e.into()),
        };
        verify::fresh(&self.authority, now)?;
        let timestamp_model = verify::role(&self.authority, &timestamp, TufRole::Timestamp, now)?;
        if let Some((version, expected)) = committed {
            let observed = verify::version(&timestamp_model);
            if observed < version || (observed == version && digest(&timestamp) != expected) {
                return Err(Error::Invalid("committed timestamp rollback"));
            }
        }
        let snapshot_version =
            timestamp_model.document()["signed"]["meta"]["snapshot.json"]["version"]
                .to_string()
                .parse::<package_tough::schema::Version>()
                .map_err(|_| Error::Invalid("snapshot version"))?;
        let snapshot = verify::role(
            &self.authority,
            &self.metadata.read(
                &format!("{snapshot_version}.snapshot.json"),
                16 * 1024 * 1024,
            )?,
            TufRole::Snapshot,
            now,
        )?;
        tuf::verify_link(&timestamp_model, &snapshot)?;
        let target_version = snapshot.document()["signed"]["meta"]["targets.json"]["version"]
            .to_string()
            .parse::<package_tough::schema::Version>()
            .map_err(|_| Error::Invalid("targets version"))?;
        let targets = verify::role(
            &self.authority,
            &self
                .metadata
                .read(&format!("{target_version}.targets.json"), 16 * 1024 * 1024)?,
            TufRole::Targets,
            now,
        )?;
        tuf::verify_link(&snapshot, &targets)?;
        let targets = targets.document()["signed"]["targets"].clone();
        self.validate_targets(&targets)?;
        Ok(View {
            predecessor: Predecessor::Timestamp {
                digest: digest(&timestamp),
            },
            targets,
        })
    }
    fn target(&self, path: &str, pin: &Value) -> Result<Vec<u8>, Error> {
        RegistryPath::parse(path).map_err(|_| Error::Invalid("target path"))?;
        let hash = pin["hashes"]["sha256"]
            .as_str()
            .ok_or(Error::Invalid("target digest"))?;
        let (parent, name) = path.rsplit_once('/').ok_or(Error::Invalid("target path"))?;
        let bytes = self
            .root
            .read_path(&format!("targets/{parent}/{hash}.{name}"), 1_048_576)?;
        if pin["length"].as_u64() != Some(bytes.len() as u64) || &digest(&bytes)[7..] != hash {
            return Err(Error::Invalid("digest-mismatch"));
        }
        if let Some(expected) = pin["hashes"].get("sha512") {
            use sha2::Digest as _;
            let actual: String = sha2::Sha512::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            if expected.as_str() != Some(&actual) {
                return Err(Error::Invalid("digest-mismatch"));
            }
        }
        Ok(bytes)
    }
    pub(super) fn existing(
        &self,
        view: &View,
        record: &RegistryRecord,
        bytes: &[u8],
        envelope: &[u8],
    ) -> Result<bool, Error> {
        for (path, pin) in view.targets.as_object().ok_or(Error::Invalid("targets"))? {
            if !path.starts_with("records/") {
                continue;
            }
            if pin["custom"]["morphir"]["release"]
                != serde_json::to_value(record.release_record().release())
                    .map_err(|_| Error::Invalid("release"))?
            {
                continue;
            }
            let existing = self.target(path, pin)?;
            let subject = Subject::Lock;
            let previous = decode_registry_record(&existing, &subject)?;
            if previous.release_record().manifest_digest()
                != record.release_record().manifest_digest()
                || previous.release_record().content_digest()
                    != record.release_record().content_digest()
            {
                return Err(Error::ReleaseConflict);
            }
            if existing != bytes
                || self.target(
                    previous.statement().path().as_str(),
                    &view.targets[previous.statement().path().as_str()],
                )? != envelope
            {
                return Err(Error::RecordReplacement);
            }
            return Ok(true);
        }
        Ok(false)
    }
}

impl Registry {
    fn validate_targets(&self, targets: &Value) -> Result<(), Error> {
        use crate::authoring::AuthoredLibrary;
        use std::collections::BTreeSet;
        let targets = targets.as_object().ok_or(Error::Invalid("targets map"))?;
        let metadata_total = targets.values().try_fold(0u64, |total, pin| {
            total
                .checked_add(
                    pin["length"]
                        .as_u64()
                        .ok_or(Error::Invalid("target length"))?,
                )
                .ok_or(Error::Invalid("metadata budget"))
        })?;
        if metadata_total > 256 * 1024 * 1024 {
            return Err(Error::Invalid("metadata budget"));
        }
        let mut releases = BTreeSet::new();
        let mut used = BTreeSet::new();
        let mut content_total = 0usize;
        for (path, pin) in targets {
            if !path.starts_with("records/") {
                continue;
            }
            let bytes = self.target(path, pin)?;
            let record = decode_registry_record(&bytes, &Subject::Lock)?;
            let release = record.release_record().release();
            if !releases.insert(release.clone()) {
                return Err(Error::Invalid("duplicate release identity"));
            }
            let custom = &pin["custom"]["morphir"];
            let status = custom["status"]
                .as_str()
                .ok_or(Error::Invalid("record status"))?;
            if !matches!(status, "active" | "yanked" | "revoked")
                || custom
                    != &json!({"formatVersion":"0.1.0-draft.3","kind":"LibraryRelease","release":release,"status":status})
            {
                return Err(Error::Invalid("record target declaration mismatch"));
            }
            let statement_path = record.statement().path().as_str();
            let statement_pin = targets
                .get(statement_path)
                .ok_or(Error::Invalid("missing statement target"))?;
            if statement_pin["custom"]["morphir"]
                != json!({"formatVersion":"0.1.0-draft.3","kind":"LibraryReleaseStatement","release":release})
            {
                return Err(Error::Invalid("statement target declaration mismatch"));
            }
            if !used.insert(path.clone()) || !used.insert(statement_path.to_owned()) {
                return Err(Error::Invalid("duplicate target association"));
            }
            let envelope = self.target(statement_path, statement_pin)?;
            let bundle = self.root.descend(record.source().path().as_str())?;
            if bundle.names()? != vec!["ir.json".to_owned(), "manifest.json".to_owned()] {
                return Err(Error::Invalid("bundle entries"));
            }
            let manifest = bundle.read("manifest.json", 1_048_576)?;
            let ir = bundle.read("ir.json", 64 * 1024 * 1024)?;
            content_total = content_total
                .checked_add(ir.len())
                .ok_or(Error::Invalid("content budget"))?;
            if content_total > 256 * 1024 * 1024 {
                return Err(Error::Invalid("content budget"));
            }
            let library = AuthoredLibrary::from_bundle(&manifest, &ir)
                .map_err(|_| Error::Invalid("invalid established Library"))?;
            verify::release(&library, &bytes, &envelope, &self.policy)?;
        }
        if used.len() != targets.len() {
            return Err(Error::Invalid("orphan or unsupported target"));
        }
        Ok(())
    }
}
