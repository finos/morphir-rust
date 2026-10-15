# Morphir package

`local_registry::mvp::restore_snapshot` returns the original bytes retained by a
successful fresh exact-lock restore. Its immutable `RestoredSnapshot` contains
the complete graph, the exact policy/full-lock digests, original manifest bytes,
and each Library's native payload and authenticated context resources. Each
retained file has a validated bundle-relative path, exact bytes and raw SHA-256
digest. Native payload revisions are distinct from normalized manifest and
package-content identities.

The snapshot becomes available only after whole-graph verification, publication
and successful trust-backend finalization. It never reconstructs the result by
reading published paths. The existing `restore` API and serialized
`RestoreReport` remain available with their existing publication semantics.
Snapshot `Debug` output includes lengths and identities, not raw manifests,
payloads or context documents.

This is an in-process API under the existing `local-library-mvp` profile and
`0.1.0-draft.1` revision. It does not establish a process protocol, binary Ion
envelope, native-to-core projection, hardened filesystem assurance or a
production naming-provider release. No serialization/import constructor grants
fresh authority to saved bytes. Each later consumption operation must restore
again. Applications that need another semantic codec must explicitly qualify
their conversion from the retained native bytes.

Registry release paths and IR package names are separate identities. An
authority-qualified release such as `example.com/metadata/provider@1.0.0` can
contain the IR package `acme/metadata`. Both identities must be checked at their
respective boundaries; a bare IR name is not a registry package path.

`provider_snapshot::project_library` derives a separately identified v4.0 core
view from a retained native 4.1 Library. Its codec is
`v4.1-document-metadata-to-v4.0/1`: original document metadata stays in its exact
JSON body, and the native revision remains the original payload's digest.
The converter reparses the derived core and checks typed distribution equality.
It rejects node-local linked metadata and nonlexical parameter maps, including
nested definitions, instead of silently discarding facts or changing argument
order. These refusals require another qualified projection profile.

The signed test-only naming release in
`tests/naming` publishes
`example.com/morphir/naming@1.0.0`, containing native IR package `morphir/naming`.
Its own native facts declare the public `target-names` value with the closed
`frontend`/`backend` record of `Dict String String`. The frozen native, context,
metadata and core files are interoperability fixtures; known test signing keys
must never become production defaults. Regenerate into a new directory with
`cargo run -p morphir-package --example provider-snapshot-fixture -- <new-dir>`.

The default exports no destination trust database. Installed signed-provider tests
can explicitly request fresh test-only trust with:

```shell
cargo run -p morphir-package --example provider-snapshot-fixture -- \
  <new-dir> --provision-test-trust
```

This initializes `<new-dir>/test-state` from the exported test policy and root,
so the real one-shot service can freshly authenticate the exported registry.
The destination must not exist. Re-export refuses before changing existing files
or trust state; unknown or repeated options refuse before creating output. An
incomplete export is not repaired automatically: inspect it and choose a fresh
test destination. No engine or production host invokes this example, and its
known test keys must never become production trust defaults.

## One-shot snapshot service

Build with `cargo build --locked -p morphir-package --bin morphir-provider-snapshot`.
The executable accepts no arguments. Write one Ion 1.0 binary request to stdin,
close stdin, and receive one binary response on stdout. The binary version marker
is mandatory; text Ion and multiple application values are refused. Failure
returns nonzero, empty stdout and one bounded diagnostic code on stderr.

The protocol identity is `morphir-package/provider-snapshot/0.1.0-draft.1`. It
identifies a wire contract, not the executable. The host must independently grant
and qualify the executable, policy/lock reads, registry and already provisioned
state roots. It owns deadlines, cancellation, process-tree quiescence and scratch
disposal. Nothing installs a service or initializes trust automatically.

All structs are closed, with unique fields and exactly the declared top-level
annotation. Nested request fields have no annotations. Correlation fields are
`invocation`, a nonempty ASCII alphanumeric/underscore/hyphen string up to 128
bytes, and `projectScope`, a lowercase 64-digit raw SHA-256 string.

| Request annotation | Fields besides correlation |
| --- | --- |
| `morphir_provider_capabilities_request_v1` | None, effect-free |
| `morphir_provider_snapshot_request_v1` | `release` canonical authority-qualified stable release, `assurance` string `portable`, absolute `registry`/`state`/`output` path strings, nonempty `policy`/`lock` blobs |

`output` must be absent with an existing parent under the shared restore contract.
The full lock must name exactly the requested root before restore begins. Every
snapshot call freshly replays authentication. Trust observations can advance even
if later projection/export fails; a caller must not roll those observations back.

Both response annotations replace `request` with `response`. They repeat
correlation and add strings `service`, `profile`, `profileVersion`, `codec`.
The accepted profile/revision are `local-library-mvp` / `0.1.0-draft.1`, with
codec `v4.1-document-metadata-to-v4.0/1`. Capabilities add integer limits
`maxRequestBytes` 4194304, `maxResponseBytes` 16777216, `maxLibraries` 128,
`maxContexts` 256 per Library and `maxTotalContexts` 1024 across the graph.
Snapshot responses instead add:

| Field | Meaning |
| --- | --- |
| `release` | Exact requested root release |
| `policySha256`, `lockSha256` | Exact input byte hashes as `sha256:<64 lowercase hex>` |
| `core` | Blob containing the checked root v4.0 JSON view |
| `metadata` | Blob of original root `$meta` JSON, or `null.blob` when absent |
| `libraries` | Complete retained graph, each a closed struct of `release`, `manifest` blob, `nativePath` string, `revision` native raw hash, `native` blob and `contexts` list |

Each context is a closed struct of bundle-relative `path`, `sha256` raw-byte
identity and `bytes` blob. No files are reread to construct these exports. The
checked codec applies to the root only; dependency native payloads remain exact.
Requests are capped at 4 MiB and complete responses at 16 MiB. Oversize results
refuse without emitting a partial response. Saved frames never grant fresh
authority. The frozen binary test responses in `tests/naming` also qualify the
MoonBit consumer; their known fixture keys must not become production defaults.

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

The `package-mvp` selector advertises draft.3 `local-library-mvp:0.1.0-draft.1`
restore. The driver sends 15 digest-bound fresh-restore cases as lowercase-hex
files with a bounded input-only environment descriptor. The adapter stages
those inputs in a temporary directory, runs the production initialization and
restore APIs, and returns observed packages or narrowly classified refusals.
It verifies unchanged inputs and hashes every published regular file or
preserved output sentinel. Expected results remain in the parent repository's
MCK corpus.

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

## Library authoring and local publication

`authoring::AuthoredLibrary::create` validates a dependency-free classic V4 IR
against an explicit manifest input and derives its payload digest. The input
contains `packagePath`, `version`, empty `dependencies`, and `exports` mapping
aliases to public module names. `from_bundle` revalidates existing manifest and IR
bytes. `sign` creates an immutable registry record and publisher DSSE envelope.
`LocalSigningKey` is an explicit Ed25519 seed adapter; callers own key acquisition
and protection. Its separate `sign_tuf` method signs caller-authored repository
metadata. No signing key enters the publication transaction.

On macOS, `local_registry::publication::Registry` provides `initialize`, `open`,
`prepare`, and `publish`. Initialize absent registry and private-state directories
with an explicitly pinned, signed bootstrap root and trust policy. Both directory
parents must already exist on local APFS. Open holds an exclusive OS lock on the
anchored registry directory inode until the handle is dropped. An operation mutex
serializes threads sharing one handle; a poisoned operation refuses reuse. Configured roots
resolve once; descendants use anchored no-follow operations, and regular files
must have exactly one hard link. The private state binds both directory identities
to the bootstrap root; do not copy, delete, or reconstruct it from public metadata.

Prepare returns a serializable `Draft`, including the exact timestamp predecessor
and next unused role versions. It does not reserve versions. `Draft::sign` accepts
explicit targets, snapshot, and timestamp signers, returning exact `Proposal`
bytes. Publish independently authenticates those bytes, the current complete view,
the publisher statement, and the requested bundle. It checks the predecessor
before immutable-release/idempotence checks. A new publication retains every
prior target and status and adds exactly the requested record and statement.
An exact retry preserves yanked/revoked status and ignores an unused proposal.

The transaction durably reserves all three role versions before installing
objects. Bundles and target files are staged and promoted without replacement;
numbered metadata and timestamp evidence are retained. The sole public commit
point replaces `metadata/timestamp.json`, followed by the required directory
flush. Version floors include durable reservations, occupied metadata names, and
private commit receipts; versions use exact positive integers. An abandoned
reservation burns its versions. Restart authenticates the actual current view
and never promotes orphan files. A missing established timestamp is a refusal,
not a new empty registry.

`commit-outcome-uncertain` means the timestamp may already be visible and must not
be rolled back. Reopen and inspect the authenticated current predecessor before
preparing a retry. A retry with an old predecessor conflicts; a retry against the
observed successor is idempotent when the exact release is already present.
Private state corruption is fail-closed and requires operator investigation.

This first profile supports dependency-free classic V4 bundles throughout the
registry and uses the pinned bootstrap root without root rotation. Publication is
unavailable on other operating systems and refuses nonlocal/non-APFS filesystems.
It uses macOS full-flush operations and exclusive rename primitives. Tests cover
two-process serialization, inode-lock replacement, interrupted transactions,
version burning, and injected final-flush failure. Process termination tests do
not establish power-loss durability or complete the shared publication MCK
qualification corpus. Error codes are exposed by the runtime, but only a subset
currently has the complete draft.3 structured witness representation.
