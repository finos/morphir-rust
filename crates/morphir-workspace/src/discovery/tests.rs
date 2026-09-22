use std::collections::BTreeMap;

use serde_json::json;

use super::{
    EffectiveConfigCollector, discover_internal,
    layers::{MemberConfigLayers, member_effective_config, without_project_or_workspace},
};
use crate::{
    DiscoveryPurpose, DiscoveryRequest, FileEntry, FileTree, ProjectOrigin, ProjectSource,
    RelativePath, SourceSelection, WORKSPACE_CONFIG_INVALID, WORKSPACE_DISCOVERY_PROTOCOL,
    WORKSPACE_LANGUAGE_ID_EMPTY, WORKSPACE_PROJECT_NAME_EMPTY, WORKSPACE_PURPOSE_UNSUPPORTED,
    WORKSPACE_SELECTION_DUPLICATE, WORKSPACE_SELECTION_EMPTY, WORKSPACE_SELECTION_INVALID,
    WORKSPACE_SELECTION_NAME_REQUIRED, WORKSPACE_SELECTION_OUTSIDE_ROOT,
    WORKSPACE_SYMLINK_UNSUPPORTED, discover, discover_with_details,
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
        purpose: Default::default(),
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

fn gleam_source(path: &str) -> (RelativePath, FileEntry) {
    (
        RelativePath::parse(path).unwrap(),
        FileEntry::File {
            text: "pub type Customer {\n  Customer(id: String)\n}\n".to_owned(),
        },
    )
}

fn ad_hoc_request_with_entries(
    entries: BTreeMap<RelativePath, FileEntry>,
    project: ProjectSource,
    sources: SourceSelection,
) -> DiscoveryRequest {
    DiscoveryRequest {
        protocol_version: WORKSPACE_DISCOVERY_PROTOCOL,
        development_root: FileTree { entries },
        morphir_home: None,
        system_config: None,
        environment: BTreeMap::new(),
        cli_overlay: json!({}),
        purpose: DiscoveryPurpose::AdHocSources {
            project,
            sources,
            language_id: "gleam".to_owned(),
        },
    }
}

fn ad_hoc_request(paths: Vec<RelativePath>, root: RelativePath) -> DiscoveryRequest {
    let (source, entry) = gleam_source("models/domain/customer.gleam");
    ad_hoc_request_with_entries(
        BTreeMap::from([
            (RelativePath::root(), FileEntry::Directory),
            (source, entry),
        ]),
        ProjectSource::Synthesized,
        SourceSelection { root, paths },
    )
}

/// A selection with no manifest yields one synthesized project, and the root
/// the request carried is the one reported. Nested on purpose: a root-level
/// fixture cannot tell a correct root from a recomputed one.
///
/// Two paths, in two different directories, sharing a basename: this is the
/// plan's own motivating collision example. Recomputing the root from the
/// files (rather than carrying `sources.root` verbatim) would name both
/// modules `customer`. The selection root stays `models` and `inputs`
/// preserves the order the request gave, proving both never happen here.
///
/// Multi-source, so this now requires an explicit overlay name (see the
/// single-source cardinality rule below); the asserted name is exactly the
/// overlay value, never anything derived from the paths.
#[test]
fn ad_hoc_sources_without_a_manifest_yield_a_synthesized_project() {
    let domain = gleam_source("models/domain/customer.gleam");
    let party = gleam_source("models/party/customer.gleam");
    let paths = vec![domain.0.clone(), party.0.clone()];
    let root = RelativePath::parse("models").unwrap();
    let entries = BTreeMap::from([(RelativePath::root(), FileEntry::Directory), domain, party]);
    let mut request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Synthesized,
        SourceSelection {
            root: root.clone(),
            paths: paths.clone(),
        },
    );
    request.cli_overlay = json!({ "project": { "name": "acme/models" } });

    let snapshot = discover(request)
        .into_result()
        .expect("an ad-hoc selection with no manifest should synthesize a project");

    assert_eq!(snapshot.projects.len(), 1);
    let project = &snapshot.projects[0];
    assert_eq!(project.origin, ProjectOrigin::Synthesized { inputs: paths });
    assert_eq!(project.relative_path, root);
    assert_eq!(project.source_directory, RelativePath::root());
    // The exposed modules are intermediate data, not a final contract:
    // deriving them means parsing source, which is the provider's job. A
    // later provider-synthesis step fills these in; this task must not.
    assert_eq!(project.name, "acme/models");
    assert_eq!(project.exposed_modules, None);
    assert_eq!(project.config_anchor, None);
    assert_eq!(project.version, None);
    assert_eq!(project.state, crate::ProjectState::Unloaded);
    assert_eq!(snapshot.config_anchor, None);
}

/// An explicit `cli_overlay.project.name` reaches the synthesized snapshot
/// unchanged. Without this, discovery writes an empty name unconditionally
/// and a provider cannot tell "no name was given" from "a name was given and
/// lost on the way here".
#[test]
fn an_explicit_package_name_reaches_the_synthesized_snapshot() {
    let (path, entry) = gleam_source("models/domain/customer.gleam");
    let root = RelativePath::parse("models").unwrap();
    let entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (path.clone(), entry),
    ]);
    let mut request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Synthesized,
        SourceSelection {
            root: root.clone(),
            paths: vec![path],
        },
    );
    request.cli_overlay = json!({ "project": { "name": "acme/orders" } });

    let snapshot = discover(request)
        .into_result()
        .expect("an explicit overlay name should synthesize successfully");

    assert_eq!(snapshot.projects.len(), 1);
    assert_eq!(snapshot.projects[0].name, "acme/orders");
}

/// An unnamed synthesized selection must contain exactly one source, because
/// there is nothing to derive a name from otherwise. A *named* one may
/// contain several (proven above).
#[test]
fn an_unnamed_synthesized_selection_requires_exactly_one_source() {
    let domain = gleam_source("models/domain/customer.gleam");
    let party = gleam_source("models/party/customer.gleam");
    let paths = vec![domain.0.clone(), party.0.clone()];
    let root = RelativePath::parse("models").unwrap();
    let entries = BTreeMap::from([(RelativePath::root(), FileEntry::Directory), domain, party]);
    let request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Synthesized,
        SourceSelection {
            root: root.clone(),
            paths,
        },
    );

    let error = discover(request).into_result().unwrap_err();

    assert_eq!(error.code, WORKSPACE_SELECTION_NAME_REQUIRED);
    assert!(error.message.contains(root.as_str()));
}

/// A non-string `cli_overlay.project.name` is a structurally invalid
/// override, not an absent one: it must not be silently treated as "no name
/// was given" and fall through to the cardinality rule.
#[test]
fn ad_hoc_overlay_project_name_must_be_a_string() {
    let (path, entry) = gleam_source("models/domain/customer.gleam");
    let root = RelativePath::parse("models").unwrap();
    let entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (path.clone(), entry),
    ]);
    let mut request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Synthesized,
        SourceSelection {
            root: root.clone(),
            paths: vec![path],
        },
    );
    request.cli_overlay = json!({ "project": { "name": 42 } });

    let error = discover(request).into_result().unwrap_err();

    assert_eq!(error.code, WORKSPACE_CONFIG_INVALID);
    assert!(error.message.contains("project.name"));
}

/// A whitespace-only `cli_overlay.project.name` is rejected outright rather
/// than treated as "no name was given": an override this visibly broken
/// deserves a loud diagnostic, not a silent fallback to the single-source
/// cardinality rule.
#[test]
fn ad_hoc_overlay_project_name_whitespace_only_is_rejected() {
    let (path, entry) = gleam_source("models/domain/customer.gleam");
    let root = RelativePath::parse("models").unwrap();
    let entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (path.clone(), entry),
    ]);
    let mut request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Synthesized,
        SourceSelection {
            root: root.clone(),
            paths: vec![path],
        },
    );
    request.cli_overlay = json!({ "project": { "name": "   " } });

    let error = discover(request).into_result().unwrap_err();

    assert_eq!(error.code, WORKSPACE_PROJECT_NAME_EMPTY);
}

#[test]
fn ad_hoc_empty_selection_is_rejected() {
    let root = RelativePath::parse("models").unwrap();
    let request = ad_hoc_request(Vec::new(), root);

    let error = discover(request).into_result().unwrap_err();

    assert_eq!(error.code, WORKSPACE_SELECTION_EMPTY);
    assert!(error.message.contains("models"));
}

#[test]
fn ad_hoc_selected_path_naming_a_directory_is_rejected() {
    let directory = RelativePath::parse("models/domain").unwrap();
    let entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (directory.clone(), FileEntry::Directory),
    ]);
    let request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Synthesized,
        SourceSelection {
            root: RelativePath::parse("models").unwrap(),
            paths: vec![directory.clone()],
        },
    );

    let error = discover(request).into_result().unwrap_err();

    assert_eq!(error.code, WORKSPACE_SELECTION_INVALID);
    assert!(error.message.contains(directory.as_str()));
}

#[test]
fn ad_hoc_selected_path_naming_nothing_is_rejected() {
    let missing = RelativePath::parse("models/domain/missing.gleam").unwrap();
    let entries = BTreeMap::from([(RelativePath::root(), FileEntry::Directory)]);
    let request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Synthesized,
        SourceSelection {
            root: RelativePath::parse("models").unwrap(),
            paths: vec![missing.clone()],
        },
    );

    let error = discover(request).into_result().unwrap_err();

    assert_eq!(error.code, WORKSPACE_SELECTION_INVALID);
    assert!(error.message.contains(missing.as_str()));
}

/// Decision: duplicate paths in a selection are rejected. The inputs are the
/// synthesized project's identity, so a repeat is a caller mistake that would
/// otherwise produce a duplicate module silently.
#[test]
fn ad_hoc_duplicate_selected_path_is_rejected() {
    let (path, entry) = gleam_source("models/domain/customer.gleam");
    let entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (path.clone(), entry),
    ]);
    let request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Synthesized,
        SourceSelection {
            root: RelativePath::parse("models").unwrap(),
            paths: vec![path.clone(), path.clone()],
        },
    );

    let error = discover(request).into_result().unwrap_err();

    assert_eq!(error.code, WORKSPACE_SELECTION_DUPLICATE);
    assert!(error.message.contains(path.as_str()));
}

/// A naive string-prefix confinement check would wave this through: the path
/// shares the text prefix `models` with the root, but `models-other` is a
/// sibling directory, not a child of `models`.
#[test]
fn ad_hoc_selected_path_sharing_a_string_prefix_with_root_is_rejected() {
    let (path, entry) = gleam_source("models-other/file.gleam");
    let entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (path.clone(), entry),
    ]);
    let request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Synthesized,
        SourceSelection {
            root: RelativePath::parse("models").unwrap(),
            paths: vec![path.clone()],
        },
    );

    let error = discover(request).into_result().unwrap_err();

    assert_eq!(error.code, WORKSPACE_SELECTION_OUTSIDE_ROOT);
    assert!(error.message.contains(path.as_str()));
}

/// The mount root `.` legitimately contains every confined path. A
/// naive check that required a non-empty root prefix would reject the
/// whole tree; segment comparison must not.
#[test]
fn ad_hoc_selection_root_of_dot_contains_every_path() {
    let (path, entry) = gleam_source("models/domain/customer.gleam");
    let entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (path.clone(), entry),
    ]);
    let request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Synthesized,
        SourceSelection {
            root: RelativePath::root(),
            paths: vec![path.clone()],
        },
    );

    let snapshot = discover(request)
        .into_result()
        .expect("a `.` selection root should confine, not reject, every selected path");

    assert_eq!(snapshot.projects.len(), 1);
    assert_eq!(snapshot.projects[0].relative_path, RelativePath::root());
}

/// `ProjectSource::Manifest` is not implemented in this task: providers can't
/// synthesize a manifest yet. The gap is loud rather than silent.
#[test]
fn ad_hoc_manifest_project_source_is_explicitly_unsupported() {
    let manifest_path = RelativePath::parse("models/morphir.toml").unwrap();
    let (path, entry) = gleam_source("models/domain/customer.gleam");
    let entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (path.clone(), entry),
    ]);
    let request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Manifest {
            path: manifest_path.clone(),
        },
        SourceSelection {
            root: RelativePath::parse("models").unwrap(),
            paths: vec![path],
        },
    );

    let error = discover(request).into_result().unwrap_err();

    assert_eq!(error.code, WORKSPACE_PURPOSE_UNSUPPORTED);
    assert!(error.message.contains("manifest"));
    assert!(error.message.contains(manifest_path.as_str()));
}

/// `discover_with_details` must not panic on the ad-hoc path: the collector
/// needs a root effective config, which the ad-hoc case now supplies by
/// collecting defaults, environment and the CLI overlay directly, since
/// there is no workspace layer to collect instead.
#[test]
fn ad_hoc_discover_with_details_collects_defaults_and_overlay_without_panicking() {
    let path = RelativePath::parse("models/domain/customer.gleam").unwrap();
    let root = RelativePath::parse("models").unwrap();
    let mut request = ad_hoc_request(vec![path.clone()], root.clone());
    request.cli_overlay = json!({ "ir": { "mode": "cli-overlay" } });

    let details = discover_with_details(request)
        .expect("the ad-hoc path must collect a root config, not panic in `finish`");

    assert_eq!(details.root_effective["ir"]["mode"], "cli-overlay");
    assert_eq!(
        details.project_effective[&root]["ir"]["mode"],
        "cli-overlay"
    );
    assert_eq!(details.snapshot.projects.len(), 1);
}

/// The spec's standalone rule: a manifest present in the tree has no effect
/// on an ad-hoc synthesized request. This is the rule a later change is most
/// likely to erode, so it is pinned directly.
#[test]
fn ad_hoc_ignores_a_manifest_present_in_the_tree() {
    let (path, entry) = gleam_source("models/domain/customer.gleam");
    let entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (path.clone(), entry),
        (
            RelativePath::parse("morphir.toml").unwrap(),
            FileEntry::File {
                text: "[workspace]\nmembers = [\"packages/*\"]\n[project]\nname = \"acme/root\"\nsource_directory = \"src\"\n"
                    .to_owned(),
            },
        ),
    ]);
    let root = RelativePath::parse("models").unwrap();
    let request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Synthesized,
        SourceSelection {
            root: root.clone(),
            paths: vec![path.clone()],
        },
    );

    let snapshot = discover(request)
        .into_result()
        .expect("an ad-hoc request should succeed even with a manifest present in the tree");

    assert_eq!(snapshot.projects.len(), 1);
    let project = &snapshot.projects[0];
    assert_eq!(
        project.origin,
        ProjectOrigin::Synthesized { inputs: vec![path] }
    );
    assert_eq!(project.relative_path, root);
    assert_eq!(project.name, "");
    assert_eq!(snapshot.config_anchor, None);
}

/// `discover_with_details` at `sources.root == "."` combines two invariants
/// each existing test only covers half of: it must not panic collecting a
/// root config, and `DetailsCollector::finish` must re-insert `"."` into
/// `project_effective`. That re-insertion matters beyond this crate:
/// `morphir-daemon`'s `Workspace::from_discovery` calls
/// `project_configs.remove(&project.relative_path).expect(...)` and panics if
/// the entry is missing, so a project whose `relative_path` is the mount root
/// must still have an effective config recorded for it.
#[test]
fn ad_hoc_discover_with_details_at_root_reinserts_root_into_project_effective() {
    let path = RelativePath::parse("models/domain/customer.gleam").unwrap();
    let root = RelativePath::root();
    let request = ad_hoc_request(vec![path], root.clone());

    let details = discover_with_details(request)
        .expect("the ad-hoc path at `.` must collect a root config, not panic in `finish`");

    assert_eq!(details.snapshot.projects.len(), 1);
    assert_eq!(details.snapshot.projects[0].relative_path, root);
    assert_eq!(
        details.project_effective.get(&root),
        Some(&details.root_effective)
    );
}

/// A caller sending `root: "models", paths: ["models"]` is reachable: the
/// path names the root itself. `path_is_under_root` rejects it because it
/// requires the path to be strictly deeper than the root, so this never
/// reaches the file-existence check — the path is a directory, not a file,
/// and would fail differently there. That ordering is what makes this safe;
/// pin the diagnostic so a later refactor that reorders the checks is caught.
#[test]
fn ad_hoc_selected_path_equal_to_root_is_rejected_by_confinement_not_file_existence() {
    let root = RelativePath::parse("models").unwrap();
    let entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (root.clone(), FileEntry::Directory),
    ]);
    let request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Synthesized,
        SourceSelection {
            root: root.clone(),
            paths: vec![root.clone()],
        },
    );

    let error = discover(request).into_result().unwrap_err();

    assert_eq!(error.code, WORKSPACE_SELECTION_OUTSIDE_ROOT);
    assert!(error.message.contains(root.as_str()));
}

/// An empty language id is a caller mistake distinct from an unconfined or
/// missing path, and gets its own diagnostic rather than surfacing as a
/// confusing downstream failure.
#[test]
fn ad_hoc_empty_language_id_is_rejected() {
    let (path, entry) = gleam_source("models/domain/customer.gleam");
    let entries = BTreeMap::from([
        (RelativePath::root(), FileEntry::Directory),
        (path.clone(), entry),
    ]);
    let mut request = ad_hoc_request_with_entries(
        entries,
        ProjectSource::Synthesized,
        SourceSelection {
            root: RelativePath::parse("models").unwrap(),
            paths: vec![path],
        },
    );
    let DiscoveryPurpose::AdHocSources {
        language_id: purpose_language_id,
        ..
    } = &mut request.purpose
    else {
        unreachable!("ad_hoc_request_with_entries always builds an AdHocSources purpose")
    };
    purpose_language_id.clear();

    let error = discover(request).into_result().unwrap_err();

    assert_eq!(error.code, WORKSPACE_LANGUAGE_ID_EMPTY);
    assert!(error.message.contains("models"));
}
