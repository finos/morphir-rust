# Morphir package

`morphir-package` implements the experimental Library integrity contract
`0.1.0-draft.1` and the bounded `flat-library` resolution operation from
`0.1.0-draft.2`. Applications can use it independently of the Compatibility Kit.

```rust
use morphir_package::{digest::Digest, metadata::NormalizedMetadata};

let metadata = NormalizedMetadata::parse(r#"{ "b": [], "a": "value" }"#)?;
assert_eq!(metadata.as_str(), r#"{"a":"value","b":[]}"#);
let manifest_digest = metadata.manifest_digest();
let package_content_digest = metadata.content_digest();
let exact_file_digest = Digest::of_bytes(b"stored file bytes\r\n");
# Ok::<(), morphir_package::InvalidDocument>(())
```

Metadata contains only objects, arrays and printable ASCII strings. Normalization
rejects a BOM, duplicate decoded keys, unsupported values and depths above 64
edges from the root. It sorts keys by ASCII order and preserves array order.
File hashes include every byte; package content hashes use the draft domain prefix.

`PackageSchemas::compile` compiles caller-supplied draft 2020-12 manifest and
lock-core schemas. Local references resolve from those schemas. A custom
retriever refuses every network or filesystem request, even when other Cargo
dependencies enable the JSON Schema library's retrieval features. Invalid schemas
and unresolved references return `SchemaError`; an invalid document returns
`false` from `validate`. Schema validation does not apply metadata normalization.

Create `LibraryInput` values from manifest text and named file bytes, then call
`VerifiedLibrarySet::verify` with compiled schemas and lock-core text. Verification
checks normalized metadata, schemas, exact content membership and digests, the
current Rust IR v4 codec without legacy warnings, Library identity, dependency
keys, public exports and the closed lock graph. Version interval comparisons
preserve arbitrarily large decimal components. The returned set exposes verified
normalized manifests and its root manifest through read-only accessors.

The draft lock-core is a graph projection. `resolution::resolve_library` accepts
raw draft-2 JSON and returns either a normalized selected graph or a typed domain
diagnostic. It validates inputs and old locks in the contract's ten phases,
backtracks over the complete finite catalog, ranks initial and update graphs
deterministically, and distinguishes update-scope conflicts from requirements
that need consumer-scoped coexistence. Stable versions use exact unbounded
decimal comparison. Inputs remain immutable.

```rust
use morphir_package::resolution::{ResolutionDiagnostic, ResolutionResult, resolve_library};

let result = resolve_library(r#"{
  "formatVersion":"0.1.0-draft.2",
  "capability":"flat-library",
  "mode":"initial",
  "root":{
    "release":{"packagePath":"example.com/app/root","version":"1.0.0"},
    "irPackageName":"example/app",
    "manifestDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000",
    "contentDigest":"sha256:0000000000000000000000000000000000000000000000000000000000000000",
    "dependencies":[]
  },
  "catalogs":[]
}"#)?;
assert!(matches!(result, ResolutionResult::Resolved(_)));
# Ok::<(), morphir_package::resolution::ResolutionExecutionError>(())
```

Graphs or diagnostic witness paths above 512 selected releases, or requests
that exceed 100,000 aggregate search and witness work units, return
`ResolutionExecutionError`. The resolver never reports resource exhaustion as
unsatisfiable. This crate does not implement a registry, archive extractor,
installer, full `morphir.lock` writer or public
specification compatibility checks.

The separate `morphir-mck-adapter` executable selects integrity operations with
`--suite package` and resolution with
`--suite package --contract 0.1.0-draft.2`. It handles wire validation and
framing only. Corpus loading, fixed expectations, comparisons and reporting stay
in the shared Rust `morphir mck` driver. Running it without arguments, or with
`--suite ir`, keeps IR protocol v1.

```sh
mise exec -- cargo test --locked -p morphir-package -p morphir-mck-adapter
mise exec -- cargo clippy --locked -p morphir-package -p morphir-mck-adapter --all-targets -- -D warnings
```

## Package TUF admission (PKG-3 development)

`local_registry::tuf` supplies the production metadata admission layer for the
approved package profile. `ProfileAdmission::load_metadata` wires strict bounded
acquisition, fixed trusted time, protected storage and transition admission into
the package-local Tough workflow. The tool-update dependency is unchanged.

Admission checks the four top-level roles, specification and algorithm profile,
duplicate decoded JSON members, exact integers, metadata limits, distinct raw
Ed25519-key quorum, required SHA256 and advertised SHA512, exact envelope lengths
and versions, and forward root continuity from the trusted policy bootstrap.
The shared 256 MiB metadata budget is checked as chunks arrive, includes other
operation inputs reported by the protected inventory, and counts exact retries
once. Original envelope bytes are retained. Unknown signed fields remain signed.
An equal timestamp returns `NoUpdate` before expiry/link fetches; it does not
create fresh authority or replace the retained timestamp.

A host must implement `AdmissionBackend` over an explicitly initialized protected
store and keep process access serialized. Each operation requires an existing
durable marker binding its repository, initial root, predecessor revision and
fixed time. Evidence must be durably recorded before the loader receives it;
transitions recheck that evidence and the evolving predecessor. Retained roles
must authenticate under a root in the provisioned forward chain. Any backend
error invalidates the in-process admission session, including when uncertain new
rows happen to be visible afterward. Restart recovery must independently
reconcile the store before constructing another session.

The backend used in admission tests is deliberately in-memory and test-only.
This slice does not deliver the production SQLite store, historical restore,
recovery/revocation rules, target package-view validation, publisher authorization,
package installation or grant reuse. Internal admission errors are not a new wire
report format. Native filesystem/SQLite candidate probes remain separate evidence;
this module does not claim a qualified provider or completed PKG-3.
