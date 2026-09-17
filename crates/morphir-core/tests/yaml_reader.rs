use morphir_core::ir::yaml::read;
use serde_json::json;

fn code(text: &str) -> (String, String) {
    let d = read(text).expect_err(text);
    (
        serde_json::to_value(d.code)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned(),
        d.cursor,
    )
}
fn value(text: &str) -> serde_json::Value {
    read(text).unwrap_or_else(|d| panic!("{text}: {d:?}"))
}
fn lexeme(v: &serde_json::Value) -> String {
    v.as_number().unwrap().to_string()
}

#[test]
fn documents() {
    assert_eq!(code(""), ("invalid_yaml".into(), "".into()));
    assert_eq!(code("a: 1\n---\nb: 2\n").0, "invalid_yaml");
    assert_eq!(value("--- \na: 1\n"), json!({"a": 1}));
    assert_eq!(code("%YAML 1.2\n---\na: 1\n").0, "unsupported_yaml_feature");
    assert_eq!(
        code("%TAG ! tag:x,2026:\n---\na: 1\n").0,
        "unsupported_yaml_feature"
    );
    // A directive is only ever unindented: an indented `%` opens a plain scalar and is no
    // directive, so refusing the document would be refusing conforming YAML.
    // A directive is only ever unindented, so an indented `%` is not one. The line is still not
    // YAML — `%` is a reserved indicator wherever a plain scalar starts — but that is the
    // parser's `invalid_yaml`, not the profile refusing a directive that is not there.
    assert_eq!(code("  %x: 1\n").0, "invalid_yaml");
}

#[test]
fn features() {
    assert_eq!(
        code("a: &x 1\nb: *x\n"),
        ("unsupported_yaml_feature".into(), "/a".into())
    );
    assert_eq!(
        code("a: !!int 1\n"),
        ("unsupported_yaml_feature".into(), "/a".into())
    );
    assert_eq!(
        code("a: !foo 1\n"),
        ("unsupported_yaml_feature".into(), "/a".into())
    );
    assert_eq!(
        code("base: {x: 1}\nchild:\n  <<: {x: 2}\n"),
        ("unsupported_yaml_feature".into(), "/child/<<".into())
    );
    assert_eq!(
        code("a: 1\na: 2\n"),
        ("duplicate_member".into(), "/a".into())
    );
    assert_eq!(code("? [1, 2]\n: x\n"), ("invalid_type".into(), "".into()));
    // A plain key resolves like any other plain scalar, so a key that is not a string is
    // refused rather than read as its text (parse.ts `keyText`).
    assert_eq!(code("1: x\n"), ("invalid_type".into(), "".into()));
    assert_eq!(code("true: x\n"), ("invalid_type".into(), "".into()));
    assert_eq!(
        code("a:\n  null: x\n"),
        ("invalid_type".into(), "/a".into())
    );
    assert_eq!(
        code("a:\n  0xF: x\n"),
        ("invalid_literal".into(), "/a".into())
    );
    assert_eq!(value("'1': x\n"), json!({"1": "x"}));
    assert_eq!(code("a: [1, 2\n").0, "invalid_yaml");
    assert_eq!(code("a:\n\t- 1\n").0, "invalid_yaml");
}

#[test]
fn scalars() {
    let v = value(
        "t: true\nT: True\nF: FALSE\nn: null\nn2: ~\nn3:\ni: 42\nneg: -7\nplus: +7\nf: 1.5\ndot: .5\ntrail: 5.\nexp: 1e3\nbig: 12345678901234567890\nkeep: 1.50\ns: hello\ndate: 2026-01-15\nq: '42'\ndq: \"true\"\nb: |\n  x\n  y\n",
    );
    assert_eq!(v["t"], json!(true));
    assert_eq!(v["T"], json!(true));
    assert_eq!(v["F"], json!(false));
    assert_eq!(v["n"], json!(null));
    assert_eq!(v["n2"], json!(null));
    assert_eq!(v["n3"], json!(null));
    assert_eq!(lexeme(&v["i"]), "42");
    assert_eq!(lexeme(&v["neg"]), "-7");
    assert_eq!(lexeme(&v["plus"]), "7");
    assert_eq!(lexeme(&v["f"]), "1.5");
    assert_eq!(lexeme(&v["dot"]), "0.5");
    assert_eq!(lexeme(&v["trail"]), "5.0");
    // serde_json's arbitrary-precision scanner normalises an exponent to carry an explicit
    // sign, so the `1e3` source lexeme is stored — and rendered — as `1e+3`.
    assert_eq!(lexeme(&v["exp"]), "1e+3");
    assert_eq!(lexeme(&v["big"]), "12345678901234567890");
    assert_eq!(lexeme(&v["keep"]), "1.50");
    assert_eq!(v["s"], json!("hello"));
    assert_eq!(v["date"], json!("2026-01-15"));
    assert_eq!(v["q"], json!("42"));
    assert_eq!(v["dq"], json!("true"));
    assert_eq!(v["b"], json!("x\ny\n"));
}

#[test]
fn invalid_literals() {
    for text in [
        "a: 01\n",
        "a: -007\n",
        "a: 0o17\n",
        "a: 0xF\n",
        "a: 0x1F\n",
        "a: .inf\n",
        "a: -.inf\n",
        "a: .nan\n",
        "a: .NaN\n",
    ] {
        assert_eq!(
            code(text),
            ("invalid_literal".into(), "/a".into()),
            "{text}"
        );
    }
    // Only the reference's exact octal and hexadecimal spellings are refused; anything else is
    // an ordinary string (parse.ts `OCTAL_OR_HEX`, /^0[ox][0-9a-fA-F]+$/).
    for text in ["0xZZ", "0x", "0o", "0X1F", "-0xF"] {
        assert_eq!(
            value(&format!("a: {text}\n"))["a"],
            json!(text),
            "{text} should be a string"
        );
    }
    assert_eq!(value("a: 0\n")["a"], json!(0));
    // `-0` is accepted, but serde_json's arbitrary-precision scanner folds the sign away: the
    // stored lexeme is `0`, not the `-0` of the source.
    assert_eq!(value("a: -0\n")["a"], json!(-0));
    assert_eq!(lexeme(&value("a: -0\n")["a"]), "0");
}

#[test]
fn shapes_and_cursors() {
    assert_eq!(
        value("a: {}\nb: []\nc: [1, [2, {d: 3}]]\n"),
        json!({"a": {}, "b": [], "c": [1, [2, {"d": 3}]]})
    );
    assert_eq!(
        code("a:\n  - x: 1\n    x: 2\n"),
        ("duplicate_member".into(), "/a/0/x".into())
    );
    assert_eq!(code("a/b: 1\na/b: 2\n").1, "/a/b"); // raw member names in pointers
    let deep = "[".repeat(1001) + &"]".repeat(1001);
    assert_eq!(code(&deep).0, "nesting_too_deep");
    let ok = "[".repeat(1000) + &"]".repeat(1000);
    assert!(read(&ok).is_ok());
}

#[test]
fn a_parser_error_keeps_its_description() {
    let d = read("a: [1, 2\n").expect_err("an unterminated flow sequence");
    assert_eq!(d.code, morphir_core::ir::DiagnosticCode::InvalidYaml);
    assert!(!d.message.is_empty(), "the parser's description is carried");
    // granit names the flow sequence by its bracket: "unclosed bracket '['".
    assert!(
        d.message.contains("unclosed bracket"),
        "unexpected message: {}",
        d.message
    );
    // The position rides in `line`/`column` rather than in the message text.
    assert!(!d.message.contains("column"), "{}", d.message);
    assert!(d.line.is_some() && d.column.is_some());
}

#[test]
fn member_order_is_kept() {
    let v = value("z: 1\na: 2\nm: 3\n");
    let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["z", "a", "m"]);
}
