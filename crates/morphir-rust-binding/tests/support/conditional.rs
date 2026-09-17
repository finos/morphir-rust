//! A shared conditional model and its external Rust consumer.
pub const SOURCE: &str = r#"
pub type Foo = i64;
pub fn foo(value: Foo) -> Foo { value }
pub fn select(a: i64, b: i64) -> i64 {
    let larger = if a > b { a } else { b };
    if larger > 10 && !(a == b) { larger }
    else if a == b { 0 }
    else { -1 }
}
pub fn decision(enabled: bool, amount: i64) -> (i64, bool) {
    let accepted = enabled || amount >= 100;
    (if accepted { amount } else { 0 }, accepted)
}
pub fn bounds(value: i64) -> bool {
    value >= -10 && value <= 10 && value != 0
}
pub fn literals() -> (i64, f64, char) { (-9223372036854775808, -2f64, 'λ') }
pub fn shadow(x: i64) -> i64 {
    let x = if x < 0 { 0 } else { x };
    let y = { let x = 7; x };
    if x > y { x } else { y }
}
"#;

pub fn assert_executable(source: &str) {
    let consumer = r#"
fn main() {
    assert_eq!(models::foo(42), 42);
    assert_eq!(models::select(11, 2), 11);
    assert_eq!(models::select(2, 12), 12);
    assert_eq!(models::select(12, 12), 0);
    assert_eq!(models::select(3, 2), -1);
    assert_eq!(models::select(-3, -2), -1);
    assert_eq!(models::decision(true, 1), (1, true));
    assert_eq!(models::decision(false, 100), (100, true));
    assert_eq!(models::decision(false, 99), (0, false));
    assert!(models::bounds(-10));
    assert!(models::bounds(10));
    assert!(!models::bounds(0));
    assert!(!models::bounds(11));
    assert_eq!(models::literals(), (i64::MIN, -2.0, 'λ'));
    assert_eq!(models::shadow(-1), 7);
    assert_eq!(models::shadow(10), 10);
}
"#;
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("main.rs");
    let executable = directory
        .path()
        .join(format!("consumer{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&input, format!("{source}\n{consumer}")).unwrap();
    let compiled = std::process::Command::new("rustc")
        .args(["--edition=2024", "--crate-name=conditional_consumer"])
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
