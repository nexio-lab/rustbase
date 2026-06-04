# Mental model

Everything in RustBase fits into a three-level hierarchy:

```
System
  └── Workspace  (organization boundary — admins live here)
        └── App  (data product — collections, records, files, end-users, OAuth live here)
```

Understanding which layer owns which thing is the single most useful thing you can learn about the system.

## System

The system is the server itself. It tracks:

- The **registry** of workspaces.
- The **master admin(s)** — accounts that can administer the entire server.
- **Master-scope policy bounds** (see [hierarchical policies](/concepts/hierarchical-policies)).
- The **master audit log**.

There is exactly one `system.db` per RustBase instance.

## The master workspace

On first boot, RustBase creates a single privileged workspace called **the master workspace**. Its rules:

- Cannot be deleted (its name and slug can be renamed by master admins).
- Owns the master admin(s).
- Is the only place from which other workspaces can be created, edited, or deleted.

The master workspace is special, but it's still a workspace — it has its own `workspace.db`, its own apps (each with its own users if you create them), and its own admins.

## Workspaces

A workspace is an **organization boundary**. It holds:

- **Workspace admins** and **app admins** (the workspace's administrators).
- Workspace-level **branding** and settings.
- Workspace-scope **policy values** (bounded by master).
- The workspace's **audit log**.

A workspace has its own **workspace admins**, scoped to that one workspace; their token is honored across every app inside the workspace.

## Apps

An app is what a developer ships against. It owns:

- **Collections** (the schema's tables).
- **Records** in those collections.
- Per-collection **access rules**.
- **Files** uploaded for this app.
- **JS/TS hooks** loaded for this app.
- The **end-user pool** for this app — every user lives in the app's `data.db`.
- **OAuth provider** configuration (Google / GitHub / etc.).
- App-scope **policy values** (bounded by workspace and master).
- The app's **audit log**.

End-users live per-app: a user registered against `acme/mobile` is a different identity than the same email registered against `acme/web`. Authentication produces a token bound to `(workspace_id, app_id, user_id)`. If you want one identity that spans two products, run them as a single app.

An app has its own **app admins** — a subset of workspace admins, plus single-app-scoped admins.

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
| Master admin | `system.db` | `/_/auth/admin/login` (username + password) |
| Workspace admin | The workspace's `workspace.db` | `/api/workspaces/<workspace>/auth/admin/login` |
| App admin | The workspace's `workspace.db`, scoped to one or more apps | (same) |
| End-user | The app's `data.db` (in the `users` table or an auth-collection table) | `/api/workspaces/<workspace>/apps/<app>/auth/users/login` |

## Tokens

- **Access tokens** are stateless JWTs, 15-minute TTL by default. They carry a role and the scope claims that match it: master admins have neither `workspace` nor `app`; workspace and app admins carry `workspace` (app admins also carry `app`); end-user tokens always carry both `workspace` and `app`.
- **Refresh tokens** are opaque random strings. Admin refresh tokens live in the workspace's `_refresh_tokens` table; end-user refresh tokens live in the app's `_refresh_tokens` table (the `users` table moved with them). Exchange them at the matching `/auth/refresh` (master) or `/auth/refresh` (workspace admin) or `/apps/:app/auth/users/refresh` (end-user) to get a fresh access token.
- **Revocation** is in-memory; entries auto-expire on the access-token TTL. Restart the server and revocations are gone (which is fine — old tokens expire on their own clock anyway).

## What lives where

| You ask… | …it lives in |
|---|---|
| Workspaces registry | `system.db` |
| Master admins | `system.db` |
| Workspace admins, app admins | `<workspace>/workspace.db` |
| Admin refresh tokens | `<workspace>/workspace.db` |
| Apps registry, workspace-scope policies, workspace audit | `<workspace>/workspace.db` |
| End-users | `<workspace>/apps/<app>/data.db` |
| OAuth providers | `<workspace>/apps/<app>/data.db` |
| End-user refresh tokens, verifications, password resets, OTPs, TOTP, MFA challenges | `<workspace>/apps/<app>/data.db` |
| Collections, records, access rules, app audit | `<workspace>/apps/<app>/data.db` |
| Files (metadata) | `<workspace>/apps/<app>/data.db` |
| Files (binary blobs) | `<workspace>/apps/<app>/storage/` |
| Hooks (JS/TS source) | `data/hooks/<workspace>/<app>/` |
| Files (binary) | `<workspace>/apps/<app>/storage/` (or S3) |
| Audit log | Each scope has its own `audit_log` table |

See [storage layout](/concepts/storage-layout) for the directory tree and [hierarchical policies](/concepts/hierarchical-policies) for how master / workspace / app values cascade.
