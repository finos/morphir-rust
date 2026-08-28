# ADR-0002: Validated typestate MEP sessions

## Status

Accepted

## Date

2026-08-27

## Context

MEP transports receive permissive JSON wire data. Earlier host adapters stored that data directly as application state and each adapter implemented its own lifecycle checks. This allowed discovery and initialization metadata to disagree, duplicate capability kinds, malformed JSON-RPC envelopes, and continued use after a transport failure whose outcome was unknown.

## Decision

The host separates transport, wire, and application concerns.

- `MepTransport` is an object-safe I/O boundary. It exchanges untrusted requests and responses and reports whether failure leaves the peer stopped or indeterminate.
- `Session<T, State>` owns request correlation, envelope validation, negotiation, capability checks, and lifecycle transitions.
- `InitializeResult` remains a wire DTO. Successful validation produces `NegotiatedSession`.
- Session state types are `Loaded`, `Ready`, `Stopped`, and `Indeterminate`. Only `Ready` exposes operation invocation and shutdown.
- Discovery metadata is authoritative when a transport has it. Initialization must agree on name, version, and capability kinds. A transport that knows only an ID validates only that ID.
- Duplicate capability kinds are invalid instead of being silently normalized.

## Rationale

Keeping wire DTOs permissive lets the host diagnose peers that violate MEP. Converting them at one boundary prevents invalid data from becoming trusted session state. Consuming typestate transitions make invalid lifecycle calls fail to compile. The explicit `Indeterminate` state prevents a timeout or lost connection from being mistaken for a clean stop.

## Consequences

Operations return the next session value along with their result. Callers must handle rejected operations separately from failures that change lifecycle knowledge. Runtime registries may erase the generic state behind an enum, but that enum must preserve all four states.

Existing object-safe session callers can migrate through a compatibility interface while transports move to `MepTransport`. New transports should implement only `MepTransport` and use the shared controller.

## Alternatives considered

Keeping mutable state enums in every adapter was rejected because it duplicates validation and permits adapter drift. Making wire DTOs strict was rejected because deserialization errors would lose useful protocol diagnostics. Treating every transport failure as stopped was rejected because connected daemons and cancellation races cannot always prove that result.
