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

## Windows directory write-through candidate

The separate Windows-only `windows_write_through.rs` helper probes a stronger
candidate than the existing `MoveFileExW` test. It is test code, with no production
provider or qualification attestation. It runs under trusted-directory assumptions.

- It opens the existing temporary root and creates each new ancestor and empty
  directory through user-mode
  [NtCreateFile](https://learn.microsoft.com/en-us/windows/win32/api/winternl/nf-winternl-ntcreatefile).
  It requests `FILE_DIRECTORY_FILE | FILE_WRITE_THROUGH |
  FILE_SYNCHRONOUS_IO_NONALERT`. Child names are relative to the parent handle;
  `FILE_CREATE` rejects existing entries. The probe reads `FileModeInformation`
  back and requires write-through and synchronous mode on each handle.
- Directory access is `DELETE | SYNCHRONIZE | FILE_LIST_DIRECTORY |
  FILE_TRAVERSE | FILE_READ_ATTRIBUTES`. The helper never requests backup intent,
  adjusts privileges, uses TxF, opens a volume or flushes a volume. A negative
  test omits `DELETE` and requires promotion to fail without changing either name.
- Payload and empty files use `create_new`, `FILE_FLAG_WRITE_THROUGH`, and
  `sync_all`. This exercises file flushing separately from directory creation.
- Promotion reopens the stage with those same directory options and calls
  [SetFileInformationByHandle](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle)
  with `FileRenameInfo`, NULL `RootDirectory`, an absolute destination under the
  canonical private fixture parent, and
  [`ReplaceIfExists = FALSE`](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_rename_info).
  An empty or populated destination must survive collision unchanged; the entire
  candidate must remain intact. No copy, overwrite or fallback is attempted.
- A child builds a tree containing empty directories at two depths, an empty file
  and payload bytes. The parent kills it after staging or promotion and launches
  a fresh reader to inspect both names and exact contents. Handles have closed
  before the readiness boundary. This tests process restart after completed calls,
  not interruption inside a native operation, OS reboot or power failure.

Each writer records the actual filesystem, requested access/options and
`TokenElevation`. A successful run under an elevated token is not ordinary-user
evidence. Native execution under a standard user remains required, including the
ownership and ACL setup, on both supported Windows architectures. Even a
non-elevated token observation alone is not a complete privilege/ACL audit.

The [CreateFileW caching contract](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew#caching-behavior)
describes NTFS metadata flushing for write-through requests, including rename.
`NtCreateFile` explicitly allows write-through directory creation. These documents
justify testing this candidate, but do not by themselves prove a durable whole-tree
protocol: persistence of newly created empty directories and all ancestor entries,
ordering across handles, and the storage device's flush guarantees still need a
documented argument and native fault evidence. Successful mode readback and API
return values do not fill that gap. The temporary root itself is created by the
test framework, so its initial creation durability is outside this probe.

The initial Windows/x86_64 NTFS run passed creation, handle-mode readback and the
missing-DELETE negative case, but promotion using a non-NULL destination parent
handle returned error 87, including when no destination existed. Increasing the
declared buffer to the complete structure plus filename did not change that result
in [the sizing-only rerun](https://github.com/finos/morphir-rust/actions/runs/35683358190).
The controlled candidate using NULL `RootDirectory` and an absolute destination
passed all 19 Windows provider tests in
[the native comparison run](https://github.com/finos/morphir-rust/actions/runs/35683729042/job/106606133842).
Source handle modes/access, buffer sizing, no-replace behavior and collision
assertions were unchanged. There is no fallback. This NTFS/x86_64 run used an
elevated token; it does not establish standard-user or power-loss qualification.

Microsoft's [2022 documentation correction](https://github.com/MicrosoftDocs/sdk-api/commit/ada04eef90bc7ebe441ce2ef938867d3a677d57d)
warned about non-NULL `RootDirectory` behavior, while the current API reference
permits it. The measured comparison identifies the relative-root call form as the
functional failure, without establishing a durability guarantee. Cross-compilation checks bindings and Rust
types only; macOS runs do not execute the Windows-only cases.

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

### Separate standard-user Windows execution

The `provider-windows-standard-user` CI job builds this same test executable for
`windows-2025` x64 and `windows-11-arm` ARM64. Its ephemeral setup creates a fresh
local account belonging only to Users, a private scratch directory owned by that
account, and an executable directory the account can read/execute but cannot write.
Setup reads back ownership and the protected scratch DACL. The only allowed ACL
principals are the test account, SYSTEM and the setup account. Tests create their
own temporary directories underneath that scratch directory.

The job uses a real credentialed local logon through
[Start-Process](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.management/start-process),
not a restricted administrator token or network-only credentials. Provisioning and
cleanup run as CI setup; provider operations run as the new account. The password
stays in setup memory and is never logged or written to an artifact. A `finally`
block removes the profile, account and sandbox and disposes the secure password;
cleanup failures fail the job. Runner cancellation also destroys the ephemeral VM.

`ordinary_user_identity_and_state_acl_are_observed` is ignored in a normal run
because it requires this setup. The dedicated job runs **all** probes with
`--include-ignored`, `MORPHIR_PROVIDER_STANDARD_USER=1`, and expected user/setup
SIDs and architecture. Missing setup is an error. Every fixture creation and each
real child-process probe checks the actual token and fixture ACL. The dedicated
case also inspects the live SQLite database, WAL and SHM file owners/ACLs. It
reads back the current executable and its containing directory: both must be owned
by the setup account and grant the test user exactly read/execute access. CI
requires the named case to report success and an unfiltered, zero-ignored passing
test summary.

Native observations use `GetTokenInformation`, `IsTokenRestricted`,
`IsWow64Process2` and `GetNamedSecurityInfoW`. They require the exact expected user
SID, no elevation or linked/restricted token, no Administrators group SID even if
deny-only, and no enabled privilege outside the ordinary-user allowlist. They
reject emulation and check both executable and native machine architecture.
Fixture owners must match the user, the user must have FullControl, and every ACL
entry must name an allowed principal. Null DACLs, unexpected ACE forms, missing
rights and other principals fail. These checks do not trust environment usernames
or runner labels as identity evidence.

Artifacts contain setup SIDs/ACLs, actual token groups and privilege attributes,
native architecture, filesystem detection, executable SHA-256 and complete test
output. The digest verifies the copied test executable, not a released provider.
Native standard-user results remain pending until these jobs execute successfully.
This is functional permissions evidence, with no new power-loss, initialization
durability or production qualification claim.

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
