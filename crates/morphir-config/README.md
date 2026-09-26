# morphir-config

Portable Morphir configuration rules for Rust. The crate parses Morphir
configuration from TOML, YAML or JSON text, merges configuration layers in
order of precedence, reads overrides from environment variables and handles
secret references. It does not read files, so it runs in any host, including
WebAssembly.

## Usage

```rust
let config = morphir_config::parse_config("morphir.toml", "[ir]\nformat_version = 4\n")?;
```

## License

Licensed under the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).
Part of [Morphir](https://github.com/finos/morphir), a FINOS project.
