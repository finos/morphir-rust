//! Morphir IR v4 naming: canonical encodings, the filesystem escape, and legacy
//! interop. See kb decisions 0001 and 0002 in finos/morphir.

use morphir_core::naming::{FQName, Name, NameStyle, PackageName, Segment};

fn value_in_usd() -> Name {
    Name::from_segments(vec![
        Segment::Word("value".into()),
        Segment::Word("in".into()),
        Segment::Initialism("usd".into()),
    ])
    .expect("valid segments")
}

#[test]
fn from_segments_rejects_segments_that_break_the_invariant() {
    // Segment's variants are publicly constructible, so Name has to check.
    assert!(Name::from_segments(vec![Segment::Word("Bad".into())]).is_err());
    assert!(Name::from_segments(vec![Segment::Word("USD".into())]).is_err());
    assert!(Name::from_segments(vec![Segment::Word("has space".into())]).is_err());
    assert!(Name::from_segments(vec![Segment::Word(String::new())]).is_err());
    // A digits-only initialism cannot round-trip: uppercasing digits is a no-op,
    // so the decoder would classify it as a word.
    assert!(Name::from_segments(vec![Segment::Initialism("12".into())]).is_err());
    // Digits are fine in a word, and in an initialism that carries a letter.
    assert!(Name::from_segments(vec![Segment::Word("2052".into())]).is_ok());
    assert!(Name::from_segments(vec![Segment::Initialism("fr2052a".into())]).is_ok());
}

#[test]
fn legacy_digit_runs_stay_words_and_round_trip() {
    // A run of single digits must not collapse into an initialism. Before this
    // was fixed, ["1","2"] became Initialism("12"), encoded as "12", and decoded
    // back as Word("12"), silently changing identity.
    let name = Name::new(&["1", "2"]);
    assert_eq!(
        name.segments(),
        &[Segment::Word("1".into()), Segment::Word("2".into())]
    );
    assert_eq!(name.to_canonical_string(), "1-2");
    assert_eq!(
        Name::from_canonical_string(&name.to_canonical_string()).unwrap(),
        name
    );

    // A digit breaks a letter run rather than joining it.
    assert_eq!(Name::new(&["u", "1"]).to_canonical_string(), "u-1");
    assert_eq!(
        Name::new(&["v", "2", "api"]).to_canonical_string(),
        "v-2-api"
    );
}

#[test]
fn parsers_reject_a_digits_only_initialism() {
    assert!(Name::from_canonical_string("--12").is_err());
    assert!(Name::from_canonical_string("value--12").is_err());
    assert!(Name::from_file_stem("_12").is_err());
}

#[test]
fn initialism_serializes_as_an_uppercase_segment() {
    let name = value_in_usd();
    let json = serde_json::to_string(&name).unwrap();

    assert_eq!(json, r#""value-in-USD""#);
    assert_eq!(serde_json::from_str::<Name>(&json).unwrap(), name);
}

#[test]
fn a_word_is_distinct_from_an_initialism_with_the_same_letters() {
    let word = Name::from_canonical_string("in-usd").unwrap();
    let initialism = Name::from_canonical_string("in-USD").unwrap();

    assert_ne!(word, initialism);
    assert_eq!(word.to_pascal_case(), "InUsd");
    assert_eq!(initialism.to_pascal_case(), "InUSD");
    // Rendering into a case-free convention is not injective; backends must
    // detect the collision.
    assert_eq!(word.to_snake_case(), initialism.to_snake_case());
}

#[test]
fn decoder_accepts_both_encodings_and_encoder_writes_only_one() {
    let name = value_in_usd();

    assert_eq!(
        name.to_canonical_string_in(NameStyle::Uppercase),
        "value-in-USD"
    );
    assert_eq!(
        name.to_canonical_string_in(NameStyle::DoubledHyphen),
        "value-in--usd"
    );

    // Both decode to the same name, whichever style is shipped.
    assert_eq!(Name::from_canonical_string("value-in-USD").unwrap(), name);
    assert_eq!(Name::from_canonical_string("value-in--usd").unwrap(), name);
}

#[test]
fn the_two_encodings_are_disjoint() {
    // A name carrying an initialism encodes differently under each style, and the
    // union decoder recovers the same name from either. That is what makes
    // flipping CANONICAL_STYLE backward compatible for readers.
    for canonical in [
        "value-in-USD",
        "get-HTML",
        "IO-error",
        "my-API-client",
        "CON",
    ] {
        let name = Name::from_canonical_string(canonical).unwrap();
        let uppercase = name.to_canonical_string_in(NameStyle::Uppercase);
        let doubled = name.to_canonical_string_in(NameStyle::DoubledHyphen);

        assert_ne!(uppercase, doubled, "{canonical} should differ per style");
        assert_eq!(Name::from_canonical_string(&uppercase).unwrap(), name);
        assert_eq!(Name::from_canonical_string(&doubled).unwrap(), name);
    }

    // A name with no initialism is legal under both and encodes identically.
    let plain = Name::from_canonical_string("user-account").unwrap();
    assert_eq!(
        plain.to_canonical_string_in(NameStyle::Uppercase),
        plain.to_canonical_string_in(NameStyle::DoubledHyphen)
    );

    // A string carrying both markers is ambiguous and rejected.
    assert!(Name::from_canonical_string("value--in-USD").is_err());
}

#[test]
fn canonical_parser_rejects_the_retired_parenthesis_encoding() {
    assert!(Name::from_canonical_string("value-in-(usd)").is_err());
    assert!(Name::from_canonical_string("value-(usd").is_err());
    assert!(Name::from_canonical_string("value-usd)").is_err());
}

#[test]
fn canonical_parser_rejects_mixed_case_segments() {
    assert!(Name::from_canonical_string("Usd").is_err());
    assert!(Name::from_canonical_string("MyName").is_err());
    assert!(Name::from_canonical_string("value-Usd").is_err());
}

#[test]
fn canonical_parser_rejects_malformed_separators() {
    assert!(Name::from_canonical_string("-user").is_err());
    assert!(Name::from_canonical_string("user-").is_err());
    assert!(Name::from_canonical_string("user---id").is_err());
    assert!(Name::from_canonical_string("--").is_err());
    assert!(Name::from_canonical_string("user_id").is_err());
}

#[test]
fn a_single_letter_word_is_a_type_variable_not_an_initialism() {
    let name = Name::new(&["a"]);
    assert_eq!(name.segments(), &[Segment::Word("a".into())]);
    assert_eq!(name.to_canonical_string(), "a");
    assert_eq!(name.to_file_stem(), "a");
}

#[test]
fn a_digits_only_segment_is_a_word() {
    let name = Name::from_canonical_string("2052").unwrap();
    assert_eq!(name.segments(), &[Segment::Word("2052".into())]);
}

#[test]
fn legacy_words_collapse_runs_of_two_or_more_single_letters() {
    assert_eq!(Name::new(&["value", "in", "u", "s", "d"]), value_in_usd());
    assert_eq!(
        Name::new(&["get", "h", "t", "m", "l"]).to_canonical_string(),
        "get-HTML"
    );
    // A run of one stays a word.
    assert_eq!(Name::new(&["max", "n"]).to_canonical_string(), "max-n");
}

#[test]
fn legacy_word_list_round_trips() {
    let words = vec!["value", "in", "u", "s", "d"];
    let name = Name::new(&words);
    assert_eq!(name.words(), words);
}

#[test]
fn file_stem_escapes_initialisms_and_reserved_device_names() {
    assert_eq!(value_in_usd().to_file_stem(), "value-in-_usd");
    assert_eq!(
        Name::from_canonical_string("user-ID")
            .unwrap()
            .to_file_stem(),
        "user-_id"
    );
    assert_eq!(
        Name::from_canonical_string("aux").unwrap().to_file_stem(),
        "aux_"
    );
    assert_eq!(
        Name::from_canonical_string("CON").unwrap().to_file_stem(),
        "_con"
    );
    assert_eq!(
        Name::from_canonical_string("com1").unwrap().to_file_stem(),
        "com1_"
    );
    // Reserved only when it is the whole stem.
    assert_eq!(
        Name::from_canonical_string("nul-pointer")
            .unwrap()
            .to_file_stem(),
        "nul-pointer"
    );
}

#[test]
fn file_stem_is_lowercase_so_it_is_stable_on_case_insensitive_filesystems() {
    let word = Name::from_canonical_string("in-usd").unwrap();
    let initialism = Name::from_canonical_string("in-USD").unwrap();

    assert_eq!(word.to_file_stem(), "in-usd");
    assert_eq!(initialism.to_file_stem(), "in-_usd");
    assert_ne!(
        word.to_file_stem().to_lowercase(),
        initialism.to_file_stem().to_lowercase()
    );
}

#[test]
fn file_stem_round_trips() {
    for canonical in [
        "value-in-USD",
        "user-ID",
        "aux",
        "CON",
        "com1",
        "nul-pointer",
        "a",
    ] {
        let name = Name::from_canonical_string(canonical).unwrap();
        assert_eq!(
            Name::from_file_stem(&name.to_file_stem()).unwrap(),
            name,
            "round trip failed for {canonical}"
        );
    }
}

#[test]
fn permissive_parsing_recognizes_camel_case_initialism_runs() {
    assert_eq!(Name::from("valueInUSD"), value_in_usd());
    assert_eq!(Name::from("ValueInUSD"), value_in_usd());
    assert_eq!(
        Name::from("value_in_usd").to_canonical_string(),
        "value-in-usd"
    );
    assert_eq!(
        Name::from("parseHTMLDocument").to_canonical_string(),
        "parse-HTML-document"
    );
    assert_eq!(
        Name::from("testModule").to_canonical_string(),
        "test-module"
    );
}

#[test]
fn rendering_applies_the_target_convention_to_initialisms() {
    let name = Name::from_canonical_string("my-API-client").unwrap();

    assert_eq!(name.to_camel_case(), "myAPIClient");
    assert_eq!(name.to_pascal_case(), "MyAPIClient");
    assert_eq!(name.to_pascal_case_pascal_initialism(), "MyApiClient");
    assert_eq!(name.to_snake_case(), "my_api_client");
    assert_eq!(name.to_kebab_case(), "my-api-client");
    assert_eq!(name.to_screaming_snake_case(), "MY_API_CLIENT");
}

#[test]
fn a_leading_initialism_lowercases_whole_in_camel_case() {
    let name = Name::from_canonical_string("IO-error").unwrap();
    assert_eq!(name.to_camel_case(), "ioError");
    assert_eq!(name.to_pascal_case(), "IOError");
    assert_eq!(name.to_pascal_case_pascal_initialism(), "IoError");
}

#[test]
fn fqname_uses_v4_hash_separator() {
    let fq = FQName::from_canonical_string("morphir/SDK:list#map").unwrap();

    assert_eq!(
        serde_json::to_string(&fq).unwrap(),
        r#""morphir/SDK:list#map""#
    );
    assert_eq!(
        serde_json::from_str::<FQName>(r#""morphir/SDK:list#map""#).unwrap(),
        fq
    );
}

#[test]
fn package_name_accepts_classic_arrays_and_writes_canonical_strings() {
    let package: PackageName = serde_json::from_str(r#"[["morphir"],["s","d","k"]]"#).unwrap();

    assert_eq!(serde_json::to_string(&package).unwrap(), r#""morphir/SDK""#);
}
