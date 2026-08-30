---
layout: default
title: "Historical: WASM Component Model"
nav_exclude: true
search_exclude: true
---

# Historical WASM Component Model design

> **Historical design, not a supported extension contract.** This page records
> rationale from an earlier WIT and WebAssembly Component Model proposal. The
> current implementation uses Extism plus MEP JSON-RPC envelopes. The CLI
> installs extensions by ID from a controlled index. Do not use this page as an
> implementation or installation guide.

For current guidance, start with the [Morphir Extensions overview](./README.md).
The controlling public documents are:

- [WASM extension runtime and Avro backend](https://morphir.finos.org/docs/design/proposals/wasm-extension-runtime-and-avro-backend)
- [Morphir Extension Protocol](https://morphir.finos.org/docs/design/draft/extensions/protocol)
- [Extension distribution and acquisition](https://morphir.finos.org/docs/design/draft/extensions/distribution-and-acquisition)
- [Generate Apache Avro](https://morphir.finos.org/docs/generate/avro)

## What this design explored

The proposal modeled Morphir IR and each extension capability as WIT types. A
component would export separate frontend, backend, validator, transform, and
workspace interfaces. Worlds would combine those interfaces with explicit
virtual filesystem imports.

The design tried to answer several useful questions:

- How can a portable guest receive Morphir IR without ambient filesystem or
  network access?
- Which host functions should a guest import?
- How should a guest declare optional capabilities?
- Which Morphir names and types need a stable boundary representation?
- How can the host constrain access to workspace files?

Those questions still matter. The current implementation answers them with a
different ABI.

## Why the implementation differs

MEP keeps the protocol independent from the runtime engine. Process and WASM
extensions use the same JSON-RPC lifecycle and typed capability model. Extism
provides the current WASM engine and guest adapter, while
`morphir-extension-sdk` supplies the shared request and response types.

This avoids maintaining a second WIT operation contract alongside MEP. It also
lets the host run the same conformance cases against process and WASM adapters.
A future Component Model adapter could still carry MEP, but it would be another
runtime adapter rather than a replacement extension protocol.

## Historical type choices

The WIT proposal made several choices worth retaining as design input:

| Concern | Historical choice | Current relevance |
|---|---|---|
| Morphir names | Canonical string forms for names, paths, QNames, and FQNames | MEP payloads still need stable Morphir identity spellings. |
| Attributes | A JSON-like document value because WIT has no generics | MEP uses JSON values directly. |
| Files | Explicit virtual filesystem reader and writer imports | Current WASM guests return artifacts; the host validates and writes them. |
| Capabilities | Separate optional interfaces and composed worlds | Current initialization negotiates typed capability records. |
| Isolation | No implicit filesystem, network, or environment access | This remains the current WASM guest boundary. |

The detailed WIT package trees, generated bindings, component manifests, raw
file discovery rules, archive format, and installation commands were never the
released contract. They have been removed here to prevent the historical draft
from being mistaken for current contributor instructions.
