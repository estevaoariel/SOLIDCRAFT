# Cloud saves: browser storage and platform dependency audit

Investigated 2026-10-22. This records dependency choices and isolated probes; it does
not claim that cloud saves are integrated or that browser recovery tests have passed.

## Recommended reuse

- Use **`indexed_db_futures` 0.6.4** for browser connection metadata, pending-save
  records, and acknowledgements. Its transaction completion API and durability
  options remove the need to write another IndexedDB callback wrapper.
- Reuse SolidCraft's **`web-sys` OPFS support** behind one focused storage adapter,
  or use **`opfs` 0.3.1** if its cross-platform interface materially replaces code.
  Do not introduce both native persistence implementations.
- Keep **`keyring`**, **`sha2`**, **`serde`**, **`serde_json`**, and **`zstd`** already
  used by the application. Native atomic replacement should reuse
  `solidcraft_io::safe_write`.
- For new Ed25519 keys, **`ssh-key` 0.6.7** can produce encrypted OpenSSH keys
  without exposing a passphrase in subprocess arguments. Existing keys and normal
  SSH agent/configuration should still be handled by system Git/OpenSSH.
- Use **`std::fs::File::try_lock`** for a native application lock. The repository's
  Rust 1.95 minimum already includes it; another locking crate is unnecessary.

## IndexedDB candidates

| Crate | Verified stable release and license | Relevant interface | Assessment |
| --- | --- | --- | --- |
| `indexed_db_futures` | 0.6.4, MIT | Version-change callback; object stores and indices; serde; `TransactionOptions` with strict durability; `commit().await` waits for completion | Preferred for the outbox. Use `serde` and `indices` features as needed. |
| `idb` | 0.6.5, MIT/Apache-2.0 | Version upgrades and indices; requests and transactions implement futures; `transaction.commit()?.await` | Viable lower-level alternative. The returned `TransactionResult` must be checked for `Committed`; `Ok(Aborted)` is possible. |
| `rexie` | 0.6.2, MIT/Apache-2.0 | Declarative database/store/index builder on `idb`; `transaction.commit().await` or `done().await` | Viable simpler schema builder. `commit` checks the result; `done` returns the result for the caller to check. |

Version/license metadata came directly from the crates.io registry:
[`indexed_db_futures`](https://crates.io/api/v1/crates/indexed_db_futures),
[`idb`](https://crates.io/api/v1/crates/idb),
[`rexie`](https://crates.io/api/v1/crates/rexie).
Interfaces were checked against published documentation and the corresponding
permissively licensed crate sources:
[`indexed_db_futures`](https://docs.rs/indexed_db_futures/0.6.4/indexed_db_futures/),
[`TransactionOptions`](https://docs.rs/indexed_db_futures/0.6.4/indexed_db_futures/transaction/struct.TransactionOptions.html),
[`idb::Transaction`](https://docs.rs/idb/0.6.5/idb/struct.Transaction.html),
[`rexie::Transaction`](https://docs.rs/rexie/0.6.2/rexie/struct.Transaction.html).

`indexed_db_futures` aborts an uncommitted transaction on drop. Its published
transaction documentation also records a Safari caveat around rollback after
awaited requests. Keep transactions short, prepare values first, submit related
requests together, and avoid depending on drop as a crash-recovery mechanism.
Await the explicit commit, and test transaction failure on the supported browsers.
[Published transaction implementation](https://docs.rs/crate/indexed_db_futures/0.6.4/source/src/transaction.rs)

Use strict durability for irreplaceable queue records where supported. Strict is
a browser durability request, not a replacement for checking commit completion or
handling quota errors. Browser support must be exercised at runtime.
[IndexedDB transaction durability](https://developer.mozilla.org/en-US/docs/Web/API/IDBDatabase/transaction)

An isolated WASM probe compiled this API successfully:

```rust
use indexed_db_futures::{
    database::Database,
    prelude::*,
    transaction::{TransactionDurability, TransactionMode, TransactionOptions},
};

async fn persist_record(db: &Database) -> indexed_db_futures::Result<()> {
    let tx = db
        .transaction("pending")
        .with_mode(TransactionMode::Readwrite)
        .with_options(
            TransactionOptions::new().with_durability(TransactionDurability::Strict),
        )
        .build()?;
    tx.object_store("pending")?
        .put("captured record")
        .with_key("save-id")
        .await?;
    tx.commit().await?;
    Ok(())
}
```

## OPFS and the local save commit point

`opfs` 0.3.1 is MIT licensed. It offers directory/file handles, ranged reads,
asynchronous writes and close, plus native and in-memory implementations. Its web
implementation delegates to browser APIs; it does not provide an atomic
transaction spanning OPFS and IndexedDB.
[Registry metadata](https://crates.io/api/v1/crates/opfs),
[API](https://docs.rs/opfs/0.3.1/opfs/),
[published web implementation](https://docs.rs/crate/opfs/0.3.1/source/src/web.rs)

SolidCraft already awaits OPFS write and close in
`apps/solidcraft-web/src/files.rs`. Extract/reuse that mechanism for immutable
chunks instead of maintaining duplicate file bindings. Keep cloud directories
separate from the existing design-file enumeration.

The recommended save sequence is:

1. Validate and capture the document snapshot and immutable chunk hashes.
2. Write missing OPFS chunks and the recovery document; await every stream close.
3. Open a short IndexedDB transaction, write the pending record and its local-save
   acknowledgement, then await commit.
4. Report **Saved locally**. Network synchronization can now start or resume later.
5. Remove/acknowledge the pending record only after confirmed remote publication.

This ordering leaves harmless unreferenced chunks if the browser stops between
steps 2 and 3. It avoids making an acknowledged queue entry depend on incomplete
file writes. Do not await OPFS or network operations inside an IndexedDB
transaction. File changes become visible after the writable stream closes.
[Browser writable-file behavior](https://developer.mozilla.org/en-US/docs/Web/API/FileSystemFileHandle/createWritable)

The `opfs` implementation makes an owned `Uint8Array` and Blob/File before calling
`write_with_blob`, with an upstream comment describing a Safari issue with direct
WASM-memory `write_with_u8_array`. That issue was **not reproduced here**. It is a
reason to use owned browser bytes for the cloud adapter and test Safari explicitly,
not evidence that the current SolidCraft writer fails.
[Published implementation](https://docs.rs/crate/opfs/0.3.1/source/src/web.rs)

Request persistent browser storage and retain the returned boolean: the browser
may deny the request. Surface denied persistence, quota exhaustion, and failed
close/commit; a failed local write must not be shown as saved.
[StorageManager.persist](https://developer.mozilla.org/en-US/docs/Web/API/StorageManager/persist)

## Native credentials, SSH keys, and locks

The existing desktop dependency is `keyring` 3.6.3, already used through the app's
credential service. Reuse that service with opaque connection-specific identifiers;
do not put tokens or passphrases into shared project metadata. The current stable
registry release is 4.2.0; this feature does not require an unrelated major upgrade.
[Registry metadata](https://crates.io/api/v1/crates/keyring),
[3.6.3 API](https://docs.rs/keyring/3.6.3/keyring/)

`ssh-key` 0.6.7 is Apache-2.0/MIT. A minimal native dependency can disable defaults
and enable `std`, `ed25519`, `encryption`, and `getrandom`. It generates Ed25519
keys, emits OpenSSH encoding, encrypts/decrypts with a passphrase, and zeroizes
private-key material. The published stable API uses
`ssh_key::rand_core::OsRng`; newer upstream documentation may show a different RNG
API. Encrypted keys still require OpenSSH agent/askpass handling when Git uses
them; generating a key does not implement SSH transport.
[Stable API and features](https://docs.rs/ssh-key/0.6.7/ssh_key/),
[published feature definitions](https://docs.rs/crate/ssh-key/0.6.7/source/Cargo.toml)

Invoking installed `ssh-keygen` is also reuse, and already matches the approved
native Git architecture. Rust key generation avoids the extra executable and
passphrase-argument problem. Whichever adapter is chosen, create private files
with exclusive creation, restrictive permissions and no overwrite, and keep
private keys out of repositories.
[OpenSSH key-generation interface](https://man.openbsd.org/ssh-keygen)

`fs2` 0.4.3 and `fs4` 1.1.0 are MIT/Apache-2.0 options. `fs4` is a maintained fork
with synchronous and runtime-specific asynchronous interfaces. The standard
library provides exclusive/shared locks, nonblocking acquisition, and unlock
since Rust 1.89, so use it here. Application locks coordinate SolidCraft instances;
they do not replace Git's own locks or protect against arbitrary external tools.
[fs2 metadata](https://crates.io/api/v1/crates/fs2),
[fs4 API](https://docs.rs/fs4/1.1.0/fs4/),
[standard File locking](https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock)

## Existing encoding dependencies and retries

The workspace already has `sha2` 0.10.9, `serde` 1.0.229, `serde_json`, `tempfile`
3.27.0, and Zstd dependencies. Reuse those implementations for addressing,
metadata, compression and native temporary-file needs. Keep bounded decoding in
the cloud format layer; a compression crate cannot decide the application's
allowed decoded sizes or compatibility policy. `tempfile` does not replace the
existing atomic writer's flush/replace protocol.
[SHA-2 API](https://docs.rs/sha2/0.10.9/sha2/),
[Serde API](https://docs.rs/serde/1.0.229/serde/),
[Zstd API](https://docs.rs/zstd/0.13.3/zstd/),
[tempfile API](https://docs.rs/tempfile/3.27.0/tempfile/)

Do not introduce a runtime solely for cancellation or retry timers. Reuse the
application's asynchronous job mechanism and platform task executor. Retry state
belongs in the durable outbox: a general retry helper cannot know whether a
remote commit succeeded before the connection broke. Cancellation must leave the
snapshot recoverable and reconcile uncertain publication before retrying.

## Verification performed

- `rustc 1.99.1`; isolated manifests and targets under `/tmp`, without editing the
  workspace manifest or lockfile.
- `cargo check --target wasm32-unknown-unknown` passed for all four browser
  candidates, including IndexedDB schema creation, an index, strict durability,
  explicit commit, and OPFS write/close calls.
- Native `ssh-key` 0.6.7 probe generated and encrypted an Ed25519 key, serialized
  and parsed its OpenSSH private-key format, recovered the same public key, and
  rejected an incorrect passphrase.
- A native standard-library lock probe verified contention between two separately
  opened handles and successful acquisition after unlock on Linux.
- An incremental compiler failure occurred while extending the WASM probe;
  the complete probe passed with `CARGO_INCREMENTAL=0`.

These are compile and native-library checks. Browser execution, storage denial,
quota failure, tab concurrency, browser restart, Safari byte fidelity, physical
crash durability, Windows/macOS key permissions, and actual SSH-agent pushes
remain integration tests. No cloud account or credentials were used.
