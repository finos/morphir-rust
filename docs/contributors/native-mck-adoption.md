---
layout: default
title: Native MCK adoption
parent: For Contributors
nav_order: 12
---

# Native MCK consumer preparation

The `check:kit-native` task uses the native CLI from `finos/morphir`. The CLI owns
case validation, coverage, schema checks, adapter execution and report
adjudication. Repository TypeScript only acquires the CLI and runs these commands.
Bun is a development and CI dependency; the native CLI and Rust adapter execute
without it.

The existing `check:kit` production gate remains active on Linux until a real
release pin and managed kit are committed. Native acquisition/orchestration tests
run on Linux, macOS and Windows meanwhile. `0.4.0-beta.2` is the intended first
native release. No checksum or release evidence is assumed while it is unavailable.

## Activation after publication

1. Verify the published `finos/morphir` release archives and their SHA-256 files.
   Add `.config/mck-cli.json` with an exact `version` and a `sha256` object keyed
   by all six native release target triples. Each value is the corresponding
   archive's lowercase SHA-256. Archive names follow
   `morphir-VERSION-TARGET.tgz`, or `.zip` for Windows. The task rejects missing
   targets and malformed hashes.
2. Use the checksum-verified released CLI to vendor the approved full parent
   commit into `vendor/morphir-mck`. Commit the whole native-managed snapshot,
   including its `mck-kit.lock.json`. Keep the source revision and archive
   checksums in the adoption review evidence.
3. Run `mise run check:kit-native` on the supported CI operating systems and retain each
   report. Verify fresh-home, cache-free offline operation with the installed CLI
   and the same snapshot separately. A local CLI override is preparation evidence,
   not proof of the published release.

The installer verifies the archive before extracting its single executable. It
publishes the executable and receipt by a directory rename, checks cached binary
bytes on reuse, and supports both `.tgz` and Windows `.zip` archives. If a cache
entry is corrupt, remove the named directory and retry. Downloads never replace
a good cache entry with partial contents.

## Local preparation

Export `MORPHIR_CLI` as an absolute path to a local native CLI with the IR-3 commands.
Create a managed kit with that CLI, then select it explicitly:

```sh
"$MORPHIR_CLI" mck kit vendor --source embedded --dest .agents/out/native-kit
MORPHIR_MCK_KIT=.agents/out/native-kit mise run check:kit-native
```

PowerShell supports the same environment overrides:

```powershell
& $env:MORPHIR_CLI mck kit vendor --source embedded --dest .agents/out/native-kit
$env:MORPHIR_MCK_KIT = '.agents/out/native-kit'
mise run check:kit-native
```

All authoring checks, execution and report checking receive the same explicit
kit. A missing manifest is an error. The task removes any old report before
acquisition or execution, writes `.dev/out/mck/native-report.json`, and delegates the
baseline decision to native `mck report check`. A run exiting 1 is accepted only
if that checker succeeds. A failed or incomplete session, or a signal termination
after a report was written, cannot be waived by `allowed-failing.json`. Fresh
reports also render to `native-report.html`; rendering a diagnostic report never
changes a failed gate into a success.

## Retained migration and package paths

`mise run check:kit` retains the frozen TypeScript driver pinned by
`.config/mck-driver-version`, with its v1 report at
`.dev/out/mck/report.json`. The old Rust report test reads only that file;
it never judges native reports. Retire this path only after the parent IR-4
cutover review. Package adapter protocols and `morphir-package` tests remain in
the conformance job until the separate package migration.

`test:cli-release` keeps its existing `.config/morphir-cli-version` pin and
`MORPHIR_CLI` override. Its download path shares the archive installer with MCK;
its extension compatibility policy does not change.
