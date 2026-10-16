---
layout: default
title: Provider Snapshot Releases
nav_order: 8
parent: For Contributors
---

# Provider snapshot releases

A `provider-snapshot/v<version>` tag starts the `Release provider snapshot tools`
workflow (`.github/workflows/release-provider-snapshot.yml`). The workflow
publishes native executables from the `morphir-package` crate for consumers
that qualify installed clients against the shared snapshot service.

The tag version must equal the `morphir-package` version. The tagged commit
must be on `main`. The GitHub release is a prerelease, because the
`local-library-mvp` profile is a draft.

## Targets

| Target | Runner |
|--------|--------|
| `x86_64-unknown-linux-gnu` | `ubuntu-24.04` |
| `aarch64-apple-darwin` | `macos-15` |

## Assets

For each target `T` and version `V`:

| Asset | Contents |
|-------|----------|
| `morphir-provider-snapshot-V-T.tgz` | The production one-shot service `morphir-provider-snapshot`. |
| `morphir-provider-snapshot-test-fixture-V-T.tgz` | The TEST-ONLY exporter `provider-snapshot-fixture`. |
| `<archive>.sha256` | The SHA-256 line of the archive, in `shasum -a 256` format. |

Each archive holds one executable with mode `0755`. The archives have fixed
metadata, so the same executable always gives the same archive bytes.

The release also has one descriptor, `morphir-provider-snapshot-V.release.json`.
It records:

- the tag, the package, the version and the source `gitCommit`;
- for each archive: the role (`service` or `test-fixture`), the target, OS,
  architecture, runner and runner image version, the build mode (`release`),
  the `rustc -vV` fields, the archive SHA-256 and the executable SHA-256;
- `compatibility`: the service ID, profile, profile version, codec and Ion
  annotations that the packaged service reported;
- `qualifiedFixtureVariants`: the signed fixture variants that each target
  qualified.

Each archive, each executable and the descriptor have a GitHub build
provenance attestation from this workflow.

## Qualification

The workflow builds both executables in release mode with no dependency cache.
Then it packages them, extracts them again and runs the opt-in acceptance test
against the extracted copies:

```bash
MORPHIR_PROVIDER_RELEASE_SERVICE=/abs/path/morphir-provider-snapshot \
MORPHIR_PROVIDER_RELEASE_FIXTURE=/abs/path/provider-snapshot-fixture \
  cargo test --locked --release -p morphir-package --test provider_release_acceptance
```

Default `cargo test` does not run this test. Each scenario exports new signed
test artifacts and fresh test trust into its own new directory. The test checks
the capabilities probe, each signed variant, the artifacts-only default, the
refusal of unknown options and the refusal of invalid current metadata.

Only qualified bytes get attestations and go to the release. After
publication, each target downloads the release again, verifies the checksums,
the descriptor and the attestations, and runs the same acceptance test against
the downloaded executables.

## Consumer verification

1. Download the two archives for your target, their `.sha256` files and the
   descriptor.
2. Compare each archive with its `.sha256` file and with the descriptor.
3. Verify the attestations:

   ```bash
   gh attestation verify <file> --repo finos/morphir-rust \
     --signer-workflow finos/morphir-rust/.github/workflows/release-provider-snapshot.yml
   ```

4. Extract each archive and compare the executable SHA-256 with the descriptor.

To do steps 2 and 4 in a checkout of the tagged commit, run:

```bash
python3 .github/scripts/provider_snapshot_release.py verify \
  --assets <dir> --tag <tag> --version <V> --commit <gitCommit> --target <T>
```

Never install the test fixture exporter, its test keys or its test trust as
production trust. Run the exporter with `--provision-test-trust` only into a
new, disposable directory for each test scenario.

## Release a new version

1. Make sure the `morphir-package` version is the version to release.
2. Tag a commit on `main` and push the tag:

   ```bash
   git tag -s provider-snapshot/v0.2.0 -m "provider-snapshot v0.2.0"
   git push origin provider-snapshot/v0.2.0
   ```

3. Monitor the workflow until the `verify` jobs pass.

The publish job does not replace an asset that is already in the release. If an
asset with the same name has different bytes, the job fails.
