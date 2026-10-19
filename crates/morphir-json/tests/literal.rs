use morphir_json::{ReadBudget, ValueKind, read};
use std::convert::Infallible;
struct Unlimited;
impl ReadBudget for Unlimited {
    type Error = Infallible;
    fn before_value(&mut self, _: usize, _: ValueKind) -> Result<(), Self::Error> {
        Ok(())
    }
    fn before_string(&mut self, _: &str) -> Result<(), Self::Error> {
        Ok(())
    }
}
#[test]
fn private_raw_value_key_remains_data() {
    let value = read(
        r#"{"extra":{"$serde_json::private::RawValue":"{\"x\":1,\"x\":2}"}}"#,
        &mut Unlimited,
    )
    .unwrap();
    assert!(value["extra"].is_object());
    assert_eq!(
        value["extra"]["$serde_json::private::RawValue"].as_str(),
        Some(r#"{"x":1,"x":2}"#)
    );
}
#[test]
fn container_grammar_matches_serde_without_private_key_interpretation() {
    let valid = [
        "null",
        "true",
        "false",
        "-0",
        "[0,1e0,{},[]]",
        r#"{"x":["comma, [] {}", "quote\" slash\\", "\uD83D\uDE00"],"x":1}"#,
        " \r\n { \"a\" : null } \t",
    ];
    for source in valid {
        let actual = read(source, &mut Unlimited).unwrap();
        let expected: serde_json::Value = serde_json::from_str(source).unwrap();
        assert_eq!(actual, expected, "{source}");
        assert_eq!(actual.to_string(), expected.to_string(), "{source}");
    }
    let invalid = [
        "",
        "[",
        "{",
        "[1,]",
        "{\"x\":1,}",
        "{x:1}",
        "[1true]",
        "true false",
        "01",
        "+1",
        ".1",
        "1.",
        "NaN",
        "[1 2]",
        "{\"x\" 1}",
        "{\"x\":1 \"y\":2}",
        "\"\\uD800\"",
        "\"\\uDC00\"",
        "\"\\x41\"",
        "\"a\nb\"",
        "\"unterminated",
        "\u{feff}{}",
        "[null]x",
    ];
    for source in invalid {
        assert!(
            serde_json::from_str::<serde_json::Value>(source).is_err(),
            "invalid test source {source}"
        );
        assert!(read(source, &mut Unlimited).is_err(), "{source}");
    }
}
#[test]
fn value_credit_precedes_scalar_decoding_and_owned_string_callbacks() {
    struct Refuse;
    impl ReadBudget for Refuse {
        type Error = &'static str;
        fn before_value(&mut self, _: usize, _: ValueKind) -> Result<(), Self::Error> {
            Err("value")
        }
        fn before_string(&mut self, _: &str) -> Result<(), Self::Error> {
            panic!("forbidden owned copy reached")
        }
    }
    assert!(matches!(
        read(r#""\uD800""#, &mut Refuse),
        Err(morphir_json::ReadError::Budget("value"))
    ));
}
