# Proposal

## Why

The released Rust extension (`morphir-rust-binding`, `extension/rust/v0.3.0`) declares MEP `0.1` only. Hosts
that use the MEP `0.2.0-draft.1` model operations, such as the MoonBit CLI worker host, can only run it on the
legacy line. The Morphir maintainers want every extension on the latest MEP. The Gleam extension
(`extension/gleam/v0.4.0`) and the Elm extension (`extension/elm/v0.5.0-preview.2`) already declare it, and
`morphir-extension-sdk` 0.4.0 implements the model operations.

## What Changes

- The Rust extension declares MEP `0.2.0-draft.1` in addition to MEP `0.1`. A host that offers only `0.1`
  continues to get the `0.1` line with no model transport.
- The frontend serves `morphir.frontend.compileModel` and the backend serves `morphir.backend.generateModel`
  for one tuple each: profile `morphir-ir`, codec `morphir-json`, envelope `0.1.0-draft.1`, IR `4.0.0`.
- The extension does not offer Ion. The released artifact is a WASM guest, and the SDK has no Ion reader for
  WASM targets.
- The capability claims (`claimsVersion` `0.1.0-draft.2`) carry the new protocol version and the model
  tuples. The registry entry in `.github/extensions.toml` declares both MEP versions.
- Release `extension/rust/v0.4.0` with the same asset shape as `v0.3.0`: the WASM, its `.wasm.sha256` and the
  version-2 `release.json` descriptor.

## Capabilities

### New Capabilities

- `rust-extension-model-operations`: how the Rust extension negotiates MEP versions and model transport
  tuples, compiles Rust source into a model envelope, generates Rust from a model envelope, and refuses
  requests outside the agreed tuple.

### Modified Capabilities

None.

## Beads

- **Tracked by**: `morphir-0xtc`
- **Related**: None found

## Impact

- `crates/morphir-rust-binding`: new model transport module, capabilities, tests and README.
- `.github/extensions.toml`: `mep_versions` of the `rust` entry.
- `CHANGELOG.md`, the binding's version (0.3.0 to 0.4.0) and `Cargo.lock`.
- No change to `morphir-extension-sdk` or to other repositories. The finos/morphir bundle pins follow in a
  separate pull request.
