//! Callable source model shared by native, BDD and WASM execution tests.
pub const SOURCE: &str = r#"
pub fn forward(value: i64) -> bool { positive(value) }
pub fn positive(value: i64) -> bool { value > 0 }
pub fn identity<T>(value: T) -> T { value }
pub fn generic_call(value: i64) -> i64 { identity(value) }
pub type Callback<T> = fn(T) -> T;
pub fn alias_call(callback: Callback<i64>, value: i64) -> i64 { callback(value) }
pub fn alias_value(value: i64) -> i64 { alias_call(identity, value) }
pub fn generic_callable(value: i64) -> i64 {
    let callback: fn(i64) -> i64 = identity;
    identity(callback)(value)
}
pub fn specialized_callable(value: i64) -> i64 {
    let apply = identity::<fn(i64, i64) -> i64>;
    apply(maximum)(value, 0)
}
pub fn annotated_lambda(value: i64) -> i64 {
    let get = || -> fn(i64) -> i64 { identity };
    get()(value)
}
pub fn first<T>(left: T, right: T) -> T { left }
pub fn contextual_pointer() -> fn(i64) -> bool { first(positive, negative) }
pub fn negative(value: i64) -> bool { value < 0 }
pub fn contextual_result(value: i64) -> bool { contextual_pointer()(value) }
pub fn contextual_multi() -> fn(i64, i64) -> i64 { first(maximum, minimum) }
pub fn minimum(left: i64, right: i64) -> i64 { if left < right { left } else { right } }
pub fn contextual_multi_result(left: i64, right: i64) -> i64 { contextual_multi()(left, right) }
pub fn coerced_lambdas(value: i64) -> i64 {
    let left: fn(i64) -> i64 = |n: i64| n;
    let right: fn(i64) -> i64 = |n: i64| n;
    first(left, right)(value)
}
pub fn maker<T>() -> fn(T) -> T { |value: T| value }
pub fn specialized_thunk(value: i64) -> i64 {
    let get = maker::<i64>;
    get()(value)
}
pub fn reused_callable_tuple(value: i64) -> i64 {
    let callback: fn(i64) -> i64 = identity;
    let pair = (callback, value);
    let first = match pair { (f, n) => f(n) };
    match pair { (f, n) => f(n) }
}
pub fn tuple_lambda(limit: i64, value: i64) -> bool {
    let check = |(n, enabled): (i64, bool)| enabled && n > limit;
    check((value, true))
}
pub fn invoke(predicate: fn(i64) -> bool, value: i64) -> bool { predicate(value) }
pub fn named_value(value: i64) -> bool {
    let predicate: fn(i64) -> bool = positive;
    invoke(predicate, value)
}
pub fn lambda_value(value: i64) -> bool { invoke(|n: i64| n > 10, value) }
pub fn get_predicate() -> fn(i64) -> bool { positive }
pub fn returned(value: i64) -> bool { get_predicate()(value) }
pub fn captured(limit: i64, value: i64) -> bool {
    let predicate = |n: i64| n > limit;
    let first = predicate(value);
    first && predicate(value)
}
pub fn captured_tuple(bounds: (i64, bool), value: i64) -> bool {
    let predicate = |n: i64| match bounds {
        (limit, true) => n > limit,
        (_, false) => false,
    };
    predicate(value)
}
pub fn immediate(value: i64) -> i64 { (|n: i64| if n > 0 { n } else { 0 })(value) }
pub fn constant() -> i64 { 42 }
pub fn nullary() -> i64 {
    let local = || 1;
    if local() > 0 { constant() } else { 0 }
}
pub fn maximum(left: i64, right: i64) -> i64 { if left > right { left } else { right } }
pub fn multi_argument(left: i64, right: i64) -> i64 { maximum(left, right) }
pub fn multi_lambda(left: i64, right: i64) -> i64 {
    let choose = |a: i64, b: i64| if a > b { a } else { b };
    choose(left, right)
}
pub fn shadow(value: i64) -> bool {
    let positive = |n: i64| n < 0;
    positive(value)
}
"#;

pub fn assert_executable(source: &str) {
    let consumer = r#"
fn main() {
    assert!(models::forward(1));
    assert!(!models::forward(0));
    assert_eq!(models::identity(String::from("owned")), "owned");
    assert_eq!(models::generic_call(42), 42);
    assert_eq!(models::alias_value(42), 42);
    assert_eq!(models::generic_callable(42), 42);
    assert_eq!(models::specialized_callable(42), 42);
    assert_eq!(models::specialized_callable(-1), 0);
    assert_eq!(models::annotated_lambda(42), 42);
    assert_eq!(models::coerced_lambdas(42), 42);
    assert!(models::contextual_result(42));
    assert!(!models::contextual_result(-1));
    assert_eq!(models::contextual_multi_result(2, 3), 3);
    assert_eq!(models::specialized_thunk(42), 42);
    assert_eq!(models::reused_callable_tuple(42), 42);
    assert!(models::tuple_lambda(10, 11));
    assert!(!models::tuple_lambda(10, 10));
    assert!(models::named_value(1));
    assert!(!models::named_value(-1));
    assert!(models::lambda_value(11));
    assert!(!models::lambda_value(10));
    assert!(models::returned(1));
    assert!(!models::returned(0));
    assert!(models::captured(10, 11));
    assert!(!models::captured(10, 10));
    assert!(models::captured_tuple((10, true), 11));
    assert!(!models::captured_tuple((10, false), 11));
    assert!(!models::captured_tuple((10, true), 10));
    assert_eq!(models::immediate(42), 42);
    assert_eq!(models::immediate(-1), 0);
    assert_eq!(models::nullary(), 42);
    assert_eq!(models::multi_argument(2, 3), 3);
    assert_eq!(models::multi_argument(3, 2), 3);
    assert_eq!(models::multi_lambda(2, 3), 3);
    assert_eq!(models::multi_lambda(3, 2), 3);
    assert!(models::shadow(-1));
    assert!(!models::shadow(1));
}
"#;
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("main.rs");
    let executable = directory
        .path()
        .join(format!("consumer{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&input, format!("{source}\n{consumer}")).unwrap();
    let compiled = std::process::Command::new("rustc")
        .args(["--edition=2024", "--crate-name=function_consumer"])
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
