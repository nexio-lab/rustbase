# Introduction

RustBaas is a **multi-tenant Backend-as-a-Service** packaged as a single statically-linked Rust binary that operates against a `data/` directory of SQLite files — one per scope — plus an embedded JS/TS hook runtime.

## What you get

Out of the box, every RustBaas instance ships with:

- **Realms** — organization boundaries. Each realm groups its apps under one set of administrators, branding, and policies.
- **Apps** — data products. Each app has its own schema (collections), records, files, hooks, access rules, **end-user pool**, and OAuth provider configuration.
- **Authentication** — email + password, email OTP, TOTP second factor, OAuth2 (Google, GitHub, any OIDC).
- **Three admin tiers** — master, realm, and app admins, each scoped exactly to what they manage.
- **REST API** — typed JSON, filterable lists, generated OpenAPI spec.
- **Realtime** — SSE and WebSocket subscriptions on every collection.
- **File storage** — local disk or any S3-compatible bucket (AWS, R2, MinIO) via `object_store`.
- **JS/TS hooks** — lifecycle handlers, custom HTTP routes, scheduled cron jobs, sent through an embedded QuickJS sandbox.
- **Hierarchical policies** — master sets bounds, realms tighten, apps pick values. Auto-clamp + audit when a parent narrows.
- **Audit log** — append-only, queryable per scope.
- **Dashboard** — SvelteKit SPA embedded in the binary, served at `/_/`.

## What it isn't

RustBaas is **not** a horizontally-scaled, multi-region system. SQLite is a single-writer database. Read replicas are out of scope. If you need a fleet of stateless API servers in front of Postgres, this isn't your tool.

What RustBaas **is** great at:

- Small-to-medium SaaS where you want operational simplicity.
- Internal tools and side projects.
- On-prem deployments where you ship the whole product as one binary plus a data folder.
- Anything where "drop one binary on a server" beats "manage a fleet."

## Built on

| Concern | Choice |
|---|---|
| Async runtime | `tokio` |
| HTTP | `axum` |
| Database | SQLite via `sqlx` |
| JS runtime | `rquickjs` + `swc` (TS strip) |
| Password hashing | `argon2` |
| JWT | `jsonwebtoken` |
| File storage | `object_store` |
| Filter parser | `nom` |
| Dashboard | SvelteKit, embedded via `include_dir!` |
| Replication | Litestream (optional) |

Read the [mental model](/concepts/mental-model) next, then jump to [getting started](/guide/getting-started).
