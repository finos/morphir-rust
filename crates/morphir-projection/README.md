# morphir-projection

Shared Morphir IR normalization for backend extensions. The crate reads IR
in the v3 or v4 format and gives a projection model: the public
declarations, their documentation, source names, dependencies and entry
point metadata, without value bodies. A backend then maps this model to its
own target.

## Usage

```rust
let package = morphir_projection::normalize(&ir_json)?; // ir_json: serde_json::Value
```

## License

Licensed under the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).
Part of [Morphir](https://github.com/finos/morphir), a FINOS project.
