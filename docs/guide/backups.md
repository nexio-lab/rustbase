# Backups (Litestream)

[Litestream](https://litestream.io) is a sidecar that streams WAL changes from SQLite to S3 (or any S3-compatible bucket) **continuously**. RustBase ships first-class integration: turn it on in `rustbase.toml`, point it at a bucket, and your `system.db`, every `workspace.db`, and every app `data.db` are replicated.

## Why Litestream

SQLite snapshots are atomic at file-copy boundaries. Litestream pipes the WAL out as it's written, so:

- **RPO** (recovery point objective) is sub-second.
- **RTO** (recovery time objective) is "download the bucket, restart RustBase."
- Cost is tiny — object storage + a few KB/s of egress.

For a single-node BaaS that lives or dies by its SQLite files, this is the right backup model.

## Turning it on

```toml
[litestream]
enabled                = true
bucket                 = "s3://my-rustbase-backups"
prefix                 = "prod"
replicate_interval_sec = 10
```

When `enabled = true`, RustBase on boot:

1. Walks `data/` for every `*.db` file.
2. Generates `data/litestream.yml` with one entry per DB.
3. Starts Litestream as a child process and forwards its logs into RustBase's tracing output.

You don't run Litestream yourself — RustBase manages it.

## Bucket layout

```
s3://my-rustbase-backups/
  prod/
    system.db/
      generations/<id>/snapshots/...
      generations/<id>/wal/...
    workspaces/acme/workspace.db/
      generations/<id>/...
    workspaces/acme/apps/web/data.db/
      generations/<id>/...
```

The on-disk path becomes the prefix in the bucket. Adding a workspace or app on disk → next replication cycle catches it automatically.

## Credentials

Litestream uses standard AWS-style env vars:

```sh
AWS_ACCESS_KEY_ID=...
AWS_SECRET_ACCESS_KEY=...
AWS_REGION=us-east-1
```

For Cloudflare R2 / Backblaze B2 / MinIO, also set:

```sh
AWS_ENDPOINT_URL_S3=https://<account>.r2.cloudflarestorage.com
```

(See [Litestream's S3 endpoint docs](https://litestream.io/reference/config/#s3-replica).)

## Restoring

1. Provision a fresh host, install the `rustbase` binary, and create the `data/` directory.
2. Install the `litestream` CLI on the host.
3. For each DB you want to restore, run:
   ```sh
   litestream restore \
     -o data/system.db \
     s3://my-rustbase-backups/prod/system.db
   ```
   Do the same for every `workspace.db` and every app `data.db`.
4. Copy the matching `data/workspaces/<workspace>/apps/<app>/storage/` (the file blobs) from wherever you mirrored them. If you used the S3 storage backend, this step is automatic — the blobs already live in S3.
5. `systemctl start rustbase`.

That's the entire DR drill.

## RPO / RTO targets

| Component | Backed up by | RPO | RTO |
|---|---|---|---|
| `system.db` + every `workspace.db` + every `data.db` | Litestream → S3 | `replicate_interval_sec` (default 10s) | Minutes to download + restart |
| File blobs (local backend) | Your own `rsync` / `restic` | Whatever you set | Minutes |
| File blobs (S3 backend) | S3 itself | — | — |
| Hooks (JS/TS source) | Your version control or your `rsync` | Whatever you set | Seconds |

Hooks aren't replicated by Litestream because they're not databases. Treat them like application code — version-control them.

## What about *write* failover?

There isn't one. SQLite is a single-writer database; replicas are passive. If the primary host dies, you restore to a new host. That's the model.

If you need a hot failover with sub-second cutover, RustBase is not the right shape — you want a Postgres cluster.

## Disabling

Set `enabled = false` (or delete the `[litestream]` section) and restart. RustBase shuts the sidecar down and doesn't recreate `data/litestream.yml`. Existing data in the bucket stays put until you clean it up.
