# Storage layout

RustBaas keeps one SQLite file per scope and one filesystem directory per scope's files. The whole working directory looks like:

```
data/
  system.db                           # realms registry, master admins, master audit
  realms/
    <realm_id>/
      realm.db                        # users, oauth, settings, refresh tokens, realm audit
      storage/                        # realm-level files (rarely used)
      apps/
        <app_id>/
          data.db                     # collections, records, access rules, app audit
          storage/                    # app-level files
  hooks/
    <realm_id>/<app_id>/              # JS/TS hook source files (*.js, *.ts)
```

Plus, when [Litestream](/guide/backups) is enabled:

```
data/
  litestream.yml                      # generated; one entry per *.db file
```

## Why one file per scope

- **Independent rotation**: cycling a realm's `realm.db` doesn't take other realms offline.
- **Trivial backup / restore**: copying `data/realms/acme/` migrates the whole realm.
- **Independent locks**: SQLite is a single-writer DB, but a write to `realms/acme/apps/web/data.db` doesn't block a write to `realms/bob/realm.db`.
- **Cleanup**: dropping a realm = one `DELETE FROM realms` row + one `rm -rf` of the realm's folder.

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
| Realm pool | `RealmId` | `realm_pool_cap = 32` | LRU eviction |
| App pool | `(RealmId, AppId)` | `app_pool_cap = 64` | LRU eviction |

Cold realms and apps stay on disk; their pools reopen lazily on next access in ~1 ms. Tune the caps via [configuration](/guide/configuration) if you run many tenants and have spare RAM.

## What the dashboard sees

The dashboard's **Audit** tab queries `audit_log` in whichever scope you're viewing. The **Files** tab walks the matching `storage/` directory's metadata. The **Hooks** tab reads and writes files directly under `data/hooks/<realm>/<app>/`. Nothing magical — everything's on disk.
