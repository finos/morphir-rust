use morphir_core::ir::classic::naming::Name;
use std::str::FromStr;

// Test cases ported from Morphir.IR.Name.elm matching fromString implementation

#[test]
fn test_name_from_str_basic() {
    // fromString "fooBar_baz 123" --> ["foo", "bar", "baz", "123"]
    let n = Name::from_str("fooBar_baz 123");
    assert_eq!(n.words, vec!["foo", "bar", "baz", "123"]);
}

#[test]
fn test_name_from_str_camel_case() {
    // fromString "valueInUSD" --> ["value", "in", "u", "s", "d"]
    let n = Name::from_str("valueInUSD");
    assert_eq!(n.words, vec!["value", "in", "u", "s", "d"]);
}

#[test]
fn test_name_from_str_title_case() {
    // fromString "ValueInUSD" --> ["value", "in", "u", "s", "d"]
    let n = Name::from_str("ValueInUSD");
    assert_eq!(n.words, vec!["value", "in", "u", "s", "d"]);
}

#[test]
fn test_name_from_str_snake_case() {
    // fromString "value_in_USD" --> ["value", "in", "u", "s", "d"]
    let n = Name::from_str("value_in_USD");
    assert_eq!(n.words, vec!["value", "in", "u", "s", "d"]);
}

#[test]
fn test_name_from_str_invalid_chars() {
    // fromString "_-%" --> []
    let n = Name::from_str("_-%");
    assert!(n.words.is_empty());
}

#[test]
fn test_name_from_str_consecutive_upper() {
    // "USD" -> ["u", "s", "d"]
    let n = Name::from_str("USD");
    assert_eq!(n.words, vec!["u", "s", "d"]);
}

#[test]
fn test_name_from_str_mixed() {
    // "myID" -> ["my", "i", "d"]
    let n = Name::from_str("myID");
    assert_eq!(n.words, vec!["my", "i", "d"]);
}
