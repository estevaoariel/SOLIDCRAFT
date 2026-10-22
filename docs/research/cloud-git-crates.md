# Rust Git dependency audit for cloud saves

Investigated 2026-10-22. This is a dependency decision, not a claim that cloud
synchronization is implemented. Versions below were checked against the public
crates.io API; current documentation was also queried through Context7.

## Recommendation

Use standard Git as the native repository engine, behind a small process adapter.
That reuses the implementation responsible for transport, worktrees, credentials,
SSH agents, hooks and signing. Keep snapshot storage and the durable outbox
independent of that adapter. The browser can implement the same publication
contract through a GitHub API client. Do not implement a Git wire protocol,
checkout engine, or embedded SSH client for this feature.

None of the examined Rust Git wrappers removes the required process handling
without introducing a material limitation. Prefer the standard library for the
small native invocation boundary and use the existing `shlex` crate for the
shell-quoted SSH override. Reconsider a larger wrapper when it satisfies the
binary-override and bounded-execution requirements without a fork.

This is an engineering recommendation based on the capability checks below.
Git's documented environment and configuration define the compatibility target:
[`GIT_SSH_COMMAND`](https://git-scm.com/docs/git#Documentation/git.txt-GIT_SSH_COMMAND),
[credential helpers](https://git-scm.com/docs/gitcredentials), and
[commit signing](https://git-scm.com/docs/git-commit).

## Compared crates

| Crate | Current checked release | License | Result for SolidCraft |
| --- | --- | --- | --- |
| `gix` | 0.89.0, 2026-10-08 | MIT OR Apache-2.0 | Maintained, substantial pure Rust Git implementation; no push operation and no complete browser repository backend. |
| `gix-command` | 0.12.0, 2026-10-08 | MIT OR Apache-2.0 | Native process preparation with Git conventions; no bounded capture, cancellation or timeout executor. |
| `git-command` | 0.2.4, 2023-02-17 | MIT/Apache-2.0 | Obsolete name; package explicitly directs users to `gix-command`. |
| `git_cmd` | 0.9.1, 2026-10-19 | MIT OR Apache-2.0 | Maintained release-plz CLI wrapper, but executable is fixed and per-process environment customization is private. |
| `git-spawn` | 0.4.0, 2026-10-03 | MIT OR Apache-2.0 | Useful bounded async subprocess engine; requires Tokio and hardcodes the Git executable. |
| `gitwrap` | 0.11.0, 2024-12-28 | Apache-2.0 | CLI builders; project describes incomplete command/option coverage. No advantage established over the narrower invocation boundary. |
| `duct` | 1.1.2, 2026-09-03 | MIT | Good general process composition; bounded capture still needs caller machinery. |
| `git2` | 0.21.0, 2026-05-18 | MIT OR Apache-2.0 for the wrapper | Excluded: links libgit2, whose own license conflicts with repository policy. |
| `git-wrapper` | 0.6.1, 2022-10-09 | LGPL-3.0-or-later | Excluded by repository policy; no source inspected. |

Release and package-license evidence:
[gix](https://crates.io/api/v1/crates/gix),
[gix-command](https://crates.io/api/v1/crates/gix-command),
[git-command](https://crates.io/api/v1/crates/git-command),
[git_cmd](https://crates.io/api/v1/crates/git_cmd),
[git-spawn](https://crates.io/api/v1/crates/git-spawn),
[gitwrap](https://crates.io/api/v1/crates/gitwrap),
[duct](https://crates.io/api/v1/crates/duct),
[git2](https://crates.io/api/v1/crates/git2),
[git-wrapper](https://crates.io/api/v1/crates/git-wrapper).
Release recency establishes activity, not a security or maturity guarantee.

## gix: useful plumbing, insufficient as the synchronization engine

Upstream lists clone and fetch as available, push as unfinished, and commit hooks
as unfinished. Worktree checkout primitives and linked worktrees exist, but full
switch/reset and commit-level merge workflows remain incomplete. The public
remote connection offers `prepare_fetch`; the presence of a `push` namespace does
not establish a publication implementation.
[Feature checklist](https://github.com/GitoxideLabs/gitoxide/blob/main/README.md),
[crate status](https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md),
[remote connection API](https://docs.rs/gix/0.89.0/gix/remote/struct.Connection.html).

Consequently, adopting gix now would still require system Git for publishing and
several compatibility-sensitive operations. Using two repository engines adds
maintenance burden without satisfying the browser requirement. That assessment
does not rule out later reuse of individual object/pack/config crates.

### Measured browser-target support

Two isolated probes were compiled outside the workspace, with separate manifests,
lockfiles and target directory under `/tmp/solidcraft-git-crate-audit`. Neither
probe changes SolidCraft's dependency graph or lockfile.

```toml
# Probe A
gix = { version = "=0.89.0", default-features = false, features = ["sha1"] }

# Probe B, in a separate manifest
gix-pack = { version = "=0.76.0", default-features = false, features = ["sha1", "wasm"] }
```

Both used `cargo check --target wasm32-unknown-unknown`:

- **Probe A failed** in `gix-sec 0.16.0`: unavailable `std::os::unix`,
  `libc::geteuid`, and `Metadata::uid` on this target. Its application code merely
  references `gix::Repository`; networking and worktree access were not attempted.
- **Probe B passed**. It references `gix_pack::data::header::SIZE`.

These are compilation results, not runtime tests. A successful pack-crate build
does not establish IndexedDB/OPFS integration, browser HTTP transport, SSH or push.
The upstream WASM workflow likewise checks selected plumbing crates, including
`gix-pack`, rather than the complete `gix` repository crate.
[Upstream WASM CI](https://github.com/GitoxideLabs/gitoxide/blob/main/.github/workflows/ci.yml),
[gix-pack release](https://crates.io/api/v1/crates/gix-pack).

## Native wrappers: exact integration costs

### git-spawn

Version 0.4.0 provides separate byte limits for stdout/stderr, byte stdin,
child-only environment changes, cancellation, deadlines and child-process
cleanup. Those are meaningful reusable capabilities. Its implementation uses
Tokio process, time and synchronization APIs; it does not provide a blocking
facade. The private command builder constructs `TokioCommand::new("git")`, and
the public executor has no executable override. Adding it would therefore need
a Tokio runtime and an upstream enhancement or fork for a required preference.
[Executor source](https://docs.rs/crate/git-spawn/0.4.0/source/src/command.rs),
[execution source](https://docs.rs/crate/git-spawn/0.4.0/source/src/execution.rs),
[dependency features](https://docs.rs/crate/git-spawn/0.4.0/source/Cargo.toml).

### git_cmd

The current implementation hardcodes `Command::new("git")`, trims every argument,
and keeps environment-enabled execution private. Raw stdin is public through
`Repo::git_output_with_input`, but its existing process helper does not expose
bounded capture, timeout or cancellation. Trimming arguments is also unsuitable
for preserving arbitrary path spelling. Its release-management helpers such as
adding all changes should not be used for a design-specific save.
[Published source](https://docs.rs/crate/git_cmd/0.9.1/source/src/lib.rs).

### gix-command and duct

`gix-command` prepares a standard `Command`, including executable lookup,
Git-specific context, optional shell behavior and environment. Its returned
child still needs capture, cancellation and lifetime handling. `duct` handles
stdin writing and captured-output reading on threads, but the capture API does
not expose a byte limit; streaming readers or external files would still need
caller-managed bounds and cleanup. Neither presently eliminates the feature's
small process boundary.
[gix-command source](https://docs.rs/crate/gix-command/0.12.0/source/src/lib.rs),
[gix-command preparation](https://docs.rs/crate/gix-command/0.12.0/source/src/prepare.rs),
[duct API](https://docs.rs/duct/1.1.2/duct/struct.Expression.html).

## Licensing exclusion

The MIT/Apache metadata on `git2` and `libgit2-sys` describes the Rust wrappers;
it does not make libgit2 itself MIT/Apache. The libgit2 project states that it is
GPLv2 with a linking exception. SolidCraft's repository instructions forbid
linking GPL/LGPL/AGPL code even when another project's linking exception would
otherwise permit distribution. Only metadata and the project's licensing
statement were inspected; libgit2 and `git-wrapper` source was not read,
downloaded or compiled.
[libgit2 licensing statement](https://libgit2.org/),
[libgit2-sys metadata](https://crates.io/api/v1/crates/libgit2-sys).

## Reuse shell quoting

`shlex 2.0.1` is MIT OR Apache-2.0. Its fallible `try_join`/`try_quote` APIs handle
POSIX shell argument quoting and reject NUL. Use that for `GIT_SSH_COMMAND`
instead of maintaining a custom quoting function. Keep normal Git invocations
as argument vectors, with no shell. POSIX quoting applies to Git's documented
shell-interpreted SSH override; it is not a general Windows `cmd.exe` or
PowerShell quoting API. Validate inputs and test actual Git for Windows before
claiming cross-platform special-character coverage.
[shlex release](https://crates.io/api/v1/crates/shlex),
[try_join](https://docs.rs/shlex/2.0.1/shlex/fn.try_join.html),
[quoting warning](https://github.com/comex/rust-shlex/blob/master/src/quoting_warning.md),
[Git SSH command semantics](https://git-scm.com/docs/git#Documentation/git.txt-GIT_SSH_COMMAND).

## Native subprocess lifetime

Use [`process-wrap` 10.0.1](https://docs.rs/process-wrap/10.0.1/) with the
`std`, `process-group`, and `job-object` features. Its Apache-2.0/MIT license
fits the repository rules. It wraps the configured Git executable in a Unix
process group or Windows Job Object, so cancellation can stop SSH, hooks, and
credential helpers together with Git. The predecessor
[`command-group`](https://github.com/watchexec/command-group) is deprecated.

The adapter still owns bounded concurrent pipe reads and its deadline; the
library's unbounded output collection is unsuitable for hostile repository
output. Linux probes passed for simultaneous stdin/stdout/stderr, oversized
stdout and stderr, cancellation with descendants holding pipes, and a deadline
with descendants holding pipes. Windows behavior follows the crate's public
Job Object API and still needs validation on Windows.

Standard-library `File::try_lock` serializes publication in an existing checkout
across application processes. The lock lives inside Git's own per-worktree
metadata and changes no shared repository files or global configuration.

## Required adapter contract

The reusable interface should express repository inspection and publication of a
captured revision, rather than expose a giant Git command API. Native Git and a
browser host API must independently provide the following application semantics:

- Inspect the destination and format before writing; preserve unrelated work.
- Publish the captured snapshot with an expected prior revision and no force.
- Keep local pending data until remote confirmation; identify an already
  published retry without duplicating its revision.
- Return conflicts, authentication failures, cancellation and disconnected state
  as typed outcomes, without silently changing configuration.
- Scope each operation to its repository connection and project folder; do not
  couple multiple configured repositories through global process state.

This contract remains SolidCraft-specific. A Git crate does not decide document
compatibility, durable-save acknowledgement or design conflict-copy policy.
