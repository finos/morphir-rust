use super::{Admission, Error, MetadataRole, Reset, Result, Snapshot, Storage, Transition};
use crate::schema::{Root, Signed, Snapshot as TufSnapshot, Targets, Timestamp as TufTimestamp};
use jiff::Timestamp;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug)]
pub(crate) struct Session {
    storage: Arc<dyn Storage>,
    admission: Arc<dyn Admission>,
    state: Mutex<Snapshot>,
    fixed_time: Timestamp,
}
impl Session {
    pub(crate) async fn open(
        storage: Arc<dyn Storage>,
        admission: Arc<dyn Admission>,
        bootstrap: &[u8],
        fixed_time: Timestamp,
    ) -> Result<Self> {
        let state = storage.snapshot().await?;
        validate(&state)?;
        if state.provisioned_root != bootstrap {
            return Err(Error::Corrupt("provisioning identity mismatch"));
        }
        if let Some(accepted_time) = state
            .accepted_time
            .filter(|accepted| fixed_time < *accepted)
        {
            return Err(Error::TimeRollback {
                fixed_time,
                accepted_time,
            });
        }
        admission.begin(&state, fixed_time).await?;
        Ok(Self {
            storage,
            admission,
            state: Mutex::new(state),
            fixed_time,
        })
    }
    pub(crate) async fn root(&self) -> Vec<u8> {
        self.state.lock().await.current_root.clone()
    }
    pub(crate) async fn baseline(&self) -> Vec<u8> {
        let state = self.state.lock().await;
        state
            .reset_baseline
            .as_ref()
            .unwrap_or(&state.current_root)
            .clone()
    }
    pub(crate) async fn bytes(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let state = self.state.lock().await;
        let role = match name {
            "timestamp.json" => MetadataRole::Timestamp,
            "snapshot.json" => MetadataRole::Snapshot,
            _ => return Err(Error::Corrupt("unexpected retained-role read")),
        };
        let bytes = state.metadata.get(&role).cloned();
        if let Some(bytes) = &bytes {
            let root = parse_root(&state.current_root)?;
            let valid = match role {
                MetadataRole::Timestamp => root
                    .signed
                    .verify_role(&parse::<Signed<TufTimestamp>>(bytes)?)
                    .is_ok(),
                MetadataRole::Snapshot => root
                    .signed
                    .verify_role(&parse::<Signed<TufSnapshot>>(bytes)?)
                    .is_ok(),
                _ => false,
            };
            if !valid {
                return Err(Error::Corrupt("retained rollback metadata signature"));
            }
        }
        Ok(bytes)
    }
    pub(crate) async fn time(&self) -> Result<Timestamp> {
        let state = self.state.lock().await;
        let persisted = self.storage.snapshot().await?;
        validate(&persisted)?;
        if persisted.revision != state.revision {
            return Err(Error::Conflict);
        }
        if let Some(accepted_time) = persisted
            .accepted_time
            .filter(|accepted| self.fixed_time < *accepted)
        {
            return Err(Error::TimeRollback {
                fixed_time: self.fixed_time,
                accepted_time,
            });
        }
        Ok(self.fixed_time)
    }
    pub(crate) async fn advance_root(&self, root: &[u8]) -> Result<()> {
        let baseline = self.baseline().await;
        self.commit(Transition::AdvanceRoot {
            root: root.to_vec(),
            baseline,
        })
        .await
    }
    pub(crate) async fn finish(&self, reset: Reset) -> Result<()> {
        if self.state.lock().await.reset_baseline.is_none() && reset == Reset::Preserve {
            return Ok(());
        }
        self.commit(Transition::FinishRootCycle { reset }).await
    }
    pub(crate) async fn retain(&self, role: MetadataRole, bytes: &[u8]) -> Result<()> {
        self.commit(Transition::Retain {
            role,
            bytes: bytes.to_vec(),
        })
        .await
    }
    async fn commit(&self, transition: Transition) -> Result<()> {
        let mut state = self.state.lock().await;
        self.admission.transition(&state, &transition).await?;
        let revision = self.storage.commit(state.revision, &transition).await?;
        let persisted = self.storage.snapshot().await?;
        if persisted.revision != revision {
            return Err(Error::Conflict);
        }
        validate(&persisted)?;
        *state = persisted;
        Ok(())
    }
}
fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| Error::Corrupt("retained metadata encoding"))
}
fn parse_root(bytes: &[u8]) -> Result<Signed<Root>> {
    let root: Signed<Root> = parse(bytes)?;
    root.signed
        .verify_role(&root)
        .map_err(|_| Error::Corrupt("retained root signature"))?;
    Ok(root)
}
fn validate(state: &Snapshot) -> Result<()> {
    parse_root(&state.provisioned_root)?;
    parse_root(&state.current_root)?;
    if let Some(baseline) = &state.reset_baseline {
        parse_root(baseline)?;
    }
    for (role, bytes) in &state.metadata {
        match role {
            MetadataRole::Timestamp => {
                parse::<Signed<TufTimestamp>>(bytes)?;
            }
            MetadataRole::Snapshot => {
                parse::<Signed<TufSnapshot>>(bytes)?;
            }
            MetadataRole::Targets | MetadataRole::Delegated(_) => {
                parse::<Signed<Targets>>(bytes)?;
            }
        }
    }
    Ok(())
}
