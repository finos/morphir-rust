//! Compiles the vendored tree-sitter-elm grammar (grammar/UPSTREAM records the version).
//! On wasm32-unknown-unknown there is no libc; tree-sitter-language ships shim headers and
//! tree-sitter 0.27 provides the runtime functions, so we only add the include path.

use std::path::PathBuf;

fn main() {
    let grammar = PathBuf::from("grammar");
    let mut build = cc::Build::new();
    build
        .include(&grammar)
        .file(grammar.join("parser.c"))
        .file(grammar.join("scanner.c"))
        .warnings(false)
        .std("c11");
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.starts_with("wasm32-unknown") {
        let headers = std::env::var("DEP_TREE_SITTER_LANGUAGE_WASM_HEADERS")
            .expect("tree-sitter-language exports wasm headers for wasm32-unknown targets");
        build.include(headers);
    }
    build.compile("tree_sitter_elm");
    println!("cargo:rerun-if-changed=grammar/parser.c");
    println!("cargo:rerun-if-changed=grammar/scanner.c");
    println!("cargo:rerun-if-changed=grammar/tree_sitter/parser.h");
    println!("cargo:rerun-if-changed=grammar/tree_sitter/alloc.h");
    println!("cargo:rerun-if-changed=grammar/tree_sitter/array.h");
    println!("cargo:rerun-if-changed=grammar/UPSTREAM");
}
