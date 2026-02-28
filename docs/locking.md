# Locking

Refreshmint uses advisory filesystem locks to coordinate GUI and CLI work.

There are two authoritative locks:

- `logins/<login>/.lock`
- `.gl.lock`

`logins/<login>/.lock` protects login-scoped work such as scrape runs, login config edits, profile clearing, and login-account extraction.

`.gl.lock` protects ledger-wide GL mutations such as posting, unposting, syncing, recategorizing, and merging GL transactions.

## Metadata Files

Each real lock has a metadata sibling that is only for observability:

- `logins/<login>/.lock.meta.json`
- `.gl.lock.meta.json`

These files include:

- `owner`
- `purpose`
- `startedAt`
- `pid`
- the locked resource

The metadata file is not the lock. The real lock is the OS-level advisory file lock held on `.lock` / `.gl.lock`.

## Why The UI Watches Metadata

Filesystem watchers can observe file create/remove/modify events, but they cannot directly observe `try_lock_exclusive()` state. The lock lives in kernel-managed file-lock state, not in the file contents.

That is why the UI watches metadata files for responsiveness, then re-checks the real lock state through a backend status command.

## Lifecycle

On acquire:

1. acquire the real lock
2. remove any stale metadata for that same lock
3. write fresh metadata

On release:

1. remove metadata while still holding the real lock
2. release the real lock

Removing metadata after unlocking is unsafe because another process could acquire the real lock and write fresh metadata before the old process finishes cleaning up.

## Stale Metadata

Crashes can leave stale `.lock.meta.json` files behind.

The backend cleans stale metadata only after it proves the real lock is currently free by successfully acquiring it. Metadata must never be trusted by itself.

## Lock Ordering

When an operation needs both GL and login locks:

1. acquire `.gl.lock` first
2. acquire distinct login locks in sorted `login_name` order

This avoids deadlocks for transfer/sync/unpost flows that span multiple login journals.

## Operator Expectations

- If the GUI or CLI says a login or the general journal is “currently in use,” another operation is holding the authoritative lock.
- If metadata exists but the backend reports the lock is free, the metadata was stale and will be cleaned up.
- `Extract All` is login-scoped and can still run for unlocked accounts while other accounts are busy.
- `Post All` depends on the GL lock and may also need one or more login locks depending on the entries being posted.
