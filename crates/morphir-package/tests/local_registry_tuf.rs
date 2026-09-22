#[path = "local_registry/tuf_mothers.rs"]
#[allow(dead_code)]
mod fixtures;
use fixtures::*;
use morphir_package::local_registry::{TufRole, tuf::*};
use serde_json::json;

#[test]
fn independent_signed_root_and_targets_admit_distinct_key_quorum() {
    let keys = [(17, key(17)), (18, key(18))];
    let root = decode_profile(&sign(root_body(1, &keys, 2), &keys), TufRole::Root).unwrap();
    assert_eq!(verify_quorum(&root, &root).unwrap(), 2);
    let targets = decode_profile(&sign(targets_body(1), &keys), TufRole::Targets).unwrap();
    assert_eq!(verify_quorum(&root, &targets).unwrap(), 2);
    let insufficient =
        decode_profile(&sign(targets_body(1), &keys[..1]), TufRole::Targets).unwrap();
    assert!(matches!(
        verify_quorum(&root, &insufficient),
        Err(AdmissionError::Signature)
    ));
}
#[test]
fn aliases_of_one_raw_key_cannot_satisfy_two_key_threshold() {
    let first = key(17);
    let mut alias = first.clone();
    alias["alias"] = json!("second identifier, same raw key");
    let keys = [(17, first), (17, alias)];
    let root = decode_profile(&sign(root_body(1, &keys, 2), &keys), TufRole::Root).unwrap();
    // Both identifiers and both signatures are independently correct.
    assert!(matches!(
        verify_quorum(&root, &root),
        Err(AdmissionError::Signature)
    ));
}
#[test]
fn invalid_extra_signature_does_not_hide_valid_threshold() {
    let root = decode_profile(&root(1, 17), TufRole::Root).unwrap();
    let mut envelope: serde_json::Value = serde_json::from_slice(&targets(1, 17)).unwrap();
    envelope["signatures"]
        .as_array_mut()
        .unwrap()
        .push(json!({"keyid":id(&key(18)),"sig":"00".repeat(64)}));
    assert_eq!(
        verify_quorum(
            &root,
            &decode_profile(&serde_json::to_vec(&envelope).unwrap(), TufRole::Targets).unwrap()
        )
        .unwrap(),
        1
    );
}
#[test]
fn exact_raw_metadata_links_include_whitespace_and_supported_sha512() {
    let bytes = targets(1, 17);
    let snapshot = decode_profile(&snapshot(&bytes, 17), TufRole::Snapshot).unwrap();
    let target = decode_profile(&bytes, TufRole::Targets).unwrap();
    verify_link(&snapshot, &target).unwrap();
    let trimmed = decode_profile(bytes.strip_prefix(b"\n ").unwrap(), TufRole::Targets).unwrap();
    assert!(matches!(
        verify_link(&snapshot, &trimmed),
        Err(AdmissionError::Link(_))
    ));
    let mut envelope: serde_json::Value = serde_json::from_slice(snapshot.bytes()).unwrap();
    envelope["signed"]["meta"]["targets.json"]["hashes"]["sha512"] = json!("00".repeat(64));
    let wrong512 =
        decode_profile(&serde_json::to_vec(&envelope).unwrap(), TufRole::Snapshot).unwrap();
    assert!(matches!(
        verify_link(&wrong512, &target),
        Err(AdmissionError::Link(_))
    ));
}

fn repository(root: &[u8], version: u64) -> morphir_package::local_registry::PolicyRepository {
    let digest = format!(
        "sha256:{}",
        hex(<sha2::Sha256 as sha2::Digest>::digest(root))
    );
    let policy = json!({"formatVersion":"0.1.0-draft.3","kind":"LibraryTrustPolicy","repositories":[{"identity":format!("sha256:{}","ab".repeat(32)),"bootstrapRoot":{"version":version,"digest":digest},"namespaces":["example.com"]}],"publisherRules":[],"continuedUse":"fresh-metadata"});
    morphir_package::local_registry::decode_trust_policy(&serde_json::to_vec(&policy).unwrap())
        .unwrap()
        .repositories()[0]
        .clone()
}
#[test]
fn provisioned_bootstrap_can_start_at_seven_but_grants_no_backward_trust() {
    let bootstrap = root(7, 17);
    let roots = AuthenticatedRoots::from_policy(
        &repository(&bootstrap, 7),
        &[bootstrap.clone(), root(8, 17)],
    )
    .unwrap();
    assert!(roots.contains(&bootstrap));
    assert!(!roots.contains(&root(6, 17)));
    assert!(
        AuthenticatedRoots::from_policy(
            &repository(&bootstrap, 7),
            &[root(6, 17), bootstrap.clone()]
        )
        .is_err()
    );
    assert!(
        AuthenticatedRoots::from_policy(&repository(&bootstrap, 7), &[bootstrap, root(9, 17)])
            .is_err()
    );
}
#[test]
fn root_rotation_requires_old_and_new_distinct_quorums() {
    let bootstrap = root(1, 17);
    let successor = root_body(2, &[(18, key(18))], 1);
    let both = sign(successor.clone(), &[(17, key(17)), (18, key(18))]);
    AuthenticatedRoots::from_policy(&repository(&bootstrap, 1), &[bootstrap.clone(), both])
        .unwrap();
    for signer in [17, 18] {
        let missing = sign(successor.clone(), &[(signer, key(signer))]);
        assert!(
            AuthenticatedRoots::from_policy(
                &repository(&bootstrap, 1),
                &[bootstrap.clone(), missing]
            )
            .is_err()
        );
    }
}
#[test]
fn self_signed_disconnected_retained_authority_and_byte_substitution_are_rejected() {
    let bootstrap = root(1, 17);
    let roots = AuthenticatedRoots::from_policy(
        &repository(&bootstrap, 1),
        std::slice::from_ref(&bootstrap),
    )
    .unwrap();
    let retained = decode_profile(&targets(1, 17), TufRole::Targets).unwrap();
    roots.verify_retained(&bootstrap, &retained).unwrap();
    assert!(
        roots
            .verify_retained(
                &root(1, 18),
                &decode_profile(&targets(1, 18), TufRole::Targets).unwrap()
            )
            .is_err()
    );
    let mut changed = bootstrap.clone();
    changed.push(b' ');
    assert!(!roots.contains(&changed));
    assert!(roots.verify_retained(&changed, &retained).is_err());
}
