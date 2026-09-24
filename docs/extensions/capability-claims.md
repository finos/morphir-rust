# Capability claim sets

An extension describes its identity, supported MEP versions, capabilities, and host
requirements in a **capability claim set**. Its entries are **claims**. The SDK
returns this document from `morphir.extension.describe`, before initialization.
The method name and MEP protocol version `0.1` are unchanged.

```json
{
  "claimsVersion": "0.1.0-draft.2",
  "protocolVersions": ["0.1"],
  "extension": {
    "id": "example",
    "name": "Example",
    "version": "1.0.0",
    "types": ["backend"]
  },
  "capabilities": {
    "backend": {"targets": ["text"], "irVersions": ["3"], "generate": true}
  }
}
```

The optional `requires` and `critical` members retain their meaning. Unknown
optional capability members survive a read and write. Readers refuse critical
paths they do not understand; prefix conversion does not make an unknown claim
understood.

Bundle descriptors, index artifact records, and installed catalogs use the
`claims` member to hold this document. `claimCheck` is `unchecked` for publisher
metadata or `probed` after a host probe. `probeSource` still identifies the route
as `describe` or `session-fallback`. A declaration alone does not establish that
the extension implements its claims.

Writers use document version `0.1.0-draft.2` and record schema version
`2.0.0-draft.2`. Readers accept the exact draft.1 and draft.2 versions, using
SemVer exact comparators. A draft.1 read converts `statementVersion`, `statement`,
and `statementSource` into the current model. The old `declared` check becomes
`unchecked`, and record critical paths beginning with `statement.` become
`claims.`. Paths below `artifacts` or `extensions` are converted in the same way.
A record whose `schemaVersion` names one draft but whose members or critical
paths use the other draft's names is refused. A record without a draft
`schemaVersion`, such as a `1.0` catalog, is converted by its member names.

Version-1 flat formats and their conversion are unchanged. Converted flat
metadata is available as unchecked claims in memory without adding a document to
the version-1 wire format. Hosts `0.4.0-beta.6` and earlier cannot read draft.2
records or `describe` responses.

Rust callers use `morphir_extension_sdk::claims::CapabilityClaimSet`,
`CLAIMS_VERSION`, `ClaimsRequirements`, `ClaimsAgreementError`, and `check_claims`.
Distribution callers use `ClaimsRecord`, `ClaimCheck::{Unchecked, Probed}`,
`claims()`, and `claim_check()`. There are no aliases for the previous draft API.

This terminology follows finos/morphir#921 and kb `morphir-extensions` decision
0007, "Extensions make capability claims".

## Packaging WASM extensions

All six `mise run extension:artifact:<id>` tasks write a version-2 `release.json`
with `schemaVersion: "2.0.0-draft.2"`. Publishing these bundles requires Morphir
CLI **0.4.0-beta.7 or later**. Older CLIs cannot read these descriptors.

The packager reads the built WASM once, writes those bytes to a private temporary
file, and runs:

```console
cargo run --quiet --locked -p morphir-host-native --bin extension-claims -- <temporary-guest.wasm>
```

The descriptor embeds the guest's complete `morphir.extension.describe` answer
under `artifacts[0].claims`, including optional members. Packaging fails before
writing a bundle if the tool fails or its claims disagree with the registry's
extension ID, name when declared, MEP versions, capability kinds, languages and
file extensions, targets, IR versions, incremental flag, or workspace discovery.
The claimed extension version must match the registered crate's `Cargo.toml`.
`.github/extensions.toml` remains the declared source of release capabilities;
packaging never replaces a guest claim with a registry value.

A bundle descriptor has this structure. The digest below is illustrative:

```json
{
  "schemaVersion": "2.0.0-draft.2",
  "shortId": "avro",
  "extensionId": "morphir-avro",
  "version": "0.1.0",
  "artifacts": [{
    "runtime": "wasm",
    "filename": "morphir-avro-extension-0.1.0.wasm",
    "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    "claims": {
      "claimsVersion": "0.1.0-draft.2",
      "protocolVersions": ["0.1"],
      "extension": {
        "id": "morphir-avro", "name": "Morphir Avro",
        "version": "0.1.0", "types": ["backend"]
      },
      "capabilities": {
        "backend": {"targets": ["avro"], "irVersions": ["3", "4"], "generate": true}
      }
    }
  }]
}
```

WASM artifacts must omit `platform`, including a null value. A single artifact
needs no `platformDifferences` policy. The descriptor retains optional
`gitCommit` for a clean HEAD snapshot; a dirty worktree omits it. The legacy
`package` member remains in the registry rather than the descriptor. Name,
protocol versions, and capability metadata now live in the guest's claims.

The staged directory still contains exactly the versioned `.wasm`, its
`.wasm.sha256` checksum, and `release.json`. Release publication names the
last file `<artifact-base>-<version>.release.json`, preserving the existing
fetch contract. Rename that downloaded file to `release.json` before running
`morphir extension repository publish`. Asset selection checks the exact
version-2 envelope, registry agreement, commit, and both checksum locations;
it uploads the original bytes.

The CLI does not probe WASM artifacts at publication or installation. The
installed catalog retains `claims` with `claimCheck: "unchecked"`, and
`morphir extension list` prints `Claims: unchecked`. Packaging-time validation
does not mark the host's claim check as probed. Run
`mise run test:cli-release <id>` to verify publication, installation, and use
through the pinned CLI.
