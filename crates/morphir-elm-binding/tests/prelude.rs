use morphir_elm_binding::prelude::*;

#[test]
fn elm_core_prelude_loads_and_maps_basics() {
    let p = builtin("elm-core").unwrap();
    assert_eq!(p.id, "elm-core");
    assert_eq!(
        p.alias_for(&["Basics".into()]),
        Some(vec!["Morphir".into(), "SDK".into(), "Basics".into()])
    );
    assert_eq!(
        p.unalias(&["Morphir".into(), "SDK".into(), "Basics".into()]),
        Some(vec!["Basics".into()])
    );
    let (pkg, module) = p
        .platform_module(&["Morphir".into(), "SDK".into(), "Basics".into()])
        .unwrap();
    assert_eq!(pkg.name, "Morphir.SDK");
    assert!(module.types.iter().any(|t| t.name == "Int"));
    let order = module.types.iter().find(|t| t.name == "Order").unwrap();
    assert_eq!(order.constructors, vec!["LT", "EQ", "GT"]);
    assert!(
        p.platform_module(&["Morphir".into(), "SDK".into(), "Decimal".into()])
            .is_some()
    );
    assert!(
        p.implicit_import
            .iter()
            .any(|i| i.module == "Basics" && i.exposing.contains(&"Int".to_string()))
    );
    assert!(
        p.implicit_import
            .iter()
            .any(|i| i.module == "Maybe" && i.exposing == vec!["Maybe(..)"])
    );
}

#[test]
fn none_prelude_is_empty() {
    let p = builtin("none").unwrap();
    assert!(p.implicit_import.is_empty() && p.module_alias.is_empty() && p.package.is_empty());
}

#[test]
fn from_option_accepts_names_and_inline_objects() {
    assert_eq!(from_option(None).unwrap().id, "elm-core");
    assert_eq!(
        from_option(Some(&serde_json::json!("none"))).unwrap().id,
        "none"
    );
    let inline = serde_json::json!({"id": "alt", "module_alias": [{"source": "Core", "target": "Acme.Std.Core"}], "package": [{"name": "Acme.Std", "module": [{"name": "Core", "types": [{"name": "Int"}]}]}]});
    let p = from_option(Some(&inline)).unwrap();
    assert_eq!(
        p.alias_for(&["Core".into()]),
        Some(vec!["Acme".into(), "Std".into(), "Core".into()])
    );
    assert!(from_option(Some(&serde_json::json!("nope"))).is_err());
    assert!(from_option(Some(&serde_json::json!(42))).is_err());
}

#[test]
fn digest_changes_with_content() {
    let a = builtin("elm-core").unwrap();
    let b = builtin("none").unwrap();
    assert!(a.digest().starts_with("sha256:"));
    assert_ne!(a.digest(), b.digest());
    assert_eq!(a.digest(), builtin("elm-core").unwrap().digest());
}

/// Pins elm-core.toml to morphir-elm's IncrementalResolve.elm. Skipped when the submodule is absent.
#[test]
fn elm_core_matches_morphir_elm_resolver() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../morphir-elm/src/Morphir/Elm/IncrementalResolve.elm");
    let Ok(src) = std::fs::read_to_string(&path) else {
        eprintln!("skipping: {} not present", path.display());
        return;
    };
    let p = builtin("elm-core").unwrap();
    // every morphirSdkPrefix "X" in sdkModuleMapping is an alias X -> Morphir.SDK.X
    for cap in regex_lite::Regex::new(r#"morphirSdkPrefix "([A-Za-z]+)""#)
        .unwrap()
        .captures_iter(&src)
    {
        let m = cap[1].to_string();
        assert_eq!(
            p.alias_for(std::slice::from_ref(&m)),
            Some(vec!["Morphir".into(), "SDK".into(), m.clone()]),
            "alias missing for {m}"
        );
    }
    // every Import (en [ "X" ]) in defaultImports is an implicit import
    for cap in regex_lite::Regex::new(r#"Import \(en \[ "([A-Za-z]+)" \]\)"#)
        .unwrap()
        .captures_iter(&src)
    {
        assert!(
            p.implicit_import.iter().any(|i| i.module == cap[1]),
            "implicit import missing for {}",
            &cap[1]
        );
    }
}
