//! Test-only SQLite provider and explicit probe admission, never production authority.
use std::convert::TryFrom;

use async_trait::async_trait;
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use tough::experimental_storage::{
    Admission, Error, MetadataRole, Reset, Result, Revision, Snapshot, Storage, Transition,
};

#[derive(Debug)]
pub struct Sqlite {
    pub path: PathBuf,
}
fn sql(error: rusqlite::Error) -> Error {
    Error::Backend(Box::new(error))
}
impl Sqlite {
    pub fn initialize(path: &Path, root: &[u8]) -> Self {
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .unwrap()
            .sync_all()
            .unwrap();
        let store = Self {
            path: path.to_owned(),
        };
        let mut connection = store.connection().unwrap();
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        tx.execute_batch("CREATE TABLE state(id INTEGER PRIMARY KEY CHECK(id=1), revision INTEGER NOT NULL, provisioned BLOB NOT NULL, root BLOB NOT NULL, baseline BLOB, accepted TEXT); CREATE TABLE metadata(role TEXT PRIMARY KEY, bytes BLOB NOT NULL); CREATE TABLE roots(sequence INTEGER PRIMARY KEY, bytes BLOB NOT NULL);").unwrap();
        tx.execute("INSERT INTO state VALUES(1,0,?1,?1,NULL,NULL)", [root])
            .unwrap();
        tx.execute("INSERT INTO roots VALUES(0,?1)", [root])
            .unwrap();
        tx.commit().unwrap();
        store
    }
    pub fn connection(&self) -> Result<Connection> {
        let connection = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(sql)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(sql)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(sql)?;
        #[cfg(target_os = "macos")]
        connection
            .pragma_update(None, "fullfsync", "ON")
            .map_err(sql)?;
        Ok(connection)
    }
}
#[async_trait]
impl Storage for Sqlite {
    async fn snapshot(&self) -> Result<Snapshot> {
        let mut connection = self.connection()?;
        let tx = connection.transaction().map_err(sql)?;
        let (revision, provisioned_root, current_root, reset_baseline, accepted): (
            i64,
            Vec<u8>,
            Vec<u8>,
            Option<Vec<u8>>,
            Option<String>,
        ) = tx
            .query_row(
                "SELECT revision,provisioned,root,baseline,accepted FROM state WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .map_err(sql)?;
        let accepted_time = accepted
            .map(|value| value.parse().map_err(|_| Error::Corrupt("accepted time")))
            .transpose()?;
        let mut metadata = BTreeMap::new();
        let mut query = tx.prepare("SELECT role,bytes FROM metadata").map_err(sql)?;
        for row in query
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(sql)?
        {
            let (role, bytes) = row.map_err(sql)?;
            let role: MetadataRole =
                serde_json::from_str(&role).map_err(|_| Error::Corrupt("role identity"))?;
            if metadata.insert(role, bytes).is_some() {
                return Err(Error::Corrupt("duplicate metadata role"));
            }
        }
        Ok(Snapshot {
            revision: Revision(
                u64::try_from(revision).map_err(|_| Error::Corrupt("negative revision"))?,
            ),
            provisioned_root,
            current_root,
            reset_baseline,
            metadata,
            accepted_time,
        })
    }
    async fn commit(&self, expected: Revision, transition: &Transition) -> Result<Revision> {
        let mut connection = self.connection()?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql)?;
        let actual: i64 = tx
            .query_row("SELECT revision FROM state WHERE id=1", [], |row| {
                row.get(0)
            })
            .map_err(sql)?;
        if u64::try_from(actual).map_err(|_| Error::Corrupt("negative revision"))? != expected.0 {
            return Err(Error::Conflict);
        }
        let next = expected
            .0
            .checked_add(1)
            .ok_or(Error::Corrupt("revision exhausted"))?;
        let sql_next =
            i64::try_from(next).map_err(|_| Error::Corrupt("SQLite revision exhausted"))?;
        match transition {
            Transition::AdvanceRoot { root, baseline } => {
                tx.execute(
                    "UPDATE state SET root=?1,baseline=?2 WHERE id=1",
                    rusqlite::params![root, baseline],
                )
                .map_err(sql)?;
                tx.execute(
                    "INSERT INTO roots VALUES(?1,?2)",
                    rusqlite::params![sql_next, root],
                )
                .map_err(sql)?;
            }
            Transition::FinishRootCycle { reset } => {
                if *reset == Reset::TimestampAndSnapshot {
                    for role in [MetadataRole::Timestamp, MetadataRole::Snapshot] {
                        tx.execute(
                            "DELETE FROM metadata WHERE role=?1",
                            [serde_json::to_string(&role).unwrap()],
                        )
                        .map_err(sql)?;
                    }
                }
                tx.execute("UPDATE state SET baseline=NULL WHERE id=1", [])
                    .map_err(sql)?;
            }
            Transition::Retain { role, bytes } => {
                tx.execute(
                    "INSERT OR REPLACE INTO metadata VALUES(?1,?2)",
                    rusqlite::params![serde_json::to_string(role).unwrap(), bytes],
                )
                .map_err(sql)?;
            }
        }
        tx.execute("UPDATE state SET revision=?1 WHERE id=1", [sql_next])
            .map_err(sql)?;
        let kind = match transition {
            Transition::AdvanceRoot { .. } => "root",
            Transition::FinishRootCycle { .. } => "finish",
            Transition::Retain { .. } => "metadata",
        };
        // Deterministic one-shot test fault/barrier at the transactional boundary.
        if std::env::var("MORPHIR_TUF_PROBE_FAIL").ok().as_deref() == Some(kind) {
            return Err(Error::Backend(Box::new(std::io::Error::other(
                "injected commit failure",
            ))));
        }
        barrier(kind, "before");
        tx.commit().map_err(sql)?;
        barrier(kind, "after");
        Ok(Revision(next))
    }
}
fn barrier(kind: &str, point: &str) {
    if std::env::var("MORPHIR_TUF_PROBE_BARRIER").ok().as_deref()
        == Some(&format!("{kind}:{point}"))
    {
        let signal = std::env::var_os("MORPHIR_TUF_PROBE_SIGNAL").unwrap();
        fs::write(signal, b"ready").unwrap();
        loop {
            std::thread::park();
        }
    }
}

#[derive(Debug)]
pub struct ProbeAdmission {
    pub reject_begin: bool,
    pub reject_transition: bool,
}
impl ProbeAdmission {
    pub fn permit_for_storage_probe_only() -> Self {
        Self {
            reject_begin: false,
            reject_transition: false,
        }
    }
}
#[async_trait]
impl Admission for ProbeAdmission {
    async fn begin(&self, _: &Snapshot, _: jiff::Timestamp) -> Result<()> {
        if self.reject_begin {
            Err(Error::Admission)
        } else {
            Ok(())
        }
    }
    async fn transition(&self, _: &Snapshot, _: &Transition) -> Result<()> {
        if self.reject_transition {
            Err(Error::Admission)
        } else {
            Ok(())
        }
    }
}

pub mod fixtures;
