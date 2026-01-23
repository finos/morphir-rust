use std::process::Command;

fn main() {
    // Set build date
    let date = Command::new("date")
        .args(["+%Y-%m-%d"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rustc-env=BUILD_DATE={}", date);
}
