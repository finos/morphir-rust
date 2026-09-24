use super::*;

#[test]
#[cfg(unix)]
fn republishing_a_draft_one_index_upgrades_the_checked_record() {
    use morphir_distribution::{PublicationDescription, PublicationStatus};
    let temp = tempfile::tempdir().unwrap();
    let bundle = bundle(temp.path(), &[claims()]);
    let suffix = if std::env::consts::OS == "macos" {
        "apple-darwin"
    } else {
        "unknown-linux-gnu"
    };
    edit(&bundle, |value| {
        value["artifacts"][0]["platform"] = json!(format!("{}-{suffix}", std::env::consts::ARCH));
    });
    let repository = LocalExtensionRepository::init(temp.path().join("repository")).unwrap();
    let describe = |artifact: &morphir_distribution::BundleArtifactDescriptor, _: &[u8]| {
        Ok(PublicationDescription::Describe(artifact.claims().clone()))
    };
    repository
        .publish_with_process_probe(&bundle, describe)
        .unwrap();
    let history = repository.root().join("extensions/example.jsonl");
    let mut old: Value = serde_json::from_slice(&fs::read(&history).unwrap()).unwrap();
    old["schemaVersion"] = json!("2.0.0-draft.1");
    old["futureIndexMember"] = json!({"preserved": true});
    let artifact = old["artifacts"][0].as_object_mut().unwrap();
    let mut document = artifact.remove("claims").unwrap();
    document.as_object_mut().unwrap().remove("claimsVersion");
    document["statementVersion"] = json!("0.1.0-draft.1");
    artifact.insert("statement".into(), document);
    artifact.remove("claimCheck");
    artifact.remove("probeSource");
    artifact.insert("statementSource".into(), json!("declared"));
    fs::write(&history, serde_json::to_vec(&old).unwrap()).unwrap();

    let publication = repository
        .publish_with_process_probe(&bundle, describe)
        .unwrap();
    assert_eq!(publication.status(), PublicationStatus::AlreadyPresent);
    let stored: Value = serde_json::from_slice(&fs::read(&history).unwrap()).unwrap();
    assert_eq!(stored["schemaVersion"], "2.0.0-draft.2");
    assert_eq!(stored["futureIndexMember"]["preserved"], true);
    assert_eq!(stored["artifacts"][0]["claimCheck"], "probed");
    assert_eq!(stored["artifacts"][0]["probeSource"], "describe");
    assert_eq!(
        stored["artifacts"][0]["claims"]["claimsVersion"],
        "0.1.0-draft.2"
    );
    assert!(stored["artifacts"][0].get("statement").is_none());
    assert!(stored["artifacts"][0].get("statementSource").is_none());
    serde_json::from_value::<morphir_distribution::ReleaseRecord>(stored).unwrap();
}
