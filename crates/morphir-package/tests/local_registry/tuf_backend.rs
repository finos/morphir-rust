use super::fixtures::*;
use async_trait::async_trait;
use morphir_package::local_registry::{tuf::*, *};
use package_tough::experimental_storage::{Revision, Snapshot, Transition};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
#[derive(Debug)]
pub struct Backend(
    pub Mutex<OperationSnapshot>,
    pub std::sync::atomic::AtomicU8,
);
#[async_trait]
impl AdmissionBackend for Backend {
    async fn read(&self) -> Result<OperationSnapshot, AdmissionError> {
        Ok(self.0.lock().unwrap().clone())
    }
    async fn record(
        &self,
        expected: Revision,
        evidence: CandidateEvidence,
    ) -> Result<(), AdmissionError> {
        let mut state = self.0.lock().unwrap();
        if state.state.revision != expected {
            return Err(AdmissionError::Context);
        }
        state.evidence.push(evidence);
        if self.1.load(std::sync::atomic::Ordering::SeqCst) == 1 {
            return Err(AdmissionError::Context);
        }
        Ok(())
    }
    async fn commit(
        &self,
        expected: Revision,
        transition: &Transition,
    ) -> Result<Revision, AdmissionError> {
        let mut state = self.0.lock().unwrap();
        if state.state.revision != expected {
            return Err(AdmissionError::Context);
        }
        match transition {
            Transition::Retain { role, metadata } => {
                state.state.metadata.insert(role.clone(), metadata.clone());
            }
            Transition::AdvanceRoot { root, baseline } => {
                state.state.current_root = root.clone();
                state.root_chain.push(root.clone());
                state.state.reset_baseline = Some(baseline.clone());
            }
            Transition::FinishRootCycle { .. } => {
                state.state.reset_baseline = None;
            }
        }
        state.state.revision.0 += 1;
        if self.1.load(std::sync::atomic::Ordering::SeqCst) == 2 {
            return Err(AdmissionError::Context);
        }
        Ok(state.state.revision)
    }
}
pub fn setup() -> (Arc<Backend>, ProfileAdmission) {
    let root = root(1, 17);
    let digest = format!(
        "sha256:{}",
        hex(<sha2::Sha256 as sha2::Digest>::digest(&root))
    );
    let identity = format!("sha256:{}", "ab".repeat(32));
    let policy=decode_trust_policy(&serde_json::to_vec(&json!({"formatVersion":"0.1.0-draft.3","kind":"LibraryTrustPolicy","repositories":[{"identity":identity,"bootstrapRoot":{"version":1,"digest":digest},"namespaces":["example.com"]}],"publisherRules":[],"continuedUse":"fresh-metadata"})).unwrap()).unwrap();
    let binding = OperationBinding {
        id: OperationId::new([23; 32]),
        repository: Digest::parse(&identity).unwrap(),
        initial_root: Digest::parse(&digest).unwrap(),
        predecessor: Revision(1),
        fixed_time: "2030-01-01T00:00:00Z".parse().unwrap(),
    };
    let backend = Arc::new(Backend(
        Mutex::new(OperationSnapshot {
            binding: binding.clone(),
            root_chain: vec![root.clone()],
            evidence: vec![],
            other_metadata_bytes: 0,
            state: Snapshot {
                revision: Revision(1),
                provisioned_root: root.clone(),
                current_root: root,
                reset_baseline: None,
                metadata: BTreeMap::new(),
                accepted_time: None,
            },
        }),
        std::sync::atomic::AtomicU8::new(0),
    ));
    let admission =
        ProfileAdmission::new(policy.repositories()[0].clone(), binding, backend.clone()).unwrap();
    (backend, admission)
}
