//! Shared pattern model and executable consumer for native and WASM tests.
pub const SOURCE: &str = r#"
pub enum Decision { Approved(i64), Rejected, Deferred(bool) }
pub fn amount(decision: Decision) -> i64 {
    match decision {
        Decision::Approved(value) => value,
        Decision::Rejected => 0,
        Decision::Deferred(true) => 1,
        Decision::Deferred(false) => -1,
    }
}
pub fn optional(value: Option<i64>) -> i64 {
    match value { Some(value) => value, None => -1 }
}
pub fn outcome(value: Result<i64, bool>) -> i64 {
    match value { Ok(value) => value, Err(true) => 1, Err(false) => -1 }
}
pub fn nested(value: Option<Result<i64, bool>>) -> i64 {
    match value {
        Some(Ok(value)) => value,
        Some(Err(true)) => 1,
        Some(Err(false)) => -1,
        None => 0,
    }
}
pub fn pair(value: (bool, i64)) -> i64 {
    match value { (true, value) => value, (false, 0) => 100, (_, value) => value }
}
pub fn letter(value: char) -> bool { match value { 'λ' => true, _ => false } }
pub fn unwrap<T>(value: Option<T>, fallback: T) -> T {
    match value { Some(value) => value, None => fallback }
}
pub fn conditional(value: Option<i64>, enabled: bool) -> i64 {
    match value { Some(value) => if enabled { value } else { 0 }, None => -1 }
}
"#;

pub fn assert_executable(source: &str) {
    let consumer = r#"
fn main() {
    assert_eq!(models::amount(models::Decision::Approved(42)), 42);
    assert_eq!(models::amount(models::Decision::Rejected), 0);
    assert_eq!(models::amount(models::Decision::Deferred(true)), 1);
    assert_eq!(models::amount(models::Decision::Deferred(false)), -1);
    assert_eq!(models::optional(Some(42)), 42);
    assert_eq!(models::optional(None), -1);
    assert_eq!(models::outcome(Ok(42)), 42);
    assert_eq!(models::outcome(Err(true)), 1);
    assert_eq!(models::outcome(Err(false)), -1);
    assert_eq!(models::nested(Some(Ok(42))), 42);
    assert_eq!(models::nested(Some(Err(true))), 1);
    assert_eq!(models::nested(Some(Err(false))), -1);
    assert_eq!(models::nested(None), 0);
    assert_eq!(models::pair((true, 0)), 0);
    assert_eq!(models::pair((false, 0)), 100);
    assert_eq!(models::pair((false, 42)), 42);
    assert!(models::letter('λ'));
    assert!(!models::letter('a'));
    assert_eq!(models::unwrap(Some(String::from("value")), String::from("fallback")), "value");
    assert_eq!(models::unwrap(None, String::from("fallback")), "fallback");
    assert_eq!(models::conditional(Some(42), true), 42);
    assert_eq!(models::conditional(Some(42), false), 0);
    assert_eq!(models::conditional(None, true), -1);
}
"#;
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("main.rs");
    let executable = directory
        .path()
        .join(format!("consumer{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&input, format!("{source}\n{consumer}")).unwrap();
    let compiled = std::process::Command::new("rustc")
        .args(["--edition=2024", "--crate-name=pattern_consumer"])
        .arg(&input)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{source}\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let executed = std::process::Command::new(executable).output().unwrap();
    assert!(
        executed.status.success(),
        "{}",
        String::from_utf8_lossy(&executed.stderr)
    );
}
