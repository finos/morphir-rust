use super::super::*;
use pretty_assertions::assert_eq;

#[test]
fn linked_schema_ownership_follows_declarations_across_cross_edges_and_cycles() {
    let owned_root = "acme/customer:customer#a-root";
    let owned_shared = "acme/customer:customer#z-owned-shared";
    let dependency = "acme/shared:types#dependency";
    let input = {
        let mut package = package(vec![
            alias(
                owned_root,
                "a-root",
                TypeExpr::Record(vec![field("dependency", reference(dependency, vec![]))]),
            ),
            alias(
                owned_shared,
                "z-owned-shared",
                TypeExpr::Record(vec![field("cycle", reference(dependency, vec![]))]),
            ),
        ]);
        package.dependencies = vec![ProjectionDependency {
            package_name: "acme/shared".to_owned(),
            modules: vec![ProjectionModule {
                path: vec!["types".to_owned()],
                types: vec![alias(
                    dependency,
                    "dependency",
                    TypeExpr::Record(vec![field("owned", reference(owned_shared, vec![]))]),
                )],
                values: Vec::new(),
                doc: None,
            }],
        }];
        package
    };
    let linked = project(
        &input,
        &AvroOptions {
            dependencies: Dependencies::Linked,
            ..AvroOptions::default()
        },
    )
    .unwrap();
    assert!(
        linked
            .named_schema("acme.customer.customer.ARoot")
            .is_some()
    );
    assert!(
        linked
            .named_schema("acme.customer.customer.ZOwnedShared")
            .is_some()
    );
    assert!(
        linked
            .linked_schema("acme.shared.types.Dependency")
            .is_some()
    );
    assert_eq!(linked.linked_schemas().len(), 1);

    let self_contained = project(&input, &AvroOptions::default()).unwrap();
    assert!(self_contained.linked_schemas().is_empty());
    assert!(
        self_contained
            .named_schema("acme.shared.types.Dependency")
            .is_some()
    );
}
