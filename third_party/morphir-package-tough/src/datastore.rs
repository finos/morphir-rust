// Copyright 2019 Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::error::{self, Result};
use jiff::Timestamp;
use log::debug;
use serde::Serialize;
use snafu::{ensure, ResultExt};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::{Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// `Datastore` persists TUF metadata files.
#[derive(Debug, Clone)]
pub(crate) struct Datastore {
    /// A lock around retrieving the datastore path.
    path_lock: Arc<RwLock<DatastorePath>>,
    /// A lock to treat the `system_time` function as a critical section.
    time_lock: Arc<Mutex<()>>,
    /// A host-supplied operation time; absent means sample the system clock.
    fixed_time: Option<Timestamp>,
    #[cfg(feature = "experimental-storage")]
    experimental: Option<Arc<crate::experimental_storage::Session>>,
}

impl Datastore {
    pub(crate) fn new(path: Option<PathBuf>, fixed_time: Option<Timestamp>) -> Result<Self> {
        Ok(Self {
            path_lock: Arc::new(RwLock::new(match path {
                None => DatastorePath::TempDir(TempDir::new().context(error::DatastoreInitSnafu)?),
                Some(p) => DatastorePath::Path(p),
            })),
            time_lock: Arc::new(Mutex::new(())),
            fixed_time,
            #[cfg(feature = "experimental-storage")]
            experimental: None,
        })
    }

    async fn read(&self) -> RwLockReadGuard<'_, DatastorePath> {
        self.path_lock.read().await
    }

    async fn write(&self) -> RwLockWriteGuard<'_, DatastorePath> {
        self.path_lock.write().await
    }

    /// Get contents of a file in the datastore. This function is thread safe.
    ///
    /// TODO: [provide a thread safe interface](https://github.com/awslabs/tough/issues/602)
    ///
    pub(crate) async fn bytes(&self, file: &str) -> Result<Option<Vec<u8>>> {
        #[cfg(feature = "experimental-storage")]
        if let Some(session) = &self.experimental {
            return session
                .bytes(file)
                .await
                .map_err(|source| error::Error::ExperimentalStorage { source });
        }

        let lock = &self.read().await;
        let path = lock.path().join(file);
        match tokio::fs::read(&path).await {
            Ok(file) => Ok(Some(file)),
            Err(err) => match err.kind() {
                ErrorKind::NotFound => Ok(None),
                _ => Err(err).context(error::DatastoreOpenSnafu { path: &path }),
            },
        }
    }

    /// Writes a JSON metadata file in the datastore. This function is thread safe.
    pub(crate) async fn create<T: Serialize>(&self, file: &str, value: &T) -> Result<()> {
        #[cfg(feature = "experimental-storage")]
        if self.is_experimental() {
            return Err(error::Error::ExperimentalStorage {
                source: crate::experimental_storage::Error::Corrupt("untyped storage mutation"),
            });
        }
        let lock = &self.write().await;
        let path = lock.path().join(file);
        let bytes = serde_json::to_vec(value).with_context(|_| error::DatastoreSerializeSnafu {
            what: format!("{file} in datastore"),
            path: path.clone(),
        })?;
        tokio::fs::write(&path, bytes)
            .await
            .context(error::DatastoreCreateSnafu { path: &path })
    }

    /// Deletes a file from the datastore. This function is thread safe.
    pub(crate) async fn remove(&self, file: &str) -> Result<()> {
        #[cfg(feature = "experimental-storage")]
        if self.is_experimental() {
            return Err(error::Error::ExperimentalStorage {
                source: crate::experimental_storage::Error::Corrupt("untyped storage mutation"),
            });
        }
        let lock = self.write().await;
        let path = lock.path().join(file);
        debug!("removing '{}'", path.display());
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(err) => match err.kind() {
                ErrorKind::NotFound => Ok(()),
                _ => Err(err).context(error::DatastoreRemoveSnafu { path: &path }),
            },
        }
    }

    /// Checks the selected operation/system time against the existing rollback record.
    /// This retains upstream persistence semantics; it is not an accepted-time store.
    /// The time check and write are protected by the existing lock guard.
    pub(crate) async fn system_time(&self) -> Result<Timestamp> {
        #[cfg(feature = "experimental-storage")]
        if let Some(session) = &self.experimental {
            return session
                .time()
                .await
                .map_err(|source| error::Error::ExperimentalStorage { source });
        }

        // Treat this function as a critical section. This lock is not used for anything else.
        let lock = self.time_lock.lock().await;

        let file = "latest_known_time.json";
        // Load the latest known system time, if it exists
        let poss_latest_known_time = self
            .bytes(file)
            .await?
            .map(|b| serde_json::from_slice::<Timestamp>(&b));

        // The fixed operation time is reused on every check; otherwise sample the clock.
        let sys_time = self.fixed_time.unwrap_or_else(Timestamp::now);

        if let Some(Ok(latest_known_time)) = poss_latest_known_time {
            // Make sure the sampled system time did not go back in time
            ensure!(
                sys_time >= latest_known_time,
                error::SystemTimeSteppedBackwardSnafu {
                    sys_time,
                    latest_known_time
                }
            );
        }
        // Store the latest known time
        // Serializes RFC3339 time string and store to datastore
        self.create(file, &sys_time).await?;

        // Explicitly drop the lock to avoid any compiler optimization.
        drop(lock);
        Ok(sys_time)
    }
}

/// Because `TempDir` is an RAII object, we need to hold on to it. This private enum allows us to
/// hold either a `TempDir` or a `PathBuf` depending on whether or not the user wants to manage the
/// directory.
#[derive(Debug)]
enum DatastorePath {
    /// Path to a user-managed directory.
    Path(PathBuf),
    /// A `TempDir` that we created on the user's behalf.
    TempDir(TempDir),
}

impl DatastorePath {
    /// Provides convenient access to the underlying filepath.
    fn path(&self) -> &Path {
        match self {
            DatastorePath::Path(p) => p,
            DatastorePath::TempDir(t) => t.path(),
        }
    }
}

impl Datastore {
    #[cfg(feature = "experimental-storage")]
    pub(crate) fn experimental(
        session: crate::experimental_storage::Session,
        fixed_time: Option<Timestamp>,
    ) -> Self {
        Self {
            path_lock: Arc::new(RwLock::new(DatastorePath::Path(PathBuf::new()))),
            time_lock: Arc::new(Mutex::new(())),
            fixed_time,
            experimental: Some(Arc::new(session)),
        }
    }
    pub(crate) fn is_experimental(&self) -> bool {
        #[cfg(feature = "experimental-storage")]
        {
            self.experimental.is_some()
        }
        #[cfg(not(feature = "experimental-storage"))]
        {
            false
        }
    }
    pub(crate) async fn current_root(&self, fallback: &[u8]) -> Vec<u8> {
        #[cfg(feature = "experimental-storage")]
        if let Some(session) = &self.experimental {
            return session.root().await;
        }
        fallback.to_vec()
    }
    pub(crate) async fn root_cycle_baseline(&self, fallback: &[u8]) -> Vec<u8> {
        #[cfg(feature = "experimental-storage")]
        if let Some(session) = &self.experimental {
            return session.baseline().await;
        }
        fallback.to_vec()
    }
    pub(crate) async fn persist_root(
        &self,
        bytes: &[u8],
        root: &crate::schema::Signed<crate::schema::Root>,
    ) -> Result<()> {
        #[cfg(feature = "experimental-storage")]
        if let Some(session) = &self.experimental {
            return session
                .advance_root(bytes)
                .await
                .map_err(|source| error::Error::ExperimentalStorage { source });
        }
        let _ = bytes;
        self.remove("root.json").await?;
        self.create("root.json", root).await
    }
    pub(crate) async fn finish_root_cycle(&self, reset: bool) -> Result<()> {
        #[cfg(feature = "experimental-storage")]
        if let Some(session) = &self.experimental {
            use crate::experimental_storage::Reset;
            return session
                .finish(if reset {
                    Reset::TimestampAndSnapshot
                } else {
                    Reset::Preserve
                })
                .await
                .map_err(|source| error::Error::ExperimentalStorage { source });
        }
        if reset {
            let timestamp = self.remove("timestamp.json").await;
            let snapshot = self.remove("snapshot.json").await;
            timestamp.and(snapshot)?;
        }
        Ok(())
    }
    pub(crate) async fn persist_metadata<T: crate::schema::Role>(
        &self,
        file: &str,
        bytes: &[u8],
        metadata: &crate::schema::Signed<T>,
        delegated: Option<&str>,
    ) -> Result<()> {
        #[cfg(feature = "experimental-storage")]
        if let Some(session) = &self.experimental {
            use crate::{experimental_storage::MetadataRole, schema::RoleType};
            let role = match (T::TYPE, delegated) {
                (RoleType::Timestamp, None) => MetadataRole::Timestamp,
                (RoleType::Snapshot, None) => MetadataRole::Snapshot,
                (RoleType::Targets, None) => MetadataRole::Targets,
                (RoleType::Targets, Some(name)) => MetadataRole::Delegated(name.to_owned()),
                _ => {
                    return Err(error::Error::ExperimentalStorage {
                        source: crate::experimental_storage::Error::Corrupt(
                            "invalid retained metadata role",
                        ),
                    })
                }
            };
            return session
                .retain(role, bytes)
                .await
                .map_err(|source| error::Error::ExperimentalStorage { source });
        }
        let _ = (bytes, delegated);
        self.create(file, metadata).await
    }
}
