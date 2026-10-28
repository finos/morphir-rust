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

## Request identity in SDK 0.4.0

`protocol::RequestId` preserves the canonical non-negative integer or string
identifier without coercion. Numeric JSON encoding remains unchanged, including
the full `u64` range. String `"007"` and numeric `7` are different identifiers.
Null, booleans, arrays, objects, negative and fractional numbers are not request
identifiers. Notifications still omit the identifier entirely. Parameter-free requests
may omit `params`, as the canonical MEP transcripts do; the decoder supplies
an empty named-parameter object.

The public request and response `id` fields now use `RequestId`, so 0.4.0 is a
breaking Rust API release. Existing numeric `ExtensionRequest::new` and
`ExtensionResponse::success`, `error` and `from_extension_error` constructors
retain their `u64` arguments. Use `new_with_id` and the corresponding response
`*_with_id` constructors when forwarding an opaque identifier. Numeric struct
literals should use `id: numeric_id.into()`.

```rust
use morphir_extension_sdk::protocol::{ExtensionRequest, RequestId, methods};
let request = ExtensionRequest::new_with_id(
    methods::PING, serde_json::json!({}), RequestId::from("python-λ-1"),
).unwrap();
assert_eq!(serde_json::to_value(request).unwrap()["id"], "python-λ-1");
```

## Opt-in MorphirModel transport

The SDK implements the exact MEP `0.2.0-draft.1` model-operation draft alongside
legacy MEP `0.1`. Existing guests and `HostConfig::new` keep the legacy offer.
A guest opts in by supplying `ExtensionCapabilities::model_transport` with
complete supported tuples and implementing `Frontend::compile_model` or
`Backend::generate_model`. A host opts in with
`HostConfig::with_model_transport`. Advertising a tuple commits the component
to implementing that operation; the trait defaults refuse it.

A tuple selects one operation, profile, codec, envelope version, and IR version.
The handshake rejects profile/codec/version combinations unsupported by the
compiled readers, intersects complete tuples, takes the smaller resource limits,
and freezes the resulting session. It does not combine independent codec and
version lists. Rich methods are independent of legacy `compile` and `generate`
flags, so a provider can support either contract or both. Legacy JSON calls
remain available when their own flags are advertised.

The envelope version is `0.1.0-draft.1`. IR `4.0.0` remains draft, using its
existing version spelling and integer `4` document tag. The transport draft does
not create a new IR version. Ion binary is preferred for lossless model flow;
Morphir JSON remains supported through an explicitly selected tuple.

```rust
use morphir_extension_sdk::model::{ModelCodec, ModelPayload};

let payload = ModelPayload::from_bytes(ModelCodec::MorphirJson, b"{}").unwrap();
assert_eq!(payload.decoded_byte_length, 2);
```

`ModelPayload::from_bytes` packages bytes. It does not admit a document.
`retain_model` checks the selected envelope/profile, canonical base64, exact
length, SHA-256, critical member paths, and aggregate payload/depth/node limits.
It returns an immutable `RetainedModel` with the exact original bytes. Ion
annotations, numeric precision, unknown payload fields and their order survive
without transcoding. The JSON reader retains exact numeric lexemes and rejects
duplicate keys. External Ion shared symbol catalogs are not resolved. The ion-rs
reader is currently native-only because its mandatory Chrono dependency enables browser
imports. Non-browser WASM MEP guests negotiate `morphir-ir` with `morphir-json`;
Ion model tuples are refused there until a suitable reader is qualified. This
restriction is explicit and never transcodes an Ion request into JSON.

Admission checks profile structure, not full semantic IR correctness. Consumers
must run their own IR/type checks before operations that require them. Ion system
symbol-table processing uses ion-rs; depth/node budgets count application values,
and the encoded/decoded byte bounds also cover system-table bytes. JSON traversal
uses bounded heap storage and may rescan at each depth, with worst-case work
proportional to document bytes times admitted depth. Choose finite limits for the
workload.

Optional evidence is retained and bounded. Identity, revision, scope and context
bindings do not authenticate its producer or prove its claim. The default policy
refuses every required remote guarantee, including a matching attributed claim.
There is no built-in checker or configurable trust-policy hook in this slice.
Guarantee-policy integration is separate work.

The shared guest protocol session and `morphir-host::Session` apply admission to
rich requests and successful compile results, including generic calls. Invalid
requests never reach the component. Invalid remote results close the host session
without a pool retry. Host-native rich generation reuses artifact path, duplicate
output and binary-content validation. Native components use the protocol adapter
for these methods; the older direct `NativeFrontend`/`NativeBackend` interfaces
remain legacy-only. This slice does not qualify the MoonBit or WASM execution
routes against the MCK.

These are handwritten interim bindings for the canonical
[contract at 63563784](https://github.com/finos/morphir/tree/63563784d69405781949f4db67090ec9baa4c82f/spec/mep).
[Source-first generated bindings](https://github.com/finos/morphir-rust/issues/241)
and the canonical executable MEP MCK remain separate work. Vendored canonical
fixtures are unit-test inputs, not a second compatibility runner or a qualification
claim.

### Rust source migration

Wire compatibility with legacy MEP is preserved. Rust struct literals for
`InitializeParams` need `model_transport: None`; `ExtensionCapabilities` literals
need the same field or `..Default::default()`. This source change must be accounted
for when selecting an SDK release. This PR does not bump a crate version or pin
an ecosystem submodule.

## License

Licensed under the [Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).
Part of [Morphir](https://github.com/finos/morphir), a FINOS project.
