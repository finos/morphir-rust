use morphir_extension_sdk::{
    CompileOptions, CompilePackage, CompileRequest, CompileResult, Frontend, SourceDocument,
};
use morphir_rust_binding::RustExtension;

fn compile(source: &str, version: &str, types_only: bool) -> CompileResult {
    RustExtension
        .compile(CompileRequest {
            language_id: "rust".into(),
            documents: vec![SourceDocument {
                uri: "file:///src/models.rs".into(),
                language_id: "rust".into(),
                text: source.into(),
                ..Default::default()
            }],
            package: CompilePackage {
                name: "acme/example".into(),
                exposed_modules: Some(vec!["Models".into()]),
            },
            options: CompileOptions {
                types_only,
                ir_version: version.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap()
}

#[test]
fn matches_compile_in_both_ir_versions() {
    for version in ["3", "4"] {
        for source in [
            "fn f(x: bool) -> i64 { match x { true => 1, false => 2 } }",
            "enum E<Box> { Full(Box), Empty } fn f(x: E<i64>) -> i64 { match x { E::Full(n) => n, E::Empty => 0 } }",
            "fn f(x: i64) -> bool { match x { -9223372036854775808 => true, 42i64 => false, _ => true } }",
            "fn f(x: char) -> bool { match x { 'a' => true, _ => false } }",
            "fn f(x: ()) -> i64 { match x { () => 1 } }",
            "fn f(x: (bool, i64)) -> i64 { match x { (true, n) => n, (false, n) => n } }",
            "enum Choice<T> { Empty, Full(T) } fn f(x: Choice<i64>) -> i64 { match x { Choice::Empty => 0, Choice::Full(n) => n } }",
            "enum Choice<T> { Full { left: T, right: bool }, Empty } fn f(x: Choice<i64>) -> i64 { match x { Choice::Full { right: true, left } => left, Choice::Full { left, .. } => left, Choice::Empty => 0 } }",
            "fn f<T>(x: Option<T>, fallback: T) -> T { match x { Some(n) => n, None => fallback } }",
            "fn f(x: Result<i64, bool>) -> i64 { match x { Result::Ok(n) => n, Result::Err(true) => 1, Result::Err(false) => 0 } }",
            "fn f(x: Option<(bool, i64)>) -> i64 { match x { Option::Some((true, n)) => n, Option::Some((false, n)) => n, Option::None => 0 } }",
            "fn f<T>(x: bool, value: T) -> T { match x { true => value, false => value } }",
            "fn f(x: i64) -> i64 { let morphir_local_0 = x; match x { x => { let x = x; x } } }",
        ] {
            let result = compile(source, version, false);
            assert!(
                result.success,
                "{version} {source}: {:?}",
                result.diagnostics
            );
            assert!(result.ir.unwrap().to_string().contains("PatternMatch"));
        }
    }
}

#[test]
fn rejects_invalid_or_unsupported_patterns_without_ir() {
    for source in [
        "fn f(x: bool) -> i64 { match x { true => 1 } }",
        "fn f(x: Option<i64>) -> i64 { match x { Some => 1 } }",
        "fn f(x: Result<i64, bool>) -> i64 { match x { Ok => 1 } }",
        "fn f(x: Option<i64>) -> i64 { match x { mut None => 1 } }",
        "fn f(x: (bool, bool)) -> i64 { match x { (true, _) => 1, (_, true) => 0 } }",
        "fn f(x: Option<bool>) -> i64 { match x { Some(true) => 1, None => 0 } }",
        "fn f(x: i64) -> i64 { match x { &a => a } }",
        "fn f(x: Vec<i64>) -> i64 { match x { [a] => a, _ => 0 } }",
        "fn f(x: bool) -> i64 { match x { #[cfg(any())] _ => 1 } }",
        "fn f(x: bool) -> i64 { #[cfg(any())] match x { _ => 1 } }",
        "struct Option; fn f(x: Option) -> i64 { match x { Option::None => 1 } }",
        "struct None; fn f(x: Option<i64>) -> i64 { match x { Some(n) => n, None => 0 } }",
        "fn f(x: i64) -> i64 { match x { 1 => 1 } }",
        "fn f(x: bool) -> i64 { match x { true => 1, false => false } }",
        "fn f(x: bool) -> i64 { match x { 1 => 1, _ => 0 } }",
        "fn f(x: (i64, i64)) -> i64 { match x { (a, a) => a } }",
        "fn f(x: bool) -> i64 { let n = match x { a => 0 }; a }",
        "fn f(x: bool) -> i64 { match x { a if a => 1, _ => 0 } }",
        "fn f(x: bool) -> i64 { match x { true | false => 1 } }",
        "fn f(x: i64) -> i64 { match x { 1..=3 => 1, _ => 0 } }",
        "fn f(x: i64) -> i64 { match x { a @ _ => a } }",
        "fn f(x: i64) -> i64 { match x { ref a => 0 } }",
        "fn f(x: i64) -> i64 { match x { mut a => a } }",
        "fn f(x: f64) -> i64 { match x { 1.0 => 1, _ => 0 } }",
        "fn f(x: String) -> i64 { match x { \"hi\" => 1, _ => 0 } }",
        "fn f(x: Option<i64>) -> i64 { match x { Some(a, b) => a, None => 0 } }",
        "fn f(x: Option<i64>) -> i64 { match x { Result::Ok(a) => a, _ => 0 } }",
        "enum E { A { x: i64 } } fn f(x: E) -> i64 { match x { E::A { bad } => bad } }",
        "enum E { A { x: i64, y: i64 } } fn f(x: E) -> i64 { match x { E::A { x } => x } }",
        "fn f<T>(x: Option<T>) -> (i64, Option<T>) { let n = match x { _ => 1 }; (n, x) }",
        "fn f<T>(x: bool, a: T) -> T { let n = match x { true => { let moved = a; 1 }, false => 0 }; a }",
    ] {
        for version in ["3", "4"] {
            let result = compile(source, version, false);
            assert!(!result.success, "accepted {source}");
            assert!(result.ir.is_none());
            assert!(result.diagnostics.iter().any(|d| d.location.is_some()));
        }
    }
}

#[test]
fn bare_subject_enum_variants_require_qualification() {
    for version in ["3", "4"] {
        for source in [
            "enum E { A, B } fn f(x: E) -> i64 { match x { A => 1 } }",
            "enum E { A(i64), B } fn f(x: E) -> i64 { match x { A => 1 } }",
            "enum E { A, B } fn f(x: Option<E>) -> i64 { match x { Some(A) => 1, None => 0 } }",
        ] {
            let result = compile(source, version, false);
            assert!(!result.success, "accepted {source}");
            assert!(result.ir.is_none());
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains("qualification"))
            );
        }
        for source in [
            "enum E { A, B } fn f(x: E) -> E { match x { other => other } }",
            "enum E { A, B } fn f(x: i64) -> i64 { match x { A => A } }",
            "enum E { A, B } enum Other { C, D } fn f(x: Other) -> Other { match x { A => A } }",
        ] {
            let result = compile(source, version, false);
            assert!(result.success, "{source}: {:?}", result.diagnostics);
        }
    }
}

#[test]
fn executable_aliases_cannot_hide_box_storage() {
    for version in ["3", "4"] {
        for source in [
            "type Payload = Box<i64>; enum E { A(Payload) } fn f(x: E) -> Payload { match x { E::A(n) => n } }",
            "type Payload = Box<i64>; enum E { A(Payload) } fn f(x: E) -> i64 { match x { E::A(_) => 0 } }",
            "type Inner<T> = Box<T>; type Outer = Inner<i64>; fn f(x: Outer) -> Outer { x }",
            "type Payload = Box<i64>; fn f(x: Option<Payload>) -> Option<Payload> { x }",
            "type Payload = Box<i64>; fn f() -> i64 { let x: Payload = 1; 0 }",
        ] {
            let result = compile(source, version, false);
            assert!(!result.success, "accepted {source}");
            assert!(result.ir.is_none());
            assert!(
                result.diagnostics.iter().any(|d| d.message.contains("Box")),
                "{:?}",
                result.diagnostics
            );
            assert!(
                compile(source, version, true).success,
                "type-only rejected {source}"
            );
        }
        for source in [
            "type Payload = i64; enum E { A(Payload) } fn f(x: E) -> Payload { match x { E::A(n) => n } }",
            "type Payload = Box<i64>; enum E<Payload> { A(Payload) } fn f(x: E<i64>) -> i64 { match x { E::A(n) => n } }",
            "type Box<T> = T; enum E { A(Box<i64>) } fn f(x: E) -> Box<i64> { match x { E::A(n) => n } }",
        ] {
            let result = compile(source, version, false);
            assert!(result.success, "{source}: {:?}", result.diagnostics);
        }
    }
}

#[test]
fn subject_alias_chains_do_not_hide_enum_variant_names() {
    for version in ["3", "4"] {
        for source in [
            "enum E { A, B } type Alias = E; fn f(x: Alias) -> i64 { match x { A => 1 } }",
            "enum E { A, B } type Identity<T> = T; type Alias<U> = Identity<U>; fn f(x: Alias<E>) -> i64 { match x { A => 1 } }",
            "enum E<T> { A(T), B } type Inner<U> = E<U>; type Outer<V> = Inner<V>; fn f(x: Outer<i64>) -> i64 { match x { A => 1 } }",
            "enum E { A, B } type Alias = E; fn f(x: Option<Alias>) -> i64 { match x { Some(A) => 1, None => 0 } }",
        ] {
            let result = compile(source, version, false);
            assert!(!result.success, "accepted {source}");
            assert!(result.ir.is_none());
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains("qualification")),
                "{:?}",
                result.diagnostics
            );
        }
        for source in [
            "enum E { A, B } type Alias = E; fn f(x: Alias) -> Alias { match x { other => other } }",
            "enum E { A, B } type Identity<T> = T; fn f(x: Identity<i64>) -> Identity<i64> { match x { A => A } }",
            "enum E { A, B } type Identity<T> = T; fn f<E>(x: Identity<E>) -> Identity<E> { match x { A => A } }",
            "enum E { A, B } type Identity<T> = T; fn f(x: Identity<Identity<i64>>) -> Identity<Identity<i64>> { match x { A => A } }",
        ] {
            let result = compile(source, version, false);
            assert!(result.success, "{source}: {:?}", result.diagnostics);
        }
    }
}
