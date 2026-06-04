# RustBase vs PocketBase, Supabase, Appwrite

An honest take. If you skim, jump to the [decision matrix](#decision-matrix)
at the bottom.

## What each tool optimises for

| Tool | Optimises for | One sentence |
|---|---|---|
| **PocketBase** | Solo dev, one app per binary | Single-tenant SQLite BaaS in a Go binary with a hosted-grade admin UI. |
| **Supabase** | Teams that want Postgres + managed cloud | Postgres + Auth + Storage + Edge Functions + dashboard, available self-hosted or on Supabase Cloud. |
| **Appwrite** | Mobile-first teams | Self-hosted BaaS with native SDKs for every mobile platform; multi-container, requires Docker compose to run. |
| **RustBase** | One operator running N small apps under one tenant | Multi-tenant SQLite BaaS in a Rust binary, with one `data.db` per app for real isolation. |

## Feature-by-feature

### Multi-tenancy

- **PocketBase** — single tenant per instance. Need N tenants → run N processes.
  At ≈30 MB RAM each, that's manageable to ~50 tenants; past that, it's
  reverse-proxy gymnastics.
- **Supabase** — Postgres schemas + RLS. Multi-tenancy is your job to model,
  policy-by-policy.
- **Appwrite** — *projects* concept, but multi-tenant on shared MariaDB.
  Isolation is logical, not physical.
- **RustBase** — built-in. `System → Realm → App` is enforced at the storage
  layer. One SQLite file per app means deleting an app removes its data
  bit-for-bit, and a noisy app can't corrupt a sibling.

### Database

- **PocketBase** — SQLite, single-writer.
- **Supabase** — Postgres. Real concurrent writes, replicas, the lot.
- **Appwrite** — MariaDB, with shared connection pool.
- **RustBase** — SQLite per scope. Single-writer **per file**, so a hot app
  bottlenecks itself but not its siblings. WAL + busy_timeout + `synchronous=NORMAL`
  set on every pool.

### Auth

| | PocketBase | Supabase | Appwrite | RustBase |
|---|---|---|---|---|
| Email + password | ✓ | ✓ | ✓ | ✓ |
| Passwordless OTP | — | ✓ | ✓ | ✓ |
| TOTP / MFA | — | ✓ | ✓ | ✓ |
| OAuth / OIDC | ✓ | ✓ | ✓ | ✓ (Google, GitHub, Microsoft, custom) |
| Per-tenant user pool | n/a (single tenant) | one Postgres | shared MariaDB | **one SQLite per app** |
| Master / org / app admin tiers | admin only | one admin role | team roles | **three tiers** |

### Server runtime / extensibility

| | PocketBase | Supabase | Appwrite | RustBase |
|---|---|---|---|---|
| Lifecycle hooks | Go callbacks (compiled) | Postgres triggers + Edge Functions (Deno) | Functions (any language, separate container) | **JS/TS hooks in QuickJS** (embedded) |
| Custom HTTP routes | Go callbacks | Edge Functions | Functions | **`$app.routerAdd`** (JS) |
| Scheduled jobs | Cron via Go | pg_cron | Cron in Functions | `$app.cron` (in roadmap for v0.3) |
| Need Node.js? | No | Yes (Edge Functions) | Yes (most Functions) | **No** |
| Sandboxing | No (compiled in) | Deno | Container isolation | QuickJS VM per app |

### Realtime

- **PocketBase** — SSE built-in.
- **Supabase** — Postgres LISTEN/NOTIFY → Realtime server → WebSocket.
- **Appwrite** — WebSocket subscriptions.
- **RustBase** — SSE today, WebSocket in v0.3. In-process broker; single-instance.

### File storage

All four support local + S3-compatible. RustBase uses `object_store`, which
covers AWS, R2, MinIO, and any S3 API.

### Dashboard

- **PocketBase** — class-leading. Native Svelte admin, JSON record editing,
  schema editor.
- **Supabase** — feature-rich, multi-tenant-friendly.
- **Appwrite** — comprehensive, mobile-focused.
- **RustBase** — SvelteKit SPA embedded in the binary. Functional for v0.1;
  polish (dark mode, optimistic updates, bulk actions) lands in v0.3.

### Operational footprint

| | PocketBase | Supabase | Appwrite | RustBase |
|---|---|---|---|---|
| Install steps | 1 binary | Docker compose (≈8 containers) or hosted | Docker compose (≈5 containers) | **1 binary** |
| Memory at idle | ≈30 MB | ≈2 GB | ≈1 GB | **≈40 MB** |
| Disk overhead | ≈30 MB | ≈5 GB | ≈2 GB | **≈18 MB** |
| Backup primitive | `cp data.db` | `pg_dump` or PITR | mysqldump + volumes | **`tar data/`** |
| Multi-region | No | Yes (Postgres replication) | Manual | No (by design) |
| Horizontal scale | One writer per binary | Yes | Containerised | **No** (single instance) |

### SDKs

- **PocketBase** — JS, Dart, Go, custom.
- **Supabase** — official JS, Dart, Python, Swift, etc.
- **Appwrite** — broadest SDK matrix in the BaaS world (Android, iOS, Flutter,
  Web, server-side everywhere).
- **RustBase** — REST API only today. JS SDK auto-generated from OpenAPI lands
  in v0.4.

### Licence

| | Licence |
|---|---|
| PocketBase | MIT |
| Supabase | Apache-2.0 (mostly; some parts Postgres-licensed) |
| Appwrite | BSD-3 |
| **RustBase** | **MIT OR Apache-2.0** |

## Decision matrix

Pick **PocketBase** if:

- You're shipping a single app to a single client / community.
- You don't need OTP, TOTP, or strict tenant isolation.
- You want the most polished BaaS admin UI in this size class.

Pick **Supabase** if:

- You need real Postgres (window functions, joins, RLS, extensions).
- You're a team of 3+ engineers who can own RLS policies.
- You want managed hosting as a first-class option.
- Your write volume will exceed what SQLite handles (~100 RPS sustained).

Pick **Appwrite** if:

- You're building a mobile-first product and want native SDKs day one.
- You're comfortable operating a Docker compose stack of ≈5 containers.

Pick **RustBase** if:

- You run **N small apps** under **one organisation** and want strict isolation
  between them without N reverse proxies.
- You want JS hooks **without a Node.js runtime** in the picture.
- You want SQLite simplicity with a multi-tenant primitive baked in.
- You're operating on the budget side and one Hetzner box is your "infra".
- You're comfortable with the v0.1 maturity: core works, polish + security
  hardening is what v0.2 / v0.3 brings.

## What RustBase is *not*

- Not a Postgres replacement. There are no joins across collections, no window
  functions, no Postgres extensions.
- Not horizontally scalable. The realtime broker is in-process and SQLite
  writes are single-threaded per file. Plan one instance, plan capacity from
  there.
- Not a mobile SDK ecosystem. The SDKs ship in v0.4; for now it's REST + your
  preferred HTTP client.
- Not (yet) hardened for public-facing production. v0.2 ships the rate-limit /
  PKCE / JWKS pack that you need for that.
