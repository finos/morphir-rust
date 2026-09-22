# PKG-3 candidate provider probes

These tests exercise candidate filesystem and SQLite calls. They do not implement
restore, qualify a production provider, or issue a qualification attestation.

Run on each native target with the temporary directory on the required local
filesystem:

```sh
cargo test -p morphir-package --test provider_qualification -- --nocapture
```

The environment test detects the mount/volume holding that directory and fails
unless it is Linux/ext4, macOS/local APFS or Windows/fixed-drive NTFS. It records
OS, architecture, detected filesystem and local status in test output. A claimed
OS name is insufficient. Linux matches the opened directory's mount ID from
`/proc/self/fdinfo` to `/proc/self/mountinfo`, since `statfs` alone cannot distinguish
ext2, ext3 and ext4 and overlapping paths can hide an older mount. See the
[kernel fdinfo contract](https://docs.kernel.org/filesystems/proc.html#proc-pid-fdinfo-fd-information-about-opened-file).
macOS uses `statfs` and
`MNT_LOCAL`; Windows uses volume information and fixed-drive detection. These
checks establish the test environment only; unusual storage stacks, virtual
backing disks, remote-backed block devices and dishonest hardware remain outside
what this detector can attest.

## What the tests establish

- Same-filesystem directory promotion uses `renameat2(RENAME_NOREPLACE)`,
  `renameatx_np(RENAME_EXCL)` or `MoveFileExW(MOVEFILE_WRITE_THROUGH)`. It never
  falls back to overwrite, copy/delete or reboot scheduling. A second candidate
  fails while preserving both the existing winner and the candidate bytes.
- Static hardlinks and nonregular files fail the regular-file check. POSIX tests
  create a dangling symlink and FIFO and inspect `/dev/null` without opening it.
  Windows creates a real directory junction with the built-in `mklink /J` command;
  setup failure fails the test, with no skip. Windows regular-file inspection also
  rejects `FILE_ATTRIBUTE_REPARSE_POINT` and checks native handle link count.
  Directory junction traversal and full inventory validation are not implemented.
- A separate process acquires a stable file lock. A peer cannot acquire it until
  the parent kills and reaps the holder. The lock file is never unlinked/replaced.
- POSIX file and directory `fsync` calls succeed on real objects. macOS additionally
  executes `F_FULLFSYNC` for both. A real unsupported socket flush fails and the
  helper propagates it. This is an API-error probe, not storage-failure injection.
- The test-only SQLite schema opens existing state with `READ_WRITE` and no
  `CREATE`. Explicit initialization reserves the database with `create_new`,
  uses mode 0600 on POSIX, then commits schema, identity and completion marker in
  one transaction. Existing empty, incomplete or corrupt stores are refused.
  Initialization does not reconstruct grants. Windows ACL qualification is pending.
- Every connection requests WAL and FULL, plus `fullfsync=ON` on macOS, and reads
  the effective settings back. Connections use the default native SQLite VFS.
  Exact evidence BLOBs live with their marker/grant rows. `BEGIN IMMEDIATE`
  transactions serialize writes. A constraint failure rolls back a prospective
  grant. SQLite's real `max_page_count` limit produces `SQLITE_FULL` during a write;
  restarting a reader finds the committed marker, without a grant.
- A parent kills a writer before or after the transaction that moves evidence
  from the marker to the grant. A newly launched reader checks row counts and
  exact BLOB bytes. These are actual executable process restarts; no exception
  handler stands in for a restart.

## Evidence limits and qualification gates

A successful rename, full flush or killed-process test does **not** establish
power-loss durability. The native filesystem, storage device and SQLite VFS must
honor the applicable synchronization contract. This probe deliberately leaves
production qualification unavailable.

[Apple's fsync documentation](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/fsync.2.html)
distinguishes filesystem synchronization from flushing hardware caches through
`F_FULLFSYNC`. Both calls are exercised; a failure is returned, never weakened to
an accepted no-op. This still needs an APFS/storage-specific persistence argument.

[SQLite's synchronous documentation](https://www.sqlite.org/pragma.html#pragma_synchronous)
and [atomic-commit assumptions](https://www.sqlite.org/atomiccommit.html) describe
its durability dependencies. The probe does not establish persistence of initial
database/WAL creation and their directory entries. It does not yet kill during
initialization, inject native VFS `xSync`/`xWrite` failures, or implement protected
repository revisions, root transitions or authenticated recovery.

[MoveFileExW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)
documents write-through behavior, including copy/delete flushing. That statement
alone does not prove persistence of the complete renamed tree. Windows file
flushing, directory-entry persistence, normal-user ownership/ACL setup and
restart evidence still require native qualification. Administrator volume flushing
is not an acceptable fallback. No Windows qualification claim follows from these
tests passing.

Stop qualification if any required native call fails, a winner is overwritten,
state is silently created/reset, a failed/uncommitted write yields a grant, a
fresh process loses committed evidence, or any target lacks a documented ordinary
user persistence guarantee. Unsupported target setup must fail, not skip or become
a synthetic pass. Additional write/flush fault schedules and concurrent promotion
obligations must be implemented before a complete provider gate is claimed.

Passing these probes is insufficient for artifact-bound qualification. Required
future evidence includes provider source/build identity, exact artifact digest,
runtime, OS/version/architecture, filesystem assumptions, all required native
cases and evidence digests. Such attestations belong outside the immutable built
artifact to avoid a self-referential hash.

## Local development observation

On macOS 26.6.2 build 25G83, aarch64, Rust 1.98.1 and detected local APFS, the
16-test probe passes, including native file/directory `F_FULLFSYNC` and real
process death/restart. Linux and Windows execution results are not supplied by
this local run. The broader PKG-3 provider gate remains incomplete.

The existing three-OS `kit-conformance` CI job retains combined stdout/stderr from
its single package/adapter test run, including these probes and detected native
environment, as `candidate-provider-probes-<runner>`. It also retains `rustc -vV`
and an explicit candidate-only scope statement. Upload runs even after test failure;
pipeline failure is preserved. These development logs are not a qualification
attestation for a released artifact.
