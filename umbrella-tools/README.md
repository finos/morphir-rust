# umbrella-tools

The Rust development tools of [finos/morphir](https://github.com/finos/morphir), the Morphir umbrella
repository. They moved here from the root of finos/morphir (commit `f0753737`) so that the umbrella repository
holds no Rust workspace.

| Crate | Program | Purpose |
| --- | --- | --- |
| `crates/morphir-mck` | (library) | Morphir Compatibility Kit engine: kit parsing, loading, hashing, adapter transport, reports. |
| `crates/morphir-mck-cli` | `morphir-mck` | The standalone command line of the shared MCK runner. Development only. |
| `crates/morphir-dev-tools` | `morphir-dev` | Internal tool of finos/morphir: the `kb` commands and the repository contract gates. |

`third_party/rust-tuf` is the test-only TUF crate these crates verify signed fixtures with. See
`third_party/rust-tuf/MORPHIR-VENDOR.md`.

None of these crates is released or published, and none is the Morphir CLI. The Morphir CLI is
[finos/morphir-dotnet](https://github.com/finos/morphir-dotnet).

## This workspace builds only inside finos/morphir

The crates read the specifications, compatibility kits and fixtures of finos/morphir (`spec/`, `docs/spec/`,
`website/static/`) by relative path. `morphir-mck` embeds the IR kit at build time. The paths assume that this
repository is checked out as the `ecosystem/morphir-rust` submodule of finos/morphir. A checkout of morphir-rust
alone cannot build this workspace: the build stops with a message that names the missing kit directory.

For that reason this directory is a Cargo workspace of its own, with its own `Cargo.lock`. It is not a member of
the morphir-rust workspace one level up, and `cargo build`, `cargo test` and the CI of morphir-rust do not build
it. It depends on crates of that workspace by path (`../crates/*`); those crates inherit their dependency versions
from `../Cargo.toml`. Keep an exact pin that both workspaces declare, such as `granit-parser`, equal in both.

## Build and test

Run the commands from the root of a finos/morphir checkout with its submodules initialized:

```bash
cargo test --locked --manifest-path ecosystem/morphir-rust/umbrella-tools/Cargo.toml --workspace
cargo run --locked --manifest-path ecosystem/morphir-rust/umbrella-tools/Cargo.toml -p morphir-mck-cli -- check spec/ir/mck
```

The mise tasks of finos/morphir wrap these: `mise run test`, `mise run lint`, `mise run mck -- <command>`,
`mise run kb -- <command>`, `mise run mck:check`.

## Change procedure

1. Change the code here, in a branch of finos/morphir-rust. Test it from a finos/morphir checkout whose
   `ecosystem/morphir-rust` submodule is on that branch.
2. Open the pull request in finos/morphir-rust. Its CI does not build this workspace, so give the test evidence
   from step 1 in the pull request.
3. After the merge, move the `ecosystem/morphir-rust` pin in finos/morphir. The CI of finos/morphir builds and
   tests this workspace against its specifications.

A change that needs new kit or fixture files and new code needs a pull request in each repository.
