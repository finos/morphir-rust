//! SQLite transaction probe; not the production security-state schema.
use rusqlite::{Connection, OpenFlags, TransactionBehavior};
use std::{fs::OpenOptions, path::Path};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub const EVIDENCE: &[u8] = b"exact signed bytes\0\xff\n";
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Initialization {
    Reserved,
    BeforeCommit,
    AfterCommit,
}
pub struct Store {
    pub connection: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let complete: (i64, Vec<u8>) = connection.query_row(
            "SELECT complete, identity FROM provisioning WHERE id=1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if complete != (1, b"original identity".to_vec()) {
            return Err("incomplete provisioning".into());
        }
        Self::configured(connection)
    }
    pub fn initialize(path: &Path) -> Result<Self> {
        Self::initialize_observed(path, |_| {})
    }
    pub fn initialize_observed(
        path: &Path,
        mut observe: impl FnMut(Initialization),
    ) -> Result<Self> {
        // SQLite OPEN_CREATE is not exclusive. Reserve the file with the OS first.
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let reserved = options.open(path)?;
        reserved.sync_all()?;
        observe(Initialization::Reserved);
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        let mut store = Self::configured(connection)?;
        let transaction = store
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch("CREATE TABLE provisioning(id INTEGER PRIMARY KEY CHECK(id=1), complete INTEGER NOT NULL CHECK(complete=1), identity BLOB NOT NULL);
            CREATE TABLE marker(id INTEGER PRIMARY KEY CHECK(id=1), evidence BLOB NOT NULL);
            CREATE TABLE grant_record(id INTEGER PRIMARY KEY CHECK(id=1), evidence BLOB NOT NULL);")?;
        transaction.execute(
            "INSERT INTO provisioning VALUES (1,1,?1)",
            [b"original identity".as_slice()],
        )?;
        observe(Initialization::BeforeCommit);
        transaction.commit()?;
        observe(Initialization::AfterCommit);
        store.assert_settings();
        Ok(store)
    }
    fn configured(connection: Connection) -> Result<Self> {
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        #[cfg(target_os = "macos")]
        connection.pragma_update(None, "fullfsync", "ON")?;
        let store = Self { connection };
        store.assert_settings();
        Ok(store)
    }
    pub fn assert_settings(&self) {
        let journal: String = self
            .connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        let synchronous: i64 = self
            .connection
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .unwrap();
        assert_eq!(journal, "wal");
        assert_eq!(synchronous, 2);
        #[cfg(target_os = "macos")]
        assert_eq!(
            self.connection
                .pragma_query_value(None, "fullfsync", |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    pub fn snapshot(&self) -> Result<(i64, i64)> {
        Ok(self.connection.query_row(
            "SELECT (SELECT count(*) FROM marker), (SELECT count(*) FROM grant_record)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?)
    }
    pub fn commit_marker(&mut self) -> Result<()> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("INSERT INTO marker VALUES (1,?1)", [EVIDENCE])?;
        transaction.commit()?;
        Ok(())
    }
    pub fn assert_evidence(&self, granted: bool) {
        let sql = if granted {
            "SELECT evidence FROM grant_record WHERE id=1"
        } else {
            "SELECT evidence FROM marker WHERE id=1"
        };
        let bytes: Vec<u8> = self
            .connection
            .query_row(sql, [], |row| row.get(0))
            .unwrap();
        assert_eq!(bytes, EVIDENCE);
    }
}

pub fn fail_grant_transaction(store: &mut Store) {
    let transaction = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    transaction
        .execute(
            "INSERT INTO grant_record SELECT id,evidence FROM marker",
            [],
        )
        .unwrap();
    let failure = transaction
        .execute("INSERT INTO marker VALUES (1,?1)", [EVIDENCE])
        .unwrap_err();
    assert_eq!(
        failure.sqlite_error_code(),
        Some(rusqlite::ErrorCode::ConstraintViolation)
    );
    // Dropping the failed operation rolls back its otherwise valid grant insert.
    drop(transaction);
}

pub fn fail_full_write(store: &mut Store) {
    let pages: i64 = store
        .connection
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .unwrap();
    store
        .connection
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    let transaction = store
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    transaction.execute("DELETE FROM marker", []).unwrap();
    let failure = transaction
        .execute("INSERT INTO grant_record VALUES(1,zeroblob(1048576))", [])
        .unwrap_err();
    assert_eq!(
        failure.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DiskFull)
    );
    drop(transaction);
}
