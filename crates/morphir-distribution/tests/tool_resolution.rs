use morphir_distribution::{
    Channel, ChannelSegment, DistributionError, Platform, Selection, ToolId, ToolReleaseRecord,
    ToolReleaseStatus, resolve_tool,
};
use semver::Version;

fn a_tool_release(
    version: &str,
    channels: &[&str],
    status: &str,
    cli_requirement: &str,
) -> ToolReleaseRecord {
    serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "kind": "morphir-tool-release",
        "tool": { "id": "desktop", "name": "Morphir Desktop" },
        "version": version,
        "channels": channels,
        "status": status,
        "compatibility": { "morphirCli": cli_requirement },
        "artifacts": [{
            "targetPath": format!("artifacts/desktop/{version}/windows-aarch64.zip"),
            "platform": { "os": "windows", "arch": "aarch64" },
            "archive": { "format": "zip", "entryPoint": "Morphir Desktop.exe" },
            "launch": {
                "kind": "executable",
                "path": "Morphir Desktop.exe",
                "args": ["--morphir-home"]
            }
        }]
    }))
    .unwrap()
}

#[test]
fn tool_release_descriptors_validate_the_v1_domain_contract() {
    let release = a_tool_release("1.0.0", &["stable"], "active", ">=0.4.0, <0.5.0");

    assert_eq!(release.tool_id(), &ToolId::parse("desktop").unwrap());
    assert_eq!(release.tool_name(), "Morphir Desktop");
    assert_eq!(release.version(), &Version::parse("1.0.0").unwrap());
    assert_eq!(release.status(), ToolReleaseStatus::Active);
    assert_eq!(release.artifacts()[0].launch().args(), ["--morphir-home"]);

    let unknown = serde_json::json!({
        "schemaVersion": 1,
        "kind": "morphir-tool-release",
        "tool": { "id": "desktop", "name": "Morphir Desktop" },
        "version": "1.0.0",
        "channels": ["stable"],
        "status": "active",
        "compatibility": { "morphirCli": ">=0.4.0" },
        "artifacts": [],
        "unexpected": true
    });
    assert!(serde_json::from_value::<ToolReleaseRecord>(unknown).is_err());
}

#[test]
fn tool_release_descriptors_reject_nul_in_fixed_launch_arguments() {
    let descriptor = serde_json::json!({
        "schemaVersion": 1,
        "kind": "morphir-tool-release",
        "tool": { "id": "desktop", "name": "Morphir Desktop" },
        "version": "1.0.0",
        "channels": ["stable"],
        "status": "active",
        "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
        "artifacts": [{
            "targetPath": "artifacts/desktop/1.0.0/windows-aarch64.zip",
            "platform": { "os": "windows", "arch": "aarch64" },
            "archive": { "format": "zip", "entryPoint": "Morphir Desktop.exe" },
            "launch": {
                "kind": "executable",
                "path": "Morphir Desktop.exe",
                "args": ["--profile=stable\0preview"]
            }
        }]
    });

    let error = serde_json::from_value::<ToolReleaseRecord>(descriptor).unwrap_err();
    assert!(error.to_string().contains("arguments cannot contain NUL"));
}

#[test]
fn tool_release_descriptors_keep_publisher_paths_within_the_declared_budget() {
    let oversized_entry_point = format!("{}/{}", "a".repeat(120), "b".repeat(120));
    let descriptor = serde_json::json!({
        "schemaVersion": 1,
        "kind": "morphir-tool-release",
        "tool": { "id": "desktop", "name": "Morphir Desktop" },
        "version": "1.0.0",
        "channels": ["stable"],
        "status": "active",
        "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
        "artifacts": [{
            "targetPath": "artifacts/desktop/1.0.0/windows-aarch64.zip",
            "platform": { "os": "windows", "arch": "aarch64" },
            "archive": { "format": "zip", "entryPoint": oversized_entry_point },
            "launch": {
                "kind": "executable",
                "path": oversized_entry_point,
                "args": []
            }
        }]
    });

    let error = serde_json::from_value::<ToolReleaseRecord>(descriptor).unwrap_err();
    assert!(error.to_string().contains("declared artifact path"));
}

#[test]
fn tool_release_descriptors_reject_impossible_raw_launch_paths() {
    let descriptor = serde_json::json!({
        "schemaVersion": 1,
        "kind": "morphir-tool-release",
        "tool": { "id": "desktop", "name": "Morphir Desktop" },
        "version": "1.0.0",
        "channels": ["stable"],
        "status": "active",
        "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
        "artifacts": [{
            "targetPath": "artifacts/desktop/1.0.0/desktop.exe",
            "platform": { "os": "windows", "arch": "x86_64" },
            "archive": { "format": "raw", "entryPoint": "bin/desktop.exe" },
            "launch": { "kind": "executable", "path": "bin/desktop.exe", "args": [] }
        }]
    });

    let error = serde_json::from_value::<ToolReleaseRecord>(descriptor).unwrap_err();
    assert!(error.to_string().contains("raw and AppImage entryPoint"));
}

#[test]
fn tool_release_descriptors_restrict_appimages_to_linux() {
    let descriptor = serde_json::json!({
        "schemaVersion": 1,
        "kind": "morphir-tool-release",
        "tool": { "id": "desktop", "name": "Morphir Desktop" },
        "version": "1.0.0",
        "channels": ["stable"],
        "status": "active",
        "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
        "artifacts": [{
            "targetPath": "artifacts/desktop/1.0.0/desktop.AppImage",
            "platform": { "os": "windows", "arch": "x86_64" },
            "archive": { "format": "appimage", "entryPoint": "desktop.AppImage" },
            "launch": { "kind": "executable", "path": "desktop.AppImage", "args": [] }
        }]
    });

    let error = serde_json::from_value::<ToolReleaseRecord>(descriptor).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("AppImage artifacts require Linux")
    );
}

#[test]
fn tool_channels_and_exact_versions_resolve_deterministically() {
    let releases = vec![
        a_tool_release("1.0.0", &["stable"], "active", ">=0.4.0, <0.5.0"),
        a_tool_release(
            "1.1.0-preview.1",
            &["preview", "preview/nightly", "insiders"],
            "active",
            ">=0.4.0, <0.5.0",
        ),
        a_tool_release("0.9.0", &[], "yanked", ">=0.4.0, <0.5.0"),
    ];
    let platform = Platform::new("windows", "aarch64").unwrap();
    let cli = Version::parse("0.4.0").unwrap();

    let stable = resolve_tool(
        &releases,
        &Selection::Channel(Channel::Stable),
        &platform,
        &cli,
    )
    .unwrap();
    assert_eq!(stable.release().version().to_string(), "1.0.0");

    let preview = resolve_tool(
        &releases,
        &Selection::Channel(Channel::Preview(None)),
        &platform,
        &cli,
    )
    .unwrap();
    assert_eq!(preview.release().version().to_string(), "1.1.0-preview.1");

    let nightly = resolve_tool(
        &releases,
        &Selection::Channel(Channel::Preview(Some(
            ChannelSegment::parse("nightly").unwrap(),
        ))),
        &platform,
        &cli,
    )
    .unwrap();
    assert_eq!(nightly.release().version().to_string(), "1.1.0-preview.1");

    let yanked = resolve_tool(
        &releases,
        &Selection::Exact(Version::parse("0.9.0").unwrap()),
        &platform,
        &cli,
    )
    .unwrap();
    assert_eq!(yanked.release().status(), ToolReleaseStatus::Yanked);
}

#[test]
fn channel_resolution_skips_incompatible_tools_and_exact_revocation_is_terminal() {
    let releases = vec![
        a_tool_release("1.0.0", &["stable"], "active", ">=0.4.0, <0.5.0"),
        a_tool_release("2.0.0", &["stable"], "active", ">=0.5.0"),
        a_tool_release("0.8.0", &[], "revoked", ">=0.4.0"),
    ];
    let platform = Platform::new("windows", "aarch64").unwrap();
    let cli = Version::parse("0.4.0").unwrap();

    let stable = resolve_tool(
        &releases,
        &Selection::Channel(Channel::Stable),
        &platform,
        &cli,
    )
    .unwrap();
    assert_eq!(stable.release().version().to_string(), "1.0.0");

    let revoked = resolve_tool(
        &releases,
        &Selection::Exact(Version::parse("0.8.0").unwrap()),
        &platform,
        &cli,
    )
    .unwrap_err();
    assert!(matches!(
        revoked,
        DistributionError::RevokedToolRelease { .. }
    ));
}

#[test]
fn tool_resolution_rejects_versions_with_equal_semver_precedence() {
    let releases = vec![
        a_tool_release("1.0.0+desktop.1", &["stable"], "active", ">=0.4.0, <0.5.0"),
        a_tool_release("1.0.0+desktop.2", &["stable"], "active", ">=0.4.0, <0.5.0"),
    ];

    let error = resolve_tool(
        &releases,
        &Selection::Channel(Channel::Stable),
        &Platform::new("windows", "aarch64").unwrap(),
        &Version::parse("0.4.0").unwrap(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        DistributionError::DuplicatePrecedence { first, second }
            if first == Version::parse("1.0.0+desktop.1").unwrap()
                && second == Version::parse("1.0.0+desktop.2").unwrap()
    ));
}

#[test]
fn tool_resolution_rejects_mixed_tool_identities() {
    let companion = serde_json::from_value(serde_json::json!({
        "schemaVersion": 1,
        "kind": "morphir-tool-release",
        "tool": { "id": "companion", "name": "Morphir Companion" },
        "version": "2.0.0",
        "channels": ["stable"],
        "status": "active",
        "compatibility": { "morphirCli": ">=0.4.0, <0.5.0" },
        "artifacts": [{
            "targetPath": "artifacts/companion/2.0.0/windows-aarch64.zip",
            "platform": { "os": "windows", "arch": "aarch64" },
            "archive": { "format": "zip", "entryPoint": "companion.exe" },
            "launch": { "kind": "executable", "path": "companion.exe", "args": [] }
        }]
    }))
    .unwrap();
    let releases = vec![
        a_tool_release("1.0.0", &["stable"], "active", ">=0.4.0, <0.5.0"),
        companion,
    ];

    let error = resolve_tool(
        &releases,
        &Selection::Channel(Channel::Stable),
        &Platform::new("windows", "aarch64").unwrap(),
        &Version::parse("0.4.0").unwrap(),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        DistributionError::MixedToolIdentity { expected, actual }
            if expected == ToolId::parse("desktop").unwrap()
                && actual == ToolId::parse("companion").unwrap()
    ));
}
