# morphir-workspace

Portable Morphir workspace discovery. The crate finds the projects of a
Morphir workspace in a file tree that the caller gives, applies the
workspace configuration and keeps every path inside the workspace root. It
does not read the file system itself, so it runs in any host, including
WebAssembly.

## Usage

```rust
let response = morphir_workspace::discover(request); // request: DiscoveryRequest
```

## License

Licensed under the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).
Part of [Morphir](https://github.com/finos/morphir), a FINOS project.
