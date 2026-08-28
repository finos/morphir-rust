use morphir_core::naming::{FQName, Name, PackageName};

#[test]
fn acronym_words_round_trip_without_collapsing_structure() {
    let name = Name {
        words: vec![
            "value".into(),
            "in".into(),
            "u".into(),
            "s".into(),
            "d".into(),
        ],
    };

    let json = serde_json::to_string(&name).unwrap();

    assert_eq!(json, r#""value-in-(usd)""#);
    assert_eq!(serde_json::from_str::<Name>(&json).unwrap(), name);
}

#[test]
fn fqname_uses_v4_hash_separator() {
    let fq = FQName::from_canonical_string("morphir/(sdk):list#map").unwrap();

    assert_eq!(
        serde_json::to_string(&fq).unwrap(),
        r#""morphir/(sdk):list#map""#
    );
    assert_eq!(
        serde_json::from_str::<FQName>(r#""morphir/(sdk):list#map""#).unwrap(),
        fq
    );
}

#[test]
fn package_name_accepts_classic_arrays_and_writes_canonical_strings() {
    let package: PackageName = serde_json::from_str(r#"[["morphir"],["s","d","k"]]"#).unwrap();

    assert_eq!(
        serde_json::to_string(&package).unwrap(),
        r#""morphir/(sdk)""#
    );
}

#[test]
fn canonical_name_parser_rejects_unmatched_parentheses() {
    assert!(Name::from_canonical_string("value-(usd").is_err());
    assert!(Name::from_canonical_string("value-usd)").is_err());
}
