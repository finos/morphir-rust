//! Name resolution: prelude implicit imports, aliases, in-package modules,
//! dependencies, ambiguity, access and interface digests.

use std::sync::LazyLock;

use morphir_elm_binding::{ast::*, frontend::resolve::*, prelude, resolved::*, span::Span};

static PACKAGE: LazyLock<Vec<String>> =
    LazyLock::new(|| vec!["local".to_string(), "example".to_string()]);

fn sp() -> Span {
    Span { start: 0, end: 0 }
}

fn r(name: &str) -> TypeExpr {
    TypeExpr::Ref {
        module: vec![],
        name: name.into(),
        args: vec![],
        span: sp(),
    }
}

fn q(module: &[&str], name: &str) -> TypeExpr {
    TypeExpr::Ref {
        module: module.iter().map(|s| s.to_string()).collect(),
        name: name.into(),
        args: vec![],
        span: sp(),
    }
}

fn alias(name: &str, body: TypeExpr) -> TypeDecl {
    TypeDecl::Alias {
        name: name.into(),
        params: vec![],
        body,
        doc: None,
        span: sp(),
    }
}

fn module(name: &[&str], exposing: Exposing, imports: Vec<Import>, types: Vec<TypeDecl>) -> Module {
    Module {
        name: name.iter().map(|s| s.to_string()).collect(),
        exposing,
        imports,
        doc: None,
        types,
        skipped_values: vec![],
        span: sp(),
    }
}

fn no_modules(_: &[String]) -> Option<Interface> {
    None
}

fn scope<'a>(
    prelude: &'a prelude::Prelude,
    pkg_modules: &'a dyn Fn(&[String]) -> Option<Interface>,
    deps: &'a [DependencyInterface],
) -> Scope<'a> {
    Scope {
        package: PACKAGE.as_slice(),
        prelude,
        package_modules: pkg_modules,
        dependencies: deps,
    }
}

#[test]
fn int_resolves_to_morphir_sdk_basics_through_the_prelude() {
    let p = prelude::builtin("elm-core").unwrap();
    let m = module(&["A"], Exposing::All, vec![], vec![alias("Id", r("Int"))]);
    let resolved = resolve(&m, Access::Public, &scope(&p, &no_modules, &[])).unwrap();
    let ResolvedBody::Alias(RType::Ref(fq, _)) = &resolved.types[0].body else {
        panic!()
    };
    assert_eq!(
        fq,
        &FqName {
            package: vec!["Morphir".into(), "SDK".into()],
            module: vec!["Basics".into()],
            name: "Int".into()
        }
    );
    assert!(resolved.depends_on.is_empty());
}

#[test]
fn none_prelude_rejects_int() {
    let p = prelude::builtin("none").unwrap();
    let m = module(&["A"], Exposing::All, vec![], vec![alias("Id", r("Int"))]);
    let errors = resolve(&m, Access::Public, &scope(&p, &no_modules, &[])).unwrap_err();
    assert_eq!(errors[0].code, "ELM_RESOLVE_NOT_FOUND");
    assert!(errors[0].message.contains("prelude: none"));
}

#[test]
fn qualified_alias_import_resolves_to_a_package_module_and_records_dependency() {
    let p = prelude::builtin("elm-core").unwrap();
    let other = |name: &[String]| {
        (name == ["Other"]).then(|| Interface {
            name: vec!["Other".into()],
            types: vec![InterfaceType {
                name: "Thing".into(),
                params: vec![],
                constructors: None,
            }],
        })
    };
    let import = Import {
        module: vec!["Other".into()],
        alias: Some("O".into()),
        exposing: None,
        span: sp(),
    };
    let m = module(
        &["A"],
        Exposing::All,
        vec![import],
        vec![alias("T", q(&["O"], "Thing"))],
    );
    let resolved = resolve(&m, Access::Public, &scope(&p, &other, &[])).unwrap();
    let ResolvedBody::Alias(RType::Ref(fq, _)) = &resolved.types[0].body else {
        panic!()
    };
    assert_eq!(fq.package, vec!["local", "example"]);
    assert_eq!(fq.module, vec!["Other"]);
    assert_eq!(resolved.depends_on, vec![vec!["Other".to_string()]]);
}

#[test]
fn ambiguous_unqualified_name_is_an_error() {
    let p = prelude::builtin("none").unwrap();
    let both = |name: &[String]| {
        (name == ["X"] || name == ["Y"]).then(|| Interface {
            name: name.to_vec(),
            types: vec![InterfaceType {
                name: "T".into(),
                params: vec![],
                constructors: None,
            }],
        })
    };
    let imp = |m: &str| Import {
        module: vec![m.into()],
        alias: None,
        exposing: Some(Exposing::All),
        span: sp(),
    };
    let m = module(
        &["A"],
        Exposing::All,
        vec![imp("X"), imp("Y")],
        vec![alias("U", r("T"))],
    );
    let errors = resolve(&m, Access::Public, &scope(&p, &both, &[])).unwrap_err();
    assert_eq!(errors[0].code, "ELM_RESOLVE_AMBIGUOUS");
}

#[test]
fn dependency_package_overrides_prelude_package() {
    let p = prelude::builtin("elm-core").unwrap();
    let deps = [DependencyInterface {
        package: vec!["Morphir".into(), "SDK".into()],
        modules: vec![Interface {
            name: vec!["Basics".into()],
            types: vec![InterfaceType {
                name: "Int".into(),
                params: vec![],
                constructors: None,
            }],
        }],
    }];
    let m = module(&["A"], Exposing::All, vec![], vec![alias("F", r("Float"))]);
    let errors = resolve(&m, Access::Public, &scope(&p, &no_modules, &deps)).unwrap_err();
    // the dependency's Basics has no Float; the prelude copy is shadowed
    assert_eq!(errors[0].code, "ELM_RESOLVE_NOT_FOUND");
}

#[test]
fn exposing_controls_type_and_constructor_access_and_interface_digest_ignores_private_and_docs() {
    let p = prelude::builtin("elm-core").unwrap();
    let custom = TypeDecl::Custom {
        name: "S".into(),
        params: vec![],
        constructors: vec![Constructor {
            name: "A".into(),
            args: vec![],
            span: sp(),
        }],
        doc: Some("d".into()),
        span: sp(),
    };
    let m1 = module(
        &["A"],
        Exposing::Explicit(vec![Exposed::Type {
            name: "S".into(),
            constructors: false,
        }]),
        vec![],
        vec![custom.clone(), alias("Hidden", r("Int"))],
    );
    let r1 = resolve(&m1, Access::Public, &scope(&p, &no_modules, &[])).unwrap();
    assert_eq!(r1.types[0].access, Access::Public);
    assert!(matches!(
        r1.types[0].body,
        ResolvedBody::Custom {
            constructor_access: Access::Private,
            ..
        }
    ));
    assert_eq!(r1.types[1].access, Access::Private);
    let mut m2 = m1.clone();
    m2.types[1] = alias("Hidden", r("Float"));
    if let TypeDecl::Custom { doc, .. } = &mut m2.types[0] {
        *doc = Some("changed".into());
    }
    let r2 = resolve(&m2, Access::Public, &scope(&p, &no_modules, &[])).unwrap();
    assert_eq!(r1.interface_digest(), r2.interface_digest());
    let m3 = module(
        &["A"],
        Exposing::Explicit(vec![Exposed::Type {
            name: "S".into(),
            constructors: true,
        }]),
        vec![],
        vec![custom],
    );
    let r3 = resolve(&m3, Access::Public, &scope(&p, &no_modules, &[])).unwrap();
    assert_ne!(r1.interface_digest(), r3.interface_digest());
}

#[test]
fn dependency_order_detects_cycles() {
    let a = (vec!["A".to_string()], vec![vec!["B".to_string()]]);
    let b = (vec!["B".to_string()], vec![]);
    assert_eq!(
        dependency_order(&[a.clone(), b.clone()]).unwrap(),
        vec![1, 0]
    );
    let b_cyc = (vec!["B".to_string()], vec![vec!["A".to_string()]]);
    let cycle = dependency_order(&[a, b_cyc]).unwrap_err();
    assert_eq!(cycle.len(), 2);
}
