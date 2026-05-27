# Mental model

Everything in RustBaas fits into a three-level hierarchy:

```
System
  └── Realm  (identity boundary — users live here)
        └── App  (data product — collections + records + files live here)
```

Understanding which layer owns which thing is the single most useful thing you can learn about the system.

## System

The system is the server itself. It tracks:

- The **registry** of realms.
- The **master admin(s)** — accounts that can administer the entire server.
- **Master-scope policy bounds** (see [hierarchical policies](/concepts/hierarchical-policies)).
- The **master audit log**.

There is exactly one `system.db` per RustBaas instance.

## The master realm

On first boot, RustBaas creates a single privileged realm called **the master realm**. Its rules:

- Cannot be deleted (its name and slug can be renamed by master admins).
- Owns the master admin(s).
- Is the only place from which other realms can be created, edited, or deleted.

The master realm is special, but it's still a realm — it has its own `realm.db`, its own users (if you choose to use them), and its own admins.

## Realms

A realm is an **identity boundary**. It holds:

- The **user pool**.
- **OAuth provider** configuration (Google / GitHub / etc.).
- Realm-level **branding** and settings.
- Realm-scope **policy values** (bounded by master).
- The realm's **audit log**.

Users authenticate against a realm. A successful login produces a token bound to `(realm_id, user_id)` — that token can be used by any app in the realm. Apps still enforce per-collection access rules, but SSO across apps in the same realm is automatic.

A realm has its own **realm admins**, scoped to that one realm.

## Apps

An app is what a developer ships against. It owns:

- **Collections** (the schema's tables).
- **Records** in those collections.
- Per-collection **access rules**.
- **Files** uploaded for this app.
- **JS/TS hooks** loaded for this app.
- App-scope **policy values** (bounded by realm and master).
- The app's **audit log**.

Apps inside the same realm share the realm's user pool. Build a mobile app and a website against the same realm and your users can sign in to both without re-registering.

An app has its own **app admins** — a subset of realm admins, plus single-app-scoped admins.

## Collections

Each collection inside an app has a `kind`:

| Kind | Use |
|---|---|
| `base` | Plain records of a fixed shape. |
| `auth` | End-users. Automatically includes `email`, `password_hash`, `verified`, `last_login`, `oauth_providers` fields. Only one `auth` collection per app makes sense. |
| `view` | SQL-backed, read-only (coming soon). |

## Identities at a glance

| Principal | Stored in | Authenticates at |
|---|---|---|
| Master admin | `system.db` | `/_/auth/admin/login` |
| Realm admin | The realm's `realm.db` | `/api/realms/<realm>/auth/admin/login` |
| App admin | The realm's `realm.db`, scoped to one or more apps | (same) |
| End-user | The realm's `realm.db` (in the `users` table or an auth-collection table) | `/api/realms/<realm>/auth/users/login` |

## Tokens

- **Access tokens** are stateless JWTs, 15-minute TTL by default. They carry `realm_id`, optional `app_id`, `user_id` or `admin_id`, and a role.
- **Refresh tokens** are opaque random strings stored in the realm's `_refresh_tokens` table. Exchange them at `/auth/refresh` to get a fresh access token.
- **Revocation** is in-memory; entries auto-expire on the access-token TTL. Restart the server and revocations are gone (which is fine — old tokens expire on their own clock anyway).

## What lives where

| You ask… | …it lives in |
|---|---|
| Realms registry | `system.db` |
| Master admins | `system.db` |
| Users | `<realm>/realm.db` |
| OAuth providers | `<realm>/realm.db` |
| Refresh tokens | `<realm>/realm.db` |
| Collections, records, access rules | `<realm>/apps/<app>/data.db` |
| App admins (which admin → which apps) | `<realm>/realm.db` |
| Hooks (JS/TS source) | `data/hooks/<realm>/<app>/` |
| Files (metadata) | `<realm>/apps/<app>/data.db` |
| Files (binary) | `<realm>/apps/<app>/storage/` (or S3) |
| Audit log | Each scope has its own `audit_log` table |

See [storage layout](/concepts/storage-layout) for the directory tree and [hierarchical policies](/concepts/hierarchical-policies) for how master / realm / app values cascade.
