---
layout: default
title: Native Elm Extension
parent: For Contributors
nav_order: 8
---

# Native Elm extension

`morphir-elm-native` is a Rust implementation of an Elm frontend and backend for
Morphir. It reads Elm source with a vendored tree-sitter grammar and writes
Morphir IR v3 or v4 natively, without migrating between the two. The same
extension generates Elm back from a distribution.

It compiles **type declarations only**. A value declaration is not dropped
silently: each one is reported as an `ELM_VALUE_SKIPPED` warning, and the
compile still succeeds.

The extension ships as `morphir-elm-binding`, both as a Rust library and as a
WebAssembly guest that the daemon installs and runs through MEP.

## What it advertises

| Field | Value |
| --- | --- |
| Extension id | `morphir-elm-native` |
| Name | `Morphir Elm (native)` |
| Language | `elm`, file extension `.elm` |
| Backend target | `elm` |
| IR versions | `3` and `4` |
| Incremental | yes |
| Fragments | no |

## Using it

Compile an Elm package with the extension named explicitly:

```sh
morphir compile --extension morphir-elm-native
```

Generate Elm from a distribution with the matching target:

```sh
morphir generate --target elm
```

Generated artifacts are one file per module, under `src/`, named after the
module path: `My.Domain.Types` is written to `src/My/Domain/Types.elm`.

## Two Elm providers

This extension does not replace [morphir-elm][morphir-elm], the JavaScript
implementation, which stays the default provider for Elm. `morphir-elm` covers
the whole Elm language, including values; `morphir-elm-native` covers types and
runs without a Node.js toolchain. Ask for the native one by extension id when
you want it.

[morphir-elm]: https://github.com/finos/morphir-elm

## The prelude option

Name resolution needs to know what `Int`, `List` and `Dict` mean. That mapping
is the prelude, chosen through the `elmPrelude` compile option:

- `"elm-core"` is the default. It maps the Elm core modules onto the
  `Morphir.SDK` package the way morphir-elm's `IncrementalResolve` does, and
  supplies the implicit imports Elm gives every module.
- `"none"` supplies nothing. Every reference has to resolve inside the package
  or its declared dependencies, and an unresolved one is reported as
  `ELM_RESOLVE_NOT_FOUND` naming `prelude: none`.
- An inline object declares a prelude of your own: `module_alias` entries map a
  source module path onto a platform one, and `package` entries declare the
  platform packages and the types their modules hold.

A dependency supplied in the compile request wins over a prelude package of the
same name, so a real `Morphir.SDK` distribution shadows the built-in
description of it.

## Incremental compilation

The extension holds no state between calls. The host holds the baseline, and
each compile decides per module whether the baseline can be reused.

A request may carry a `baseline` with one entry per module: its name, uri,
`sourceDigest`, `interfaceDigest`, `dependsOn` list and module IR. Every result
carries `moduleResults`, one entry per module in the request, with the same
fields plus a `status` and that module's diagnostics. A host feeds one run's
`moduleResults` back as the next run's `baseline`.

`dependsOn` is every in-package module the module *imports*, together with every
in-package module its type references resolved to. It is deliberately wider than
the references alone: a module imported `exposing (..)` and never named today
can still change what a bare name means tomorrow — adding a type to it can make
a name ambiguous — so a module that recorded only what it resolved would be
reused against a scope that moved underneath it. Widening to the imports is what
keeps an incremental run's answer equal to a clean run's; the cost is
recompiling a module whose unused import changed.

The baseline also carries an optional `preludeDigest`. What a name resolves to
depends on the prelude, so a baseline built with a different one describes a
different compilation: when the request's `preludeDigest` does not match the
prelude the request selects, the whole baseline is ignored and an `ELM_REQUEST`
warning says so. A host should store the digest next to the results it keeps and
echo it back on the next request; `morphir_elm_binding::frontend::boundary::prelude_digest_for`
computes it from the same `CompileOptions` the request carries. A baseline with
no `preludeDigest` is taken at face value, as before.

The statuses are:

| Status | Meaning |
| --- | --- |
| `compiled` | The module was parsed, resolved and emitted in this run. |
| `unchanged` | Its source and every dependency interface matched the baseline, so the baseline IR was reused. |
| `failed` | The module could not be compiled; it carries the diagnostics that say why. |
| `blocked` | A dependency failed and no baseline interface was available to resolve against (`ELM_BLOCKED`). |

Two digests decide the reuse. `sourceDigest` is `sha256` over the module text,
so any edit changes it. `interfaceDigest` is `sha256` over the canonical JSON of
the module's public interface, computed from the resolved model and independent
of the IR version, so it changes only when what the module *offers* changes.
That is the distinction that keeps a doc comment edit from recompiling
dependents while retyping an exposed alias does recompile them.

A module that is neither recompiled nor reusable fails rather than resolving
against a stale interface. Deleting a dependency fails its dependents with
`ELM_RESOLVE_NOT_FOUND`, and a baseline entry this version cannot read is
treated as absent, with an `ELM_REQUEST` warning saying so.

Editing a module's header away is not the same as deleting the module. A
document that no longer says which module it is still has a uri, and when the
baseline recognises that uri the document is reported as *that* module, `failed`
with its `ELM_SYNTAX` diagnostic. Its dependents then take the ordinary
failed-dependency path — the last good interface when there is one, `blocked`
when there is not — instead of being told a module disappeared. Without a
baseline match there is no name to report, so the document is dropped with its
diagnostics as before.

## Diagnostics

Every diagnostic that concerns source carries a `location` with a `uri` and a
zero-based UTF-16 range.

| Code | Reported when |
| --- | --- |
| `ELM_SYNTAX` | The grammar could not parse the module. |
| `ELM_VALUE_SKIPPED` | A value declaration is outside the subset. Warning; the compile still succeeds. |
| `ELM_RESOLVE_NOT_FOUND` | A type reference names nothing reachable. |
| `ELM_RESOLVE_AMBIGUOUS` | A bare name is offered by more than one import. |
| `ELM_DUPLICATE_TYPE` | Two declarations a Morphir document cannot tell apart, such as `Foo_Bar` and `FooBar`. |
| `ELM_IMPORT_CYCLE` | The modules in the request import each other in a cycle. |
| `ELM_TYPE_CYCLE` | A module's own type aliases stand for each other in a circle. A custom type may be recursive; an alias may not. |
| `ELM_BLOCKED` | A dependency failed and has no baseline interface to resolve against. |
| `ELM_REQUEST` | The request itself is wrong: an unsupported IR version, a foreign language id, two documents claiming one module. |
| `ELM_IR` | An IR version's reader or writer refused the document. |
| `ELM_UNSUPPORTED` | A distribution holds a construct with no Elm form. It is left out, the rest of the module is still generated, and the result is not a success. |

A result is a success only when it carries no Error diagnostic at all. That
covers diagnostics about the request rather than about a module — an unreadable
dependency distribution, say — which no per-module status would otherwise
report.

## Building the WebAssembly guest

The grammar is vendored C, so a wasm build needs a C compiler that targets
`wasm32-unknown-unknown`. Install clang and `llvm-ar` — the release task checks
for both and stops with a message naming them if either is missing — and build:

```sh
cargo build -p morphir-elm-binding --release --target wasm32-unknown-unknown
```

The release bundle, its descriptor and the offline install test run from one
task:

```sh
mise run extension:artifact:elm-native
```
