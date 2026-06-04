# Positioning — who this is for

A short, opinionated read.

## Who RustBase is built for

### The agency / freelance shop running N client apps

You ship internal tools for clients. Three different verticals, three different
schemas, three different user pools. Today you stand up three PocketBase
instances behind three reverse-proxy paths, three Postgres schemas with RLS,
or three docker-compose stacks. You spend more time on the plumbing than the
features.

RustBase gives you one binary, one `data/` folder, and a `System → Realm → App`
hierarchy where each *app* is a fully isolated SQLite database. Add a new
client = create a realm, create their apps. Delete a client = `rm -rf` their
folder. Backup = `tar`. No new container, no new domain, no new auth service
per client.

### The indie hacker with a portfolio of side products

You have three half-finished ideas, two side hustles, and a "maybe this is the
one" project. They don't share users. They don't share schemas. They share
your weekend.

RustBase puts all of them behind one process, with one dashboard. Each app's
data is its own file, so when one of the side hustles dies, you delete its
folder and move on. When one suddenly grows, you can lift its `data.db` out
into its own RustBase instance — same binary, no migration.

### The internal-tools team in a small company

You're 1–3 engineers running a fleet of tiny internal tools — an admin panel,
a contractor portal, a status page, a runbook collector. The "right" stack
(Postgres + Redis + S3 + auth-service + admin UI per tool) is overkill.
PocketBase per tool means N reverse-proxy entries and N admins to remember.

RustBase puts every tool into the same multi-tenant primitive. One admin
hierarchy, one set of policies, one binary to update.

## Who RustBase is **not** built for

### High-write workloads

SQLite is single-writer per file. Per *app*, sustained writes above ~100 RPS
will hit lock contention. Cross-app writes are independent — each `data.db`
has its own writer — so the global ceiling scales with the number of hot apps,
but no single app outgrows that limit without architectural surgery.

If you're shipping a chat app, a real-time leaderboard, or anything with
write fan-out, **don't pick RustBase**. Postgres-based solutions
(Supabase, plain Postgres + libraries) are the right shape.

### Horizontally scaled deployments

The realtime broker is `tokio::sync::broadcast` — in-process, single-instance.
There is no shared session store, no clustered cache, no event bus
behind it. Multi-instance is **not on the v1 roadmap**.

If you need a load balancer in front of N RustBase instances, that pattern
breaks two of the design's foundations (SQLite writer-per-file, in-process
broker). Use a different tool.

### Multi-region / DR-sensitive workloads

A RustBase install is one binary on one machine. Litestream replication to
S3 gives you a recovery point (lag-bounded by `replicate_interval_sec`), but
restoring to a second region is a manual process and reads aren't served from
that replica. If your RPO is "zero data loss" or your RTO is "minutes", this
is not the tool.

### Compliance-bound workloads without external review

The v0.1 surface is honest engineering, but it has not been through a SOC2
audit, a HIPAA review, or a third-party penetration test. The v0.2 hardening
pack (rate limits, PKCE, JWKS, signed releases, SBOM) is the precondition for
that conversation, not the conclusion.

### Anyone who needs a polished mobile SDK day one

Appwrite is your tool. We'll have a JS/TS SDK in v0.4 and Dart/Go after, but
"the entire mobile SDK matrix already exists" is not a v1 promise.

## What "multi-tenant" means here precisely

The word is overloaded. RustBase's flavour:

- **Shared infrastructure.** One binary, one Postgres-style admin, one
  dashboard.
- **Physical isolation per app.** Each app's data is in its own SQLite file.
  No cross-app query is possible. A noisy app cannot drag a sibling app down
  via shared connection pool.
- **Two grouping levels.** *Realm* groups apps under a single administrative
  tenant (the agency / org). *App* groups data under a single product.
- **End-users live per app.** A user registered against `acme/mobile` is a
  different identity than the same email against `acme/web`. This is the
  current default; a realm-shared identity pool is on the v0.4 roadmap.

What it is *not*:

- Not "logical multi-tenancy" via row-level security (Supabase's flavour).
- Not "schema-per-tenant" (Postgres pattern).
- Not "container-per-tenant" (Kubernetes pattern).
- Not Salesforce-style "every customer is a tenant" — closer to "every
  customer is a *realm*, every product they have is an *app*."

## When to outgrow RustBase

You will outgrow this tool. That's healthy. Signs you should plan migration:

- An individual app sustains >100 RPS writes for hours.
- You need real-time joins or window functions on cross-collection queries.
- You need to scale a hot app horizontally (multiple processes serving the
  same data).
- You're paying for SOC2 / HIPAA / PCI compliance and need a paper trail
  RustBase doesn't currently provide.

The migration path is friendly: every app's data is a self-contained SQLite
file. You can lift one out, point a Postgres ETL at it, and run the migrated
app on Supabase (or whatever fits the new constraint) while leaving the
other apps on RustBase.
