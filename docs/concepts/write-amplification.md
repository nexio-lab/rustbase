# Write amplification

Every API call that mutates state on RustBase eventually becomes one or
more `INSERT` / `UPDATE` statements against one of the SQLite files in
`data/`. Under WAL with `synchronous = NORMAL` (the per-connection
default), **each commit costs one `fsync`** of the WAL — so the cap on
sustained write throughput on a given app's `data.db` is the disk's
fsync rate, not its raw IOPS.

This page documents how many commits each common path costs after
[`feat(perf): coalesce auth-path writes`](https://github.com/pjonaszik/rustbase/commit/HEAD)
(the May 2026 batching pass) and where the future wins lie.

## Per-pool fsync rate

A commodity NVMe SSD with `synchronous = NORMAL` can sustain
~1 000–3 000 commits/s per file before it starts queuing. A spinning
disk drops to ~80–200/s. SQLite serialises writers per file, so:

- **Cross-app traffic scales** — each app's `data.db` has its own
  writer queue.
- **Single-app sustained writes** hit the per-file ceiling well before
  Postgres-class numbers.

For the [positioning](positioning) RustBase ships against — many small
apps inside one workspace — this is the correct shape. A single
`data.db` sustaining > 100 RPS of writes is a sign you've outgrown the
tool.

## Hot-path commit count (post-batching)

| Path | Commits | Notes |
|---|---|---|
| `POST /…/auth/users/login` (password OK, no TOTP) | **1** | `commit_user_login` batches `users.last_login` + `_refresh_tokens` INSERT into one txn. |
| `POST /…/auth/users/login/totp` (TOTP OK) | **1** | Same batching; the MFA challenge consumed earlier counts as its own commit (see below). |
| `POST /…/auth/otp/login` (success, returning user) | **1** | `commit_user_login` + already-completed verification. |
| `POST /…/auth/otp/login` (success, *new* user) | **3** | Insert user (1) + mark verified (1) + `commit_user_login` (1). |
| `POST /_/auth/refresh`, `…/workspace`, `…/users/refresh` | **1** | `rotate_refresh_token` does the revoke + insert in one txn. |
| `POST /…/auth/oauth/:provider/callback` (returning user) | **1** | `commit_user_login`. Provider state consumed by the matching `/authorize`. |
| Record CRUD (`POST/PATCH/DELETE /…/records`) | **2** | Underlying row write (1) + `audit_log` append (1). |

The audit append is *not* in the same txn as the row write — the audit
log is the after-the-fact paper trail, not a constraint on the
underlying op. Doing the audit append asynchronously (via a per-pool
mpsc channel + a background writer that coalesces N events) would drop
the record-write cost to **1** commit. That's the next planned win;
file an issue if you hit a workload where it matters.

## What was *not* batched

The following multi-step paths still issue one commit per step. They
fire infrequently enough that the txn refactor isn't justified yet:

- Email-verification *confirm* — token consume + `users.verified = 1`.
- Password-reset *confirm* — token consume + `users.password_hash` UPDATE
  + invalidate sibling reset tokens.
- TOTP enrolment — `_user_totp` upsert (1 commit, fine as-is).
- OAuth state consume — single UPDATE per flow.

Each of these is at most ~3 commits per user-initiated action. If your
load profile makes any one of them hot, the batching pattern is
mechanical: wrap the steps in a `pool.begin()` / `tx.commit()` and
push the helper into `rustbase_db`.

## SQLite pragmas

Set on every connection by `rustbase_db::pool`:

```sql
PRAGMA journal_mode  = WAL;       -- one writer + many readers
PRAGMA synchronous   = NORMAL;    -- fsync on commit, not every page write
PRAGMA busy_timeout  = 5000;      -- 5 s of busy-wait on writer contention
PRAGMA foreign_keys  = ON;        -- referential integrity end-user data depends on
```

`synchronous = FULL` would halve the throughput; `synchronous = OFF`
risks corruption on power loss. `NORMAL` is the WAL-recommended
balance — the [SQLite docs](https://sqlite.org/pragma.html#pragma_synchronous)
confirm WAL is durable under crashes with `NORMAL` as long as the
filesystem flushes the WAL on `fsync`.

## When to scale out

You've outgrown a single RustBase instance when:

- Any one app's `data.db` sustains > 100 writes/s for hours.
- The audit log on a single workspace's `workspace.db` is the hottest
  file in the install.
- Litestream replication lag (`replicate_interval_sec`) is no longer
  acceptable as an RPO.

[Positioning](positioning) covers the migration path.
