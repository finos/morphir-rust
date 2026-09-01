# ADR-0003: Dual native invocation for built-in extensions

## Status

Accepted

## Date

2026-09-01

## Context

Morphir needs built-in Rust extensions to avoid protocol serialization on routine local calls without losing coverage of MEP. Installed extensions have different trust and isolation requirements. They run as verified process or WebAssembly artifacts and must remain behind the protocol boundary.

Provider origin and invocation mode answer different questions. Treating a built-in provider as synonymous with one invocation mode made provider selection harder to reason about and prevented callers from requesting protocol conformance explicitly.

## Decision

Trusted Rust built-ins expose typed native capability traits and native MEP dispatch from the same extension instance. Hosts use typed native invocation by default for performance. Tests and callers that need protocol parity can select native MEP instead.

The provider registry models provider origin separately from invocation mode. Built-in and installed are origins. Native direct, native MEP, process MEP, and WebAssembly MEP are invocation modes. Installed process and WebAssembly providers remain MEP-only.

Provider resolution first filters candidates by capability and supported Morphir IR version. If both origins contain an eligible provider, the installed provider takes precedence over the built-in provider. Invocation policy then chooses the permitted route. `PreferDirect` selects the typed path for a native built-in, while `ProtocolOnly` selects native MEP. Neither policy bypasses MEP for an installed provider.

## Rationale

Typed traits remove serialization and session setup from the default in-process path. Keeping native MEP on the same instance avoids a second implementation and lets conformance tests compare complete results across the two routes. Separating origin from invocation mode keeps installed override behavior independent of transport details.

## Consequences

Every native built-in must keep its typed and MEP behavior identical. The SDK adapter owns both views of one instance, and parity tests compare full protocol results rather than selected fields.

CLI applications decide which built-ins they link and register. The transport-neutral registry and daemon do not depend on a particular language extension. Installed extensions continue through verified installation, activation, MEP negotiation, and lifecycle handling.

## Alternatives considered

Envelope-only execution was rejected because it imposes avoidable overhead on trusted in-process calls. A separate native and MEP implementation was rejected because the paths could drift. Inferring invocation from provider origin was rejected because it would make protocol-only testing and future native hosts special cases.
