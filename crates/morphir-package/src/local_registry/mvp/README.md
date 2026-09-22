# Fresh local Library resolution and restore MVP

`initialize` provisions a new directory with an independently supplied exact root
pin. `restore` accepts one local registry, a fresh-metadata policy and a complete
lock. Both policy and lock retain the existing draft.3 wire format. Reports name
`local-library-mvp`, version `0.1.0-draft.1`; this is not a filesystem qualification.

`resolve` takes a fixed published `ReleaseId`, one local registry and initialized
trust state. It authenticates the current target view, rejects malformed,
duplicate or conflicting records and checks record/statement associations before
filtering candidates. Active releases in authorized namespaces enter the unchanged
draft.2 resolver; yanked releases cannot enter a new graph. The selected graph
passes the same publisher, bundle and complete Library checks as restore before
the runtime atomically publishes a new full draft.3 lock file. The host does not
write the lock. Existing files and symlinks are refused; its parent must exist.
Resolve's report includes the normalized graph and verified releases, with their
future restore directory names. Resolve does not publish package directories.

Lock generation is deterministic: registry alias `local`, metadata evidence IDs
`root`, `snapshot`, `targets`, `timestamp`, and `statement-N` IDs assigned in
lexical release order. Acquisitions follow that order, evidence sorts by ID, and
the graph retains the resolver's root-first order. The artifact contains explicit
draft.3 discriminators, pretty JSON and a final LF, and passes strict full-lock
decoding before publication. A generated lock can be consumed by `restore`.

Any authenticated revoked record refuses the entire resolve operation, including
when that release would not be selected. The unresolved-operation marker prevents
a later active assertion from clearing that observation. This is a deliberately
stricter MVP limitation while durable revocation transitions remain deferred to
finos/morphir#912; it is not a production revocation implementation. Initial
resolve does not implement registry refresh or old-lock update.

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
mutation; **every operation failing after its marker is created leaves it in place** and subsequent
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
