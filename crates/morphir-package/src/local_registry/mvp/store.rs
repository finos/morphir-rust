use super::{Error, files, require};
use crate::local_registry::{Digest, PolicyRepository, TufRole, tuf::*};
use async_trait::async_trait;
use package_tough::experimental_storage::{
    MetadataRole, Reset, RetainedMetadata, Revision, Snapshot, Transition,
};
use rusqlite::{Connection, OpenFlags, params};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
const BUDGET: usize = 268_435_456;
fn connect(path: &Path) -> Result<Connection, Error> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    db.execute_batch(
        "PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
    )?;
    Ok(db)
}
pub(super) fn initialize(path: &Path, policy: &PolicyRepository, root: &[u8]) -> Result<(), Error> {
    fs::create_dir(path)?;
    // Creation is exclusive. A failed initialization leaves a directory that cannot be reset implicitly.
    let lock = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path.join("lock"))?;
    fs2::FileExt::try_lock_exclusive(&lock)?;
    lock.sync_all()?;
    let dbfile = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path.join("trust.sqlite"))?;
    dbfile.sync_all()?;
    let db = connect(&path.join("trust.sqlite"))?;
    db.execute_batch("BEGIN IMMEDIATE;
    CREATE TABLE state (id INTEGER PRIMARY KEY CHECK(id=1), identity TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision>0), bootstrap BLOB NOT NULL, current BLOB NOT NULL, baseline BLOB, accepted TEXT, charged INTEGER NOT NULL CHECK(charged>=0), seal TEXT NOT NULL);
    CREATE TABLE roots (ordinal INTEGER PRIMARY KEY, bytes BLOB NOT NULL);
    CREATE TABLE metadata (role TEXT PRIMARY KEY, bytes BLOB NOT NULL, root BLOB NOT NULL);
    CREATE TABLE evidence (role TEXT NOT NULL, digest TEXT NOT NULL, bytes BLOB NOT NULL, PRIMARY KEY(role,digest));")?;
    db.execute(
        "INSERT INTO state VALUES(1,?1,1,?2,?2,NULL,NULL,0,'')",
        params![policy.identity().as_str(), root],
    )?;
    db.execute("INSERT INTO roots VALUES(1,?1)", [root])?;
    seal::write(&db)?;
    db.execute_batch("COMMIT")?;
    File::open(path.join("trust.sqlite"))?.sync_all()?;
    Ok(())
}
#[derive(Debug)]
pub(super) struct Backend {
    db: Mutex<Connection>,
    _lock: File,
    path: PathBuf,
    marker: Vec<u8>,
    pub binding: OperationBinding,
}
impl Backend {
    pub fn begin(
        path: &Path,
        policy: &PolicyRepository,
        time: jiff::Timestamp,
        charged: usize,
    ) -> Result<Arc<Self>, Error> {
        require(path.exists(), "trust state is uninitialized or missing")?;
        let path = files::directory(path)?;
        files::regular(&path.join("lock"))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.join("lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .map_err(|_| Error::Refused("trust state is locked by another operation"))?;
        files::absent(&path.join("operation")).map_err(|_| {
            Error::Refused("unresolved prior operation; manual intervention required")
        })?;
        require(
            path.join("trust.sqlite").exists(),
            "trust state database is missing",
        )?;
        files::regular(&path.join("trust.sqlite"))?;
        let db = connect(&path.join("trust.sqlite"))?;
        let check: String = db.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
        require(check == "ok", "corrupt trust database")?;
        Self::budget(&db)?;
        seal::check(&db)?;
        let (identity, revision, current): (String, i64, Vec<u8>) = db.query_row(
            "SELECT identity,revision,current FROM state WHERE id=1 AND length(current)<=1048576",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        require(
            identity == policy.identity().as_str(),
            "repository identity does not match provisioned state",
        )?;
        require(charged <= BUDGET, "metadata budget")?;
        // An exclusive filesystem marker precedes every transaction, and remains on ALL errors.
        let marker = format!("{}:{}:{}", identity, revision, time).into_bytes();
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path.join("operation"))?;
        file.write_all(&marker)?;
        file.sync_all()?;
        db.execute_batch("BEGIN IMMEDIATE; DELETE FROM evidence;")?;
        db.execute("UPDATE state SET charged=?1 WHERE id=1", [charged as i64])?;
        seal::write(&db)?;
        db.execute_batch("COMMIT")?;
        let binding = OperationBinding {
            id: OperationId::new(<sha2::Sha256 as sha2::Digest>::digest(&marker).into()),
            repository: policy.identity().clone(),
            initial_root: Digest::parse(&crate::digest::Digest::of_bytes(&current).to_string())
                .map_err(|_| Error::Refused("root digest"))?,
            predecessor: Revision(
                u64::try_from(revision).map_err(|_| Error::Refused("corrupt revision"))?,
            ),
            fixed_time: time,
        };
        Ok(Arc::new(Self {
            db: Mutex::new(db),
            _lock: lock,
            path,
            marker,
            binding,
        }))
    }
    fn check_marker(&self) -> Result<(), Error> {
        require(
            files::read(&self.path, "operation", 1024)? == self.marker,
            "unresolved or changed operation marker",
        )
    }
    fn budget(db: &Connection) -> Result<(), Error> {
        let size:i64=db.query_row("SELECT charged + length(bootstrap)+length(current)+coalesce(length(baseline),0)+(SELECT coalesce(sum(length(bytes)),0) FROM roots)+(SELECT coalesce(sum(length(bytes)+length(root)),0) FROM metadata)+(SELECT coalesce(sum(length(bytes)),0) FROM evidence) FROM state WHERE id=1",[],|r|r.get(0))?;
        require(
            size >= 0 && size as usize <= BUDGET,
            "aggregate metadata budget",
        )
    }
    pub fn charge(&self, n: usize) -> Result<(), Error> {
        self.check_marker()?;
        let mut db = self
            .db
            .lock()
            .map_err(|_| Error::Refused("poisoned trust state"))?;
        let tx = db.transaction()?;
        seal::check(&tx)?;
        tx.execute("UPDATE state SET charged=charged+?1 WHERE id=1", [n as i64])?;
        Self::budget(&tx)?;
        seal::write(&tx)?;
        tx.commit()?;
        Ok(())
    }
    pub fn authorized(&self) -> Result<(), Error> {
        self.check_marker()?;
        let mut db = self
            .db
            .lock()
            .map_err(|_| Error::Refused("poisoned trust state"))?;
        let tx = db.transaction()?;
        seal::check(&tx)?;
        tx.execute(
            "UPDATE state SET accepted=?1 WHERE id=1",
            [self.binding.fixed_time.to_string()],
        )?;
        seal::write(&tx)?;
        tx.commit()?;
        File::open(self.path.join("trust.sqlite"))?.sync_all()?;
        Ok(())
    }
    pub fn finish(&self) -> Result<(), Error> {
        self.check_marker()?;
        fs::remove_file(self.path.join("operation"))?;
        Ok(())
    }
}
fn role_name(role: TufRole) -> &'static str {
    match role {
        TufRole::Root => "root",
        TufRole::Timestamp => "timestamp",
        TufRole::Snapshot => "snapshot",
        TufRole::Targets => "targets",
    }
}
fn role(text: &str) -> Result<TufRole, Error> {
    match text {
        "root" => Ok(TufRole::Root),
        "timestamp" => Ok(TufRole::Timestamp),
        "snapshot" => Ok(TufRole::Snapshot),
        "targets" => Ok(TufRole::Targets),
        _ => Err(Error::Refused("corrupt metadata role")),
    }
}
fn metadata_role(role: TufRole) -> Result<MetadataRole, Error> {
    match role {
        TufRole::Timestamp => Ok(MetadataRole::Timestamp),
        TufRole::Snapshot => Ok(MetadataRole::Snapshot),
        TufRole::Targets => Ok(MetadataRole::Targets),
        _ => Err(Error::Refused("invalid retained role")),
    }
}
fn port(error: Error) -> AdmissionError {
    AdmissionError::Backend(Box::new(error))
}
#[async_trait]
impl AdmissionBackend for Backend {
    async fn read(&self) -> Result<OperationSnapshot, AdmissionError> {
        (|| -> Result<_, Error> {
            self.check_marker()?;
            let mut db = self
                .db
                .lock()
                .map_err(|_| Error::Refused("poisoned trust state"))?;
            let tx = db.transaction()?;
            Self::budget(&tx)?;
            seal::check(&tx)?;
            let mut state = tx
                .query_row(
                    "SELECT revision,bootstrap,current,baseline,accepted FROM state WHERE id=1",
                    [],
                    |r| {
                        let time: Option<String> = r.get(4)?;
                        Ok((
                            r.get::<_, i64>(0)? as u64,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            time,
                        ))
                    },
                )
                .and_then(
                    |(revision, provisioned_root, current_root, reset_baseline, time)| {
                        let accepted_time = time
                            .map(|s| s.parse().map_err(|_| rusqlite::Error::InvalidQuery))
                            .transpose()?;
                        Ok(Snapshot {
                            revision: Revision(revision),
                            provisioned_root,
                            current_root,
                            reset_baseline,
                            accepted_time,
                            metadata: BTreeMap::new(),
                        })
                    },
                )?;
            let roots = tx
                .prepare("SELECT bytes FROM roots ORDER BY ordinal")?
                .query_map([], |r| r.get(0))?
                .collect::<Result<Vec<Vec<u8>>, _>>()?;
            for row in tx
                .prepare("SELECT role,bytes,root FROM metadata")?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)))?
            {
                let (name, bytes, acceptance_root) = row?;
                state.metadata.insert(
                    metadata_role(role(&name)?)?,
                    RetainedMetadata {
                        bytes,
                        acceptance_root,
                    },
                );
            }
            let mut evidence = vec![];
            for row in tx
                .prepare("SELECT role,bytes FROM evidence")?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))?
            {
                let (name, bytes) = row?;
                evidence.push(CandidateEvidence {
                    role: role(&name)?,
                    bytes,
                });
            }
            let charged: i64 =
                tx.query_row("SELECT charged FROM state WHERE id=1", [], |r| r.get(0))?;
            Ok(OperationSnapshot {
                binding: self.binding.clone(),
                state,
                root_chain: roots,
                evidence,
                other_metadata_bytes: usize::try_from(charged)
                    .map_err(|_| Error::Refused("invalid metadata budget"))?,
            })
        })()
        .map_err(port)
    }
    async fn record(
        &self,
        expected: Revision,
        evidence: CandidateEvidence,
    ) -> Result<(), AdmissionError> {
        (|| -> Result<_, Error> {
            self.check_marker()?;
            let mut db = self
                .db
                .lock()
                .map_err(|_| Error::Refused("poisoned trust state"))?;
            let tx = db.transaction()?;
            require(
                tx.query_row("SELECT revision FROM state WHERE id=1", [], |r| {
                    r.get::<_, i64>(0)
                })? == expected.0 as i64,
                "revision conflict",
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO evidence VALUES(?1,?2,?3)",
                params![
                    role_name(evidence.role),
                    crate::digest::Digest::of_bytes(&evidence.bytes).to_string(),
                    evidence.bytes
                ],
            )?;
            Self::budget(&tx)?;
            seal::write(&tx)?;
            tx.commit()?;
            Ok(())
        })()
        .map_err(port)
    }
    async fn commit(
        &self,
        expected: Revision,
        transition: &Transition,
    ) -> Result<Revision, AdmissionError> {
        (|| -> Result<_, Error> {
            self.check_marker()?;
            let mut db = self
                .db
                .lock()
                .map_err(|_| Error::Refused("poisoned trust state"))?;
            let tx = db.transaction()?;
            require(
                tx.query_row("SELECT revision FROM state WHERE id=1", [], |r| {
                    r.get::<_, i64>(0)
                })? == expected.0 as i64,
                "revision conflict",
            )?;
            match transition {
                Transition::AdvanceRoot { root, baseline } => {
                    tx.execute(
                        "UPDATE state SET current=?1,baseline=?2 WHERE id=1",
                        params![root, baseline],
                    )?;
                    tx.execute(
                        "INSERT INTO roots SELECT max(ordinal)+1,?1 FROM roots",
                        [root],
                    )?;
                }
                Transition::FinishRootCycle { reset } => {
                    if *reset == Reset::TimestampAndSnapshot {
                        tx.execute(
                            "DELETE FROM metadata WHERE role IN ('timestamp','snapshot')",
                            [],
                        )?;
                    }
                    tx.execute("UPDATE state SET baseline=NULL WHERE id=1", [])?;
                }
                Transition::Retain { role, metadata } => {
                    let name = match role {
                        MetadataRole::Timestamp => "timestamp",
                        MetadataRole::Snapshot => "snapshot",
                        MetadataRole::Targets => "targets",
                        _ => return Err(Error::Refused("delegated metadata")),
                    };
                    tx.execute(
                        "INSERT OR REPLACE INTO metadata VALUES(?1,?2,?3)",
                        params![name, metadata.bytes, metadata.acceptance_root],
                    )?;
                }
            }
            Self::budget(&tx)?;
            let next = expected
                .0
                .checked_add(1)
                .ok_or(Error::Refused("revision overflow"))?;
            tx.execute(
                "UPDATE state SET revision=?1 WHERE id=1",
                [i64::try_from(next).map_err(|_| Error::Refused("revision overflow"))?],
            )?;
            seal::write(&tx)?;
            tx.commit()?;
            Ok(Revision(next))
        })()
        .map_err(port)
    }
}
// Detect missing or accidentally altered rows before they can erase rollback
// floors. This is consistency evidence, never a replacement for authentication.
mod seal {
    use super::*;
    use sha2::{Digest as _, Sha256};
    fn digest(db: &Connection) -> Result<String, Error> {
        let mut hash = Sha256::new();
        for query in [
            "SELECT identity,revision,bootstrap,current,baseline,accepted,charged FROM state ORDER BY id",
            "SELECT ordinal,bytes FROM roots ORDER BY ordinal",
            "SELECT role,bytes,root FROM metadata ORDER BY role",
            "SELECT role,digest,bytes FROM evidence ORDER BY role,digest",
        ] {
            hash.update(query.as_bytes());
            let mut stmt = db.prepare(query)?;
            let count = stmt.column_count();
            let mut rows = stmt.query([])?;
            while let Some(row) = rows.next()? {
                hash.update([0xff]);
                for i in 0..count {
                    use rusqlite::types::ValueRef;
                    match row.get_ref(i)? {
                        ValueRef::Null => hash.update([0]),
                        ValueRef::Integer(n) => {
                            hash.update([1]);
                            hash.update(n.to_be_bytes());
                        }
                        ValueRef::Text(b) | ValueRef::Blob(b) => {
                            hash.update([2]);
                            hash.update((b.len() as u64).to_be_bytes());
                            hash.update(b);
                        }
                        _ => return Err(Error::Refused("corrupt trust value")),
                    }
                }
            }
        }
        Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }
    pub fn check(db: &Connection) -> Result<(), Error> {
        let expected: String =
            db.query_row("SELECT seal FROM state WHERE id=1", [], |r| r.get(0))?;
        require(
            expected == digest(db)?,
            "corrupt trust state consistency seal",
        )
    }
    pub fn write(db: &Connection) -> Result<(), Error> {
        db.execute("UPDATE state SET seal=?1 WHERE id=1", [digest(db)?])?;
        Ok(())
    }
}
