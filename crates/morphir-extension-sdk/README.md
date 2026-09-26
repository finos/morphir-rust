# morphir-extension-sdk

The SDK for Morphir extensions. An extension is a WebAssembly plugin that
the Morphir host loads, for example a frontend that compiles a language to
Morphir IR or a backend that generates code from it. The crate gives the
extension traits and the JSON-RPC 2.0 protocol types. The protocol types
also compile on native targets, so a host can use them too.

## Usage

```rust
use morphir_extension_sdk::prelude::*; // Extension, ExtensionInfo, ExtensionCapabilities
```

## License

Licensed under the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).
Part of [Morphir](https://github.com/finos/morphir), a FINOS project.
