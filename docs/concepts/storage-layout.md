# Storage layout

RustBase keeps one SQLite file per scope and one filesystem directory per scope's files. The whole working directory looks like:

```
data/
  system.db                           # workspaces registry, master admins, master audit
  workspaces/
    <workspace_id>/
      workspace.db                        # apps, workspace/app admins, admin refresh tokens, workspace audit
      storage/                        # workspace-level files (rarely used)
      apps/
        <app_id>/
          data.db                     # collections, records, access rules, users,
                                      # oauth, user refresh tokens, verifications,
                                      # password resets, OTPs, TOTP, MFA challenges,
                                      # file metadata, app audit
          storage/                    # app-level files (binary blobs)
  hooks/
    <workspace_id>/<app_id>/              # JS/TS hook source files (*.js, *.ts)
```

End-users live in each app's `data.db` — the `users` table is per-app along with the OAuth provider config, refresh tokens, and every auxiliary auth table (verifications, password resets, OTPs, TOTP, MFA challenges).

Plus, when [Litestream](/guide/backups) is enabled:

```
data/
  litestream.yml                      # generated; one entry per *.db file
```

## Why one file per scope

- **Independent rotation**: cycling a workspace's `workspace.db` doesn't take other workspaces offline.
- **Trivial backup / restore**: copying `data/workspaces/acme/` migrates the whole workspace.
- **Independent locks**: SQLite is a single-writer DB, but a write to `workspaces/acme/apps/web/data.db` doesn't block a write to `workspaces/bob/workspace.db`.
- **Cleanup**: dropping a workspace = one `DELETE FROM workspaces` row + one `rm -rf` of the workspace's folder.

## Per-connection PRAGMAs

Every pool sets the same PRAGMAs on connection:

```
journal_mode = WAL
foreign_keys = ON
busy_timeout = 5000
synchronous  = NORMAL
```

WAL means concurrent readers don't block the single writer. `busy_timeout` smooths over transient writer contention. `synchronous=NORMAL` is the standard fsync-per-checkpoint tradeoff.

## Pool management

Three pool kinds, each managed independently:

| Pool | Key | Default cap | Notes |
|---|---|---|---|
| System pool | — | always open | one DB, opened at boot |
| Workspace pool | `WorkspaceId` | `workspace_pool_cap = 32` | LRU eviction |
| App pool | `(WorkspaceId, AppId)` | `app_pool_cap = 64` | LRU eviction |

Cold workspaces and apps stay on disk; their pools reopen lazily on next access in ~1 ms. Tune the caps via [configuration](/guide/configuration) if you run many tenants and have spare RAM.

## What the dashboard sees

The dashboard's **Audit** tab queries `audit_log` in whichever scope you're viewing. The **Files** tab walks the matching `storage/` directory's metadata. The **Hooks** tab reads and writes files directly under `data/hooks/<workspace>/<app>/`. Nothing magical — everything's on disk.
