use std::collections::BTreeMap;

use serde_json::json;

use super::{
    EffectiveConfigCollector, discover_internal,
    layers::{MemberConfigLayers, member_effective_config, without_project_or_workspace},
};
use crate::{
    DiscoveryRequest, FileEntry, FileTree, RelativePath, WORKSPACE_DISCOVERY_PROTOCOL,
    WORKSPACE_SYMLINK_UNSUPPORTED,
};

#[derive(Default)]
struct CountingCollector {
    roots: usize,
    projects: Vec<RelativePath>,
}

impl EffectiveConfigCollector for CountingCollector {
    fn root(&mut self, _effective: &serde_json::Value) {
        self.roots += 1;
    }

    fn project(&mut self, path: &RelativePath, _effective: &serde_json::Value) {
        self.projects.push(path.clone());
    }
}

fn collection_request() -> DiscoveryRequest {
    DiscoveryRequest {
        protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
        development_root: FileTree {
            entries: BTreeMap::from([
                (RelativePath::root(), FileEntry::Directory),
                (
                    RelativePath::parse("morphir.toml").unwrap(),
                    FileEntry::File {
                        text:
                            "[workspace]\nmembers = ['packages/*']\n[project]\nname = 'acme/root'\n"
                                .to_owned(),
                    },
                ),
                (
                    RelativePath::parse("packages/orders").unwrap(),
                    FileEntry::Directory,
                ),
                (
                    RelativePath::parse("packages/orders/morphir.toml").unwrap(),
                    FileEntry::File {
                        text: "[project]\nname = 'acme/orders'\n".to_owned(),
                    },
                ),
            ]),
        },
        morphir_home: None,
        system_config: None,
        environment: BTreeMap::new(),
        cli_overlay: json!({}),
    }
}

#[test]
fn effective_configs_are_collected_only_when_a_sink_is_supplied() {
    let request = collection_request();
    let ordinary = discover_internal(request.clone(), None).unwrap();
    let mut collector = CountingCollector::default();
    let detailed = discover_internal(request, Some(&mut collector)).unwrap();

    assert_eq!(ordinary, detailed);
    assert_eq!(collector.roots, 1);
    assert_eq!(
        collector.projects,
        [
            RelativePath::root(),
            RelativePath::parse("packages/orders").unwrap(),
        ]
    );
}

#[test]
fn portable_discovery_rejects_unmaterialized_symlinks_in_every_mount() {
    enum Mount {
        DevelopmentRoot,
        MorphirHome,
        SystemConfig,
    }

    for (mount, context) in [
        (Mount::DevelopmentRoot, "development root"),
        (Mount::MorphirHome, "Morphir Home"),
        (Mount::SystemConfig, "system configuration"),
    ] {
        let first = RelativePath::parse("a-link").unwrap();
        let first_target = RelativePath::parse("targets/first").unwrap();
        let links = FileTree {
            entries: BTreeMap::from([
                (RelativePath::root(), FileEntry::Directory),
                (
                    RelativePath::parse("z-link").unwrap(),
                    FileEntry::Symlink {
                        target: RelativePath::parse("targets/last").unwrap(),
                    },
                ),
                (
                    first.clone(),
                    FileEntry::Symlink {
                        target: first_target.clone(),
                    },
                ),
            ]),
        };
        let mut request = collection_request();
        match mount {
            Mount::DevelopmentRoot => request.development_root = links,
            Mount::MorphirHome => request.morphir_home = Some(links),
            Mount::SystemConfig => request.system_config = Some(links),
        }

        let failure = discover_internal(request, None).unwrap_err();

        assert_eq!(failure.code, WORKSPACE_SYMLINK_UNSUPPORTED);
        assert_eq!(failure.path, Some(first.clone()));
        assert!(failure.message.contains(context));
        assert!(failure.message.contains(first.as_str()));
        assert!(failure.message.contains(first_target.as_str()));
        assert!(!failure.message.contains("z-link"));
    }
}

#[test]
fn member_merge_inherits_only_shared_root_user_sections() {
    let empty = json!({});
    let root_user = json!({
        "workspace": { "name": "root-user-workspace" },
        "project": {
            "name": "root/user",
            "version": "2.0.0",
            "source_directory": "root-user-src"
        },
        "ir": { "strict_mode": true, "mode": "root-user" }
    });
    let shared_root_user = without_project_or_workspace(&root_user);
    let member_primary = json!({
        "project": {
            "name": "member/primary",
            "version": "1.0.0",
            "source_directory": "member-src"
        },
        "ir": { "format_version": 3, "mode": "member-primary" }
    });
    let member_user = json!({
        "project": { "version": "3.0.0" },
        "ir": { "mode": "member-user" }
    });

    let effective = member_effective_config(MemberConfigLayers {
        system: &empty,
        global: &empty,
        shared_workspace: &empty,
        member_primary: &member_primary,
        shared_workspace_user: &shared_root_user,
        member_user: Some(&member_user),
        environment: &empty,
        cli_overlay: &empty,
    });

    assert_eq!(effective["project"]["name"], "member/primary");
    assert_eq!(effective["project"]["version"], "3.0.0");
    assert_eq!(effective["project"]["source_directory"], "member-src");
    assert!(effective.get("workspace").is_none());
    assert_eq!(effective["ir"]["strict_mode"], true);
    assert_eq!(effective["ir"]["format_version"], 3);
    assert_eq!(effective["ir"]["mode"], "member-user");
}
