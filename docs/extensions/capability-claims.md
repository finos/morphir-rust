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
Mixing one draft's members with the other draft's version is an error.

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
