# Tasks

## 1. Model operations in the Rust binding (morphir-0xtc.1)

- [x] 1.1 Write failing native tests for negotiation: a `0.1`-only host gets `0.1` and no `modelTransport`; a
  `0.2.0-draft.1` host with an offer gets the agreed tuples and the smaller limits; an offer without
  `modelTransport` gets `0.1`; an Ion-only offer agrees on no tuple. Verify they fail before 1.3.
- [x] 1.2 Write failing native tests for `compileModel` and `generateModel` with golden IR and golden Rust
  under `crates/morphir-rust-binding/tests/fixtures/model/`, and for the refusals (unagreed tuple, wrong
  envelope version, payload digest mismatch, model operation on the `0.1` line, required guarantee, frontend
  failure). Verify they fail before 1.3.
- [x] 1.3 Add the `model_transport` module and wire `capabilities()`, `compile_model` and `generate_model`;
  verify `cargo test --locked -p morphir-rust-binding` passes.
- [x] 1.4 Verify the compiled model's payload equals the `morphir.frontend.compile` IR v4 result and the
  generated `lib.rs` equals the `morphir.backend.generate` result for the same input (test).
- [x] 1.5 Verify `cargo clippy --locked -p morphir-rust-binding --all-targets -- -D warnings` and
  `cargo fmt --check` pass.
- [x] 1.6 Document the model operations, the tuples and the absence of Ion in the binding README; verify the
  README states the claimed protocol versions.

## 2. WASM guest, claims and release descriptor (morphir-0xtc.2)

- [x] 2.1 Set `mep_versions = ["0.1", "0.2.0-draft.1"]` for `rust` in `.github/extensions.toml`; verify
  `python3 -m unittest discover -s tests/ci` passes.
- [x] 2.2 Extend the ignored WASM host test (`tests/wasm.rs`) to describe the guest, run both model
  operations through the guest and compare with the native results; verify it passes against the release
  WASM.
- [x] 2.3 Verify the 0.1-only host paths still pass: the installed-bundle test
  `cargo test -p morphir-host-native --test rust_extension -- --ignored` (through
  `mise run extension:artifact:rust`) and `mise run test:cli-release rust`.
- [x] 2.4 Verify the built `release.json` claims hold `protocolVersions` `0.2.0-draft.1` and `0.1`,
  `claimsVersion` `0.1.0-draft.2` and the two model tuples.
- [x] 2.5 Add a `CHANGELOG.md` entry under `[Unreleased]`; verify `mise run release:changelog-validate`.

## 3. Qualify on the MoonBit worker host (morphir-0xtc.3)

- [ ] 3.1 Build the native and npm CLIs from finos/morphir-moonbit `main` and record the revision.
- [ ] 3.2 With a contract `0.2.0` pin (WASM and descriptor), run `extension describe`, `extension compile`
  (model line) and `extension generate` (model envelope) from the native CLI; check the outputs against the
  golden files and that each process exits by itself.
- [ ] 3.3 Run the same operations from the npm CLI.
- [ ] 3.4 Run compile and generate once on the `0.1` line (plain IR) from both CLIs, and record the receipts
  in the pull request.

## 4. Release extension/rust/v0.4.0 (morphir-0xtc.4)

- [ ] 4.1 Bump `morphir-rust-binding` to `0.4.0`, update `Cargo.lock`, the README version references and the
  changelog release section; verify `cargo test --locked -p morphir-rust-binding` passes.
- [ ] 4.2 Open the pull request, get CI green and handle each verified review finding.
- [ ] 4.3 Dry run on the merge candidate: `.github/scripts/extension_release.py` for the tag,
  `mise run extension:artifact:rust` from a clean checkout and `select_extension_assets.py
  --validate-short-id rust`; record the asset digests.
- [ ] 4.4 After approval, merge and tag `extension/rust/v0.4.0` on the merge commit; verify the release
  workflow publishes the WASM, `.wasm.sha256` and `release.json`.
- [ ] 4.5 Verify the published assets: digests equal the dry run, and `extension describe` on the
  downloaded bundle from the MoonBit native CLI reports MEP `0.1, 0.2.0-draft.1`.

## Workflow follow-up

- Archive the change after the release, and close the epic.
- Report the release, tag, asset digests and qualification receipts to the main session.
