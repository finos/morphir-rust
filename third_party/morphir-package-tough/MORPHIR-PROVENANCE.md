# Package-local Tough adaptation

Source: [`tough` 0.24.0](https://crates.io/crates/tough/0.24.0), upstream
[awslabs/tough](https://github.com/awslabs/tough). The published crate archive has
SHA-256 `35b378d98765c2ae9cdc3e9963ea7e670da8cdd9ee39611b8d722083c7f1ac11`.
The upstream source, integration tests, test assets, README and MIT/Apache-2.0
licenses are retained. Package metadata is renamed to `morphir-package-tough`;
the library remains `tough` for upstream imports and doctests. Consumers use a
separate Cargo dependency alias. There is no global Cargo patch or replacement
of the CLI tool-update dependency.

## Approved adaptation

The package trust profile requires positive integer metadata versions without a
`u64` ceiling. `schema::Version` stores a validated canonical decimal integer,
orders by digit count then digits, and implements exact successor/predecessor.
All signed role and metafile versions, editor version inputs and version error
fields use this type. Role filenames and persisted metadata retain exact digits.
Root-update limits count accepted transitions in a separate bounded `u64`; they
are not computed by adding a limit to a signed version. The root-cache iterator
likewise decrements the exact version without narrowing it.
It retains upstream's traversal back to version 1 and is not the bounded
historical-chain operation required by the package profile. Package integration
must not use that method as its historical evidence provider.

JSON deserialization reads the actual raw token and accepts only positive decimal
integer tokens. In particular it rejects an object impersonating serde_json's
private arbitrary-precision number representation. Serialization emits a JSON
number with `serde_json/arbitrary_precision`; no float or string substitution is
used. `serde_json/raw_value` is enabled for the token boundary.

Signature algorithms, verification and upstream update ordering are unchanged.
This adaptation supplies the fixed host-clock hook described below. It does not
yet supply durable storage, raw-key quorum or assurance-provider integration. Passing these tests is not
qualification of a production package restore operation.

## Test maintenance

Upstream test code has mechanical conversions where a version formerly used
`NonZeroU64`; signature thresholds remain `NonZeroU64`. Original test assets are
unchanged. Two additional `.pkcs8` fixtures normalize the old `targetskey` and
`targetskey-1` encodings to PKCS#8 v1, preserving each original Ed25519 seed and
omitting the legacy explicitly tagged public-key field. Current aws-lc rejects
those original encodings; the same failures were reproduced on unmodified Tough
0.24.0. A regression test derives and compares both public keys with the originals.
Only affected editor-test fixture paths use the normalized files. Production key
parsing and verification are unchanged. No failing upstream test was disabled.
The existing ignored `expired_root_json_signature_is_err` unit test remains as
upstream documents: it does not actually test expiration.

The standalone Cargo.lock records the dependency resolution used to run this
vendored suite. The consuming workspace has its own authoritative lockfile.
`tests/exact_versions.rs` covers the machine boundaries, exact ordering and
successors, all role/metafile JSON versions and filenames, invalid tokens and
fixture identity. Independently signed loader, root-transition and process-restart
coverage lives in the consuming `morphir-package` tests.

```sh
cargo test --locked --manifest-path third_party/morphir-package-tough/Cargo.toml
cargo fmt --manifest-path third_party/morphir-package-tough/Cargo.toml --all --check
```

## Patch inventory

- `Cargo.toml`: renamed non-publishable package, added `arbitrary_precision` and
  `raw_value` serde_json features and the exact-version test target. Upstream
  optional features are retained; no default feature is added. `Cargo.lock` is
  generated for this standalone test package; `.gitignore` excludes its target.
- `src/schema/version.rs`: validated exact-integer type and JSON boundary.
- `src/schema/mod.rs`: role/metafile version fields, constructors and accessors.
- `src/error.rs`: exact version values in error payloads.
- `src/lib.rs`: separate root-transition counter, exact root successor, and
  necessary clones of version values in diagnostics.
- `src/cache.rs`: exact descending root-version traversal.
- `src/editor/mod.rs`, `src/editor/targets.rs`: exact version inputs and clones.
- `src/editor/test.rs`, `tests/repo_editor.rs`,
  `tests/raw_metadata_verification.rs`, `tests/target_path_safety.rs`,
  `tests/rotated_root.rs`: version API conversions and the fixture paths described
  above. `tests/data/targetskey.pkcs8`, `tests/data/targetskey-1.pkcs8` are the
  additional normalized test keys; `tests/exact_versions.rs` proves their identity
  and the version invariants.

All other copied source and upstream test files are unchanged. In particular,
`src/schema/verify.rs` and `src/sign.rs` are unchanged. The narrowly adapted
`src/datastore.rs` time source is described below.

## Fixed operation clock

`RepositoryLoader::fixed_time(jiff::Timestamp)` accepts the trusted host's time
captured before a bounded current operation. The loader's datastore reuses that
value for authentication, every target read and the existing known-time rollback
check. Without the override, ambient system-clock sampling is unchanged.
Combining fixed time with `ExpirationEnforcement::Unsafe` is rejected before
loading or changing the datastore, regardless of builder order. This API is not
exposed as a package-controlled setting or CLI option; callers must begin a new
operation with a new loader and time.

The shared role expiration comparison changes from `time <= expires` to
`time < expires`. The former accepted equality during metadata loading while
upstream target reads already rejected it. A failing equality regression with
original signed metadata reproduced the inconsistency. The strict comparison
matches the pinned TUF specification's requirement that expiration be higher
than the fixed operation start time. All other default expiration behavior is
retained.

`tests/fixed_time.rs` covers before/equal/after expiration, repeated top-level and
delegated target reads, known-time recording and rollback at both load/read, and
rejection of fixed-time/unsafe combinations in either builder order. A unit test
checks exact boundaries for all four role types. These tests use original signed
fixtures; they do not re-sign modified timestamps through the library under test.

The datastore still has upstream write ordering and durability limitations. Its
existing known-time record is not a package accepted-time floor. This clock-only
hook does not qualify a production provider or supply the pending transactional
storage/recovery integration.
