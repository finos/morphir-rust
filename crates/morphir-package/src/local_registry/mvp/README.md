# Fresh local Library restore MVP

`initialize` provisions a new directory with an independently supplied exact root
pin. `restore` accepts one local registry, a fresh-metadata policy and a complete
lock. Both policy and lock retain the existing draft.3 wire format. Reports name
`local-library-mvp`, version `0.1.0-draft.1`; this is not a filesystem qualification.

Each restore runs the guarded TUF update, then reacquires and authenticates the
complete current metadata chain (including timestamp equality). Locked historical
metadata pins must match that fresh view exactly. Metadata advancement requires
refreshed lock evidence; historical evidence verification is deferred. Every
release needs current namespace permission, an active signed target, an authorized
publisher statement, exact hashes and lengths, and a complete valid Library graph.
Only then is the staged graph exposed at `<output>/<packagePath>/<version>`.
The destination must be absent and its parent must already exist.

The registry uses consistent-snapshot metadata and target filenames. Bundle paths
come from the full lock; each bundle has `manifest.json` and exactly its declared
content files, with only necessary nonempty directories. Symlinks, special files,
unsafe paths, unlisted content and existing destinations are refused. Bounds:
policy/root/record/statement/manifest 1 MiB, lock/targets 16 MiB, aggregate metadata
256 MiB, content file 64 MiB and complete graph content 256 MiB.

Caller-controlled trust/output roots and an immutable or coordinated registry are
required. SQLite transactions with synchronous FULL and a process-held filesystem
lock protect trust transitions. An external flushed operation marker precedes any
mutation; **every failed or interrupted restore leaves it in place** and subsequent
operations refuse it. A consistency seal detects missing or changed store rows;
it is not package authentication. Initialization never replaces an existing state
directory, even if its database is missing. Manual intervention is required after
failure. Do not delete state or clear markers to bypass a refusal or rollback floor.

No automatic recovery, historical authorization, durable continued-use grants,
hostile local filesystem defense, or power-loss recovery guarantee is provided.
Directory-entry persistence and comprehensive provider qualification remain in
finos/morphir#912. Existing downloaded files are not authorization for later use.

The embedded draft.1 package schemas are exact copies from the parent
`spec/package/schemas/` and remain part of the versioned contract. Tests use the
independently signed parent local-registry two-Library fixture, with a private
fixed test clock. Production callers cannot supply a clock or fault override.
