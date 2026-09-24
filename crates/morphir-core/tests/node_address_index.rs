use indexmap::IndexMap;
use morphir_core::ir::{classic, v4};
use morphir_core::naming::{Name, PackageName, Path};
use morphir_core::node_address::{
    ArtifactSelector, NodeCatalog, NodeIndex, NodeResolutionError, NodeUri, Sha256Digest,
    convert_v3_node_id,
};

fn v4_record() -> v4::Distribution {
    let type_definition = v4::TypeDefinition::TypeAliasDefinition {
        type_params: vec![],
        type_expr: v4::Type::record(
            v4::TypeAttributes::default(),
            vec![v4::Field::new(
                Name::from("customerId"),
                v4::Type::unit(v4::TypeAttributes::default()),
            )],
        ),
    };
    let module = v4::ModuleDefinition {
        types: IndexMap::from([(
            "order".to_owned(),
            v4::AccessControlled {
                access: v4::Access::Public,
                value: v4::Documented::new(None, type_definition),
            },
        )]),
        values: IndexMap::new(),
        doc: None,
    };
    v4::Distribution::Library(v4::LibraryContent {
        package_name: PackageName::new(Path::new("acme/orders")),
        dependencies: IndexMap::new(),
        def: v4::PackageDefinition {
            modules: IndexMap::from([(
                "domain".to_owned(),
                v4::AccessControlled {
                    access: v4::Access::Public,
                    value: module,
                },
            )]),
        },
    })
}

#[test]
fn v3_specs_distribution_indexes_its_public_interface() {
    let distribution = classic::Distribution {
        format_version: 3,
        distribution: classic::DistributionBody::Specs(
            classic::Path::new(vec![classic::Name::from_str("Acme")]),
            vec![],
            classic::PackageSpecification {
                modules: vec![classic::package::ModuleSpecEntry {
                    path: classic::Path::new(vec![classic::Name::from_str("Domain")]),
                    specification: classic::ModuleSpecification {
                        types: vec![],
                        values: vec![],
                        doc: None,
                    },
                }],
            },
        ),
    };
    let index = NodeIndex::v3(
        &distribution,
        ArtifactSelector::Package(PackageName::new(Path::new("acme"))),
    )
    .unwrap();
    let uri = NodeUri::parse("morphir://ir/pkg/acme?format=3.1.0#/package").unwrap();
    assert!(index.resolve(&uri).is_ok());
    let module = NodeUri::parse("morphir://ir/pkg/acme?format=3.1.0#/module/domain").unwrap();
    assert!(index.resolve(&module).is_ok());
    let bytes = serde_json::to_vec(&distribution).unwrap();
    let mut catalog = NodeCatalog::new();
    catalog.add_v3_json_snapshot(&bytes, None).unwrap();
    assert!(convert_v3_node_id(&distribution, &index, "Acme:Domain").is_err());

    let patch_release = String::from_utf8(bytes)
        .unwrap()
        .replace("\"3.1.0\"", "\"3.1.1\"");
    let mut patch_catalog = NodeCatalog::new();
    let digest = patch_catalog
        .add_v3_json_snapshot(patch_release.as_bytes(), None)
        .unwrap();
    let patched = NodeUri::parse(&format!(
        "morphir://ir/pkg/acme?format=3.1.1&rev={digest}#/module/domain"
    ))
    .unwrap();
    assert!(patch_catalog.resolve(&patched).is_ok());
}

fn order_type(distribution: &mut v4::Distribution) -> &mut v4::Type {
    let v4::Distribution::Library(library) = distribution else {
        unreachable!()
    };
    let module = &mut library.def.modules.get_mut("domain").unwrap().value;
    let v4::TypeDefinition::TypeAliasDefinition { type_expr, .. } =
        &mut module.types.get_mut("order").unwrap().value.value
    else {
        unreachable!()
    };
    type_expr
}

#[test]
fn v4_index_resolves_named_field_independent_of_source_layout() {
    let distribution = v4_record();
    let index = NodeIndex::v4(&distribution).unwrap();
    let uri = NodeUri::parse("morphir://ir/pkg/acme/orders?format=4.0.0#/module/domain/type/order/type-exp/record/field/customer-id").unwrap();
    assert!(index.resolve(&uri).is_ok());
    assert!(
        index
            .addresses()
            .any(|address| address.root() == uri.root() && address.steps() == uri.steps())
    );
    for address in index.addresses() {
        let rendered = address.to_string();
        assert_eq!(NodeUri::parse(&rendered).unwrap(), *address, "{rendered}");
    }
}

#[test]
fn v4_index_rejects_duplicate_semantic_fields() {
    let mut distribution = v4_record();
    let v4::Distribution::Library(library) = &mut distribution else {
        unreachable!()
    };
    let module = &mut library.def.modules.get_mut("domain").unwrap().value;
    let v4::TypeDefinition::TypeAliasDefinition {
        type_expr: v4::Type::Record(_, fields),
        ..
    } = &mut module.types.get_mut("order").unwrap().value.value
    else {
        unreachable!()
    };
    fields.push(fields[0].clone());
    assert_eq!(
        NodeIndex::v4(&distribution).unwrap_err(),
        NodeResolutionError::AmbiguousTarget
    );
}

#[test]
fn v3_apply_children_have_distinct_addresses() {
    let body: classic::Value<classic::Attrs, classic::Type<classic::Attrs>> = classic::Value::Apply(
        classic::Type::Unit(classic::Attrs::None),
        Box::new(classic::Value::Variable(
            classic::Type::Unit(classic::Attrs::None),
            classic::Name::from_str("f"),
        )),
        Box::new(classic::Value::Variable(
            classic::Type::Unit(classic::Attrs::None),
            classic::Name::from_str("x"),
        )),
    );
    let value = classic::ValueDefinition {
        input_types: vec![classic::value::ValueArgument {
            name: classic::Name::from_str("input"),
            annotation: classic::Type::Unit(classic::Attrs::None),
            ty: classic::Type::Unit(classic::Attrs::None),
        }],
        output_type: classic::Type::Unit(classic::Attrs::default()),
        body,
    };
    let module = classic::ModuleDefinition {
        types: vec![],
        values: vec![(
            classic::Name::from_str("calculateTotal"),
            classic::AccessControlled {
                access: classic::Access::Public,
                value: classic::Documented::new("", value),
            },
        )],
        doc: None,
    };
    let distribution = classic::Distribution {
        format_version: 3,
        distribution: classic::DistributionBody::Library(
            classic::Path::new(vec![
                classic::Name::from_str("Acme"),
                classic::Name::from_str("Orders"),
            ]),
            vec![],
            classic::PackageDefinition {
                modules: vec![classic::ModuleEntry {
                    path: classic::Path::new(vec![classic::Name::from_str("Domain")]),
                    definition: classic::AccessControlled {
                        access: classic::Access::Public,
                        value: module,
                    },
                }],
            },
        ),
    };
    let index = NodeIndex::v3(
        &distribution,
        ArtifactSelector::Package(PackageName::new(Path::new("acme/orders"))),
    )
    .unwrap();
    for role in ["function", "argument"] {
        let uri = NodeUri::parse(&format!("morphir://ir/pkg/acme/orders?format=3.0.0#/module/domain/value/calculate-total/body/apply/{role}")).unwrap();
        assert!(index.resolve(&uri).is_ok(), "{role}");
    }
    let annotation = NodeUri::parse("morphir://ir/pkg/acme/orders?format=3.0.0#/module/domain/value/calculate-total/input-annotation/input").unwrap();
    assert!(index.resolve(&annotation).is_ok());
    assert_eq!(
        convert_v3_node_id(
            &distribution,
            &index,
            "Acme.Orders:Domain:calculateTotal.value#1"
        )
        .unwrap()
        .to_string(),
        "morphir://ir/pkg/acme/orders?format=3.0.0#/module/domain/value/calculate-total/body/apply/argument"
    );
    assert_eq!(
        convert_v3_node_id(
            &distribution,
            &index,
            "Acme.Orders:Domain:calculateTotal/value#0"
        )
        .unwrap()
        .to_string(),
        "morphir://ir/pkg/acme/orders?format=3.0.0#/module/domain/value/calculate-total/body/apply/function"
    );
    assert!(
        convert_v3_node_id(
            &distribution,
            &index,
            "Acme.Orders:Domain:calculateTotal.other#0"
        )
        .is_err()
    );
    for address in index.addresses() {
        let rendered = address.to_string();
        assert_eq!(NodeUri::parse(&rendered).unwrap(), *address, "{rendered}");
    }
    let bytes = serde_json::to_vec(&distribution).unwrap();
    let mut catalog = NodeCatalog::new();
    let digest = catalog.add_v3_json_snapshot(&bytes, None).unwrap();
    let pinned = NodeUri::parse(&format!(
        "morphir://ir/pkg/acme/orders?format=3.0.0&rev={digest}#/module/domain/value/calculate-total/body/apply/argument"
    ))
    .unwrap();
    assert!(catalog.resolve_node(&pinned).is_ok());
}

#[test]
fn selected_tuple_child_stales_when_shifted_but_not_when_later_sibling_added() {
    let mut distribution = v4_record();
    *order_type(&mut distribution) = v4::Type::tuple(
        v4::TypeAttributes::default(),
        vec![
            v4::Type::variable(v4::TypeAttributes::default(), Name::from("a")),
            v4::Type::unit(v4::TypeAttributes::default()),
        ],
    );
    let original = NodeIndex::v4(&distribution).unwrap();
    let selected = original
        .addresses()
        .find(|address| {
            address.steps().last() == Some(&morphir_core::node_address::NodeStep::TupleElement(1))
        })
        .unwrap()
        .clone();
    assert!(original.resolve(&selected).is_ok());

    let mut relocated = distribution.clone();
    let v4::Type::Tuple(_, relocated_elements) = order_type(&mut relocated) else {
        unreachable!()
    };
    relocated_elements[1] = v4::Type::unit(v4::TypeAttributes::with_source(
        v4::SourceLocation::new(20, 1, 20, 5),
    ));
    assert!(
        NodeIndex::v4(&relocated)
            .unwrap()
            .resolve(&selected)
            .is_ok()
    );

    let v4::Type::Tuple(_, elements) = order_type(&mut distribution) else {
        unreachable!()
    };
    elements.push(v4::Type::unit(v4::TypeAttributes::default()));
    let appended = NodeIndex::v4(&distribution).unwrap();
    assert!(appended.resolve(&selected).is_ok());

    let v4::Type::Tuple(_, elements) = order_type(&mut distribution) else {
        unreachable!()
    };
    elements.insert(
        0,
        v4::Type::variable(v4::TypeAttributes::default(), Name::from("inserted")),
    );
    let shifted = NodeIndex::v4(&distribution).unwrap();
    assert_eq!(
        shifted.resolve(&selected),
        Err(NodeResolutionError::StaleTarget)
    );
}

#[test]
fn a_case_body_guard_stales_when_its_pattern_moves() {
    let mut distribution = v4_record();
    {
        let v4::Distribution::Library(library) = &mut distribution else {
            unreachable!()
        };
        let module = &mut library.def.modules.get_mut("domain").unwrap().value;
        let pattern_match = v4::Value::PatternMatch(
            v4::ValueAttributes::default(),
            Box::new(v4::Value::Unit(v4::ValueAttributes::default())),
            vec![
                v4::PatternCase::new(
                    v4::Pattern::WildcardPattern(v4::ValueAttributes::default()),
                    v4::Value::Unit(v4::ValueAttributes::default()),
                ),
                v4::PatternCase::new(
                    v4::Pattern::UnitPattern(v4::ValueAttributes::default()),
                    v4::Value::Unit(v4::ValueAttributes::default()),
                ),
            ],
        );
        module.values.insert(
            "evaluate".into(),
            v4::AccessControlled {
                access: v4::Access::Public,
                value: v4::Documented::new(
                    None,
                    v4::ValueDefinition {
                        input_types: IndexMap::new(),
                        output_type: Some(v4::Type::unit(v4::TypeAttributes::default())),
                        body: v4::ValueBody::Expression(pattern_match),
                    },
                ),
            },
        );
    }
    let before = NodeIndex::v4(&distribution).unwrap();
    let body_uri = before
        .addresses()
        .find(|uri| {
            uri.steps().last()
                == Some(&morphir_core::node_address::NodeStep::PatternMatchCaseBody(
                    0,
                ))
        })
        .unwrap()
        .clone();
    assert!(before.resolve(&body_uri).is_ok());
    let v4::Distribution::Library(library) = &mut distribution else {
        unreachable!()
    };
    let module = &mut library.def.modules.get_mut("domain").unwrap().value;
    let v4::ValueBody::Expression(v4::Value::PatternMatch(_, _, cases)) =
        &mut module.values.get_mut("evaluate").unwrap().value.value.body
    else {
        unreachable!()
    };
    cases.swap(0, 1);
    assert_eq!(
        NodeIndex::v4(&distribution).unwrap().resolve(&body_uri),
        Err(NodeResolutionError::StaleTarget)
    );
}

#[test]
fn pinned_resolution_uses_exact_snapshot_while_current_can_be_ambiguous() {
    let distribution = v4_record();
    let file = v4::IRFile {
        format_version: v4::FormatVersion::Integer(4),
        distribution: distribution.clone(),
    };
    let bytes = morphir_core::ir::json::write_ir_file(&file).into_bytes();
    let mut catalog = NodeCatalog::new();
    catalog.add_current(NodeIndex::v4(&distribution).unwrap());
    catalog.add_current(NodeIndex::v4(&distribution).unwrap());
    let current =
        NodeUri::parse("morphir://ir/pkg/acme/orders?format=4.0.0#/module/domain/type/order")
            .unwrap();
    assert_eq!(
        catalog.resolve(&current),
        Err(NodeResolutionError::AmbiguousArtifact)
    );
    let digest = catalog.add_v4_json_snapshot(&bytes, None).unwrap();
    let pinned = NodeUri::parse(&format!(
        "morphir://ir/pkg/acme/orders?format=4.0.0&rev={digest}#/module/domain/type/order"
    ))
    .unwrap();
    assert!(catalog.resolve(&pinned).is_ok());
    let absent = NodeUri::parse(&format!(
        "morphir://ir/pkg/acme/orders?format=4.0.0&rev=sha256:{}#/module/domain/type/order",
        "a".repeat(64)
    ))
    .unwrap();
    assert_eq!(
        catalog.resolve(&absent),
        Err(NodeResolutionError::RevisionUnavailable)
    );
    assert_eq!(
        catalog.add_v4_json_snapshot(
            &bytes,
            Some(&Sha256Digest::parse(&format!("sha256:{}", "a".repeat(64))).unwrap())
        ),
        Err(NodeResolutionError::RevisionMismatch)
    );
    assert_eq!(
        catalog.resolve_node(&pinned).unwrap().semantic_value,
        NodeIndex::v4_file(&file)
            .unwrap()
            .resolve_node(&current)
            .unwrap()
            .semantic_value
    );
}

#[test]
fn equivalent_v4_json_and_yaml_yield_identical_semantic_addresses() {
    let file = v4::IRFile {
        format_version: v4::FormatVersion::Integer(4),
        distribution: v4_record(),
    };
    let json = morphir_core::ir::json::write_ir_file(&file);
    let yaml = morphir_core::ir::yaml::write_ir_file(&file);
    let (from_json, _) = morphir_core::ir::json::read_ir_file(&json).unwrap();
    let (from_yaml, _) = morphir_core::ir::yaml::read_ir_file(&yaml).unwrap();
    let addresses = |file: &v4::IRFile| {
        let index = NodeIndex::v4_file(file).unwrap();
        let mut values = index
            .addresses()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        values.sort();
        values
    };
    let target = NodeUri::parse("morphir://ir/pkg/acme/orders?format=4.0.0#/module/domain/type/order/type-exp/record/field/customer-id").unwrap();
    let selected = |file: &v4::IRFile| {
        NodeIndex::v4_file(file)
            .unwrap()
            .resolve_node(&target)
            .unwrap()
            .clone()
    };
    assert_eq!(addresses(&from_json), addresses(&from_yaml));
    assert_eq!(addresses(&file), addresses(&from_json));
    assert_eq!(selected(&from_json), selected(&from_yaml));
    for profile in [
        morphir_core::ir::layout::Profile::Json,
        morphir_core::ir::layout::Profile::Yaml,
    ] {
        let files = morphir_core::ir::layout::write_tree(
            &file,
            &morphir_core::ir::layout::TreePolicy {
                profile,
                path_budget: 4000,
            },
        )
        .unwrap()
        .into_iter()
        .collect();
        let (from_tree, _) = morphir_core::ir::layout::read_tree(&files, profile).unwrap();
        assert_eq!(addresses(&file), addresses(&from_tree));
        assert_eq!(selected(&file), selected(&from_tree));
    }
    let mut patched = file.clone();
    patched.format_version = v4::FormatVersion::String("4.0.1".to_owned());
    let patch_index = NodeIndex::v4_file(&patched).unwrap();
    assert!(
        patch_index
            .addresses()
            .all(|address| address.format().to_exact_string() == "4.0.1")
    );
}

#[test]
fn workspace_alias_can_select_a_local_v4_artifact() {
    let distribution = v4_record();
    let index = NodeIndex::v4_with_selector(
        &distribution,
        ArtifactSelector::Workspace("orders".to_owned()),
    )
    .unwrap();
    let address =
        NodeUri::parse("morphir://ir/workspace/orders?format=4.0.0#/module/domain/type/order")
            .unwrap();
    assert!(index.resolve(&address).is_ok());
    assert_eq!(
        NodeIndex::v4_with_selector(
            &distribution,
            ArtifactSelector::Package(PackageName::new(Path::new("another/package")))
        )
        .unwrap_err(),
        NodeResolutionError::ArtifactMismatch
    );
}

#[test]
fn v3_legacy_type_conversion_checks_alias_and_custom_type_shape() {
    let unit = || classic::Type::Unit(classic::Attrs::None);
    let alias = classic::TypeDefinition::Alias(
        vec![],
        classic::Type::Record(
            classic::Attrs::None,
            vec![classic::Field {
                name: classic::Name::from_str("customerId"),
                ty: classic::Type::Tuple(classic::Attrs::None, vec![unit(), unit()]),
            }],
        ),
    );
    let custom = classic::TypeDefinition::Custom(
        vec![],
        classic::AccessControlled {
            access: classic::Access::Public,
            value: vec![classic::Constructor {
                name: classic::Name::from_str("accountId"),
                args: vec![(classic::Name::from_str("value"), unit())],
            }],
        },
    );
    let module = classic::ModuleDefinition {
        types: vec![
            (
                classic::Name::from_str("order"),
                classic::AccessControlled {
                    access: classic::Access::Public,
                    value: classic::Documented::new("", alias),
                },
            ),
            (
                classic::Name::from_str("accountId"),
                classic::AccessControlled {
                    access: classic::Access::Public,
                    value: classic::Documented::new("", custom),
                },
            ),
        ],
        values: vec![],
        doc: None,
    };
    let distribution = classic::Distribution {
        format_version: 3,
        distribution: classic::DistributionBody::Library(
            classic::Path::new(vec![
                classic::Name::from_str("Acme"),
                classic::Name::from_str("Orders"),
            ]),
            vec![],
            classic::PackageDefinition {
                modules: vec![classic::ModuleEntry {
                    path: classic::Path::new(vec![classic::Name::from_str("Domain")]),
                    definition: classic::AccessControlled {
                        access: classic::Access::Public,
                        value: module,
                    },
                }],
            },
        ),
    };
    let index = NodeIndex::v3(
        &distribution,
        ArtifactSelector::Package(PackageName::new(Path::new("acme/orders"))),
    )
    .unwrap();
    for suffix in [".type", "/type"] {
        let uri = convert_v3_node_id(
            &distribution,
            &index,
            &format!("Acme.Orders:Domain:order{suffix}#customerId"),
        )
        .unwrap();
        assert_eq!(
            uri.to_string(),
            "morphir://ir/pkg/acme/orders?format=3.0.0#/module/domain/type/order/type-exp/record/field/customer-id"
        );
        let nested = convert_v3_node_id(
            &distribution,
            &index,
            &format!("Acme.Orders:Domain:order{suffix}#customerId:1"),
        )
        .unwrap();
        assert!(
            nested
                .to_string()
                .contains("/record/field/customer-id/tuple/element/1")
        );
        assert!(nested.guard().is_some());
        assert!(index.resolve(&nested).is_ok());
    }
    let custom =
        convert_v3_node_id(&distribution, &index, "Acme.Orders:Domain:accountId.type").unwrap();
    assert_eq!(
        custom.to_string(),
        "morphir://ir/pkg/acme/orders?format=3.0.0&guard=sha256:614fc05ccde1917095dcdfaf1123aa7acbbf19dcef28f1a40297173396288d79#/module/domain/type/account-id/constructor/account-id/argument/0"
    );
    assert!(index.resolve(&custom).is_ok());
    assert!(
        convert_v3_node_id(
            &distribution,
            &index,
            "Acme.Orders:Domain:order.type#missingField"
        )
        .is_err()
    );
}
