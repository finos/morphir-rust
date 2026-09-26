# morphir-core

The Morphir IR model for Rust. The crate defines names, paths and fully
qualified names, the types and values of the IR, and the distribution
formats. It reads and writes Morphir IR as JSON or YAML, in the classic
(v3) format and the v4 format, and migrates classic IR to v4.

## Usage

```rust
let fqname = morphir_core::naming::FQName::parse("morphir.sdk:basics:int");
```

## License

Licensed under the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).
Part of [Morphir](https://github.com/finos/morphir), a FINOS project.
