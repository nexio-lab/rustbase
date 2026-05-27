---
layout: home

hero:
  name: RustBaas
  text: One binary. One data folder. Your backend.
  tagline: A multi-tenant Backend-as-a-Service in Rust — realms, apps, collections, auth, hooks, files, realtime, and a built-in dashboard.
  actions:
    - theme: brand
      text: Get started
      link: /guide/getting-started
    - theme: alt
      text: Mental model
      link: /concepts/mental-model
    - theme: alt
      text: View on GitHub
      link: https://github.com/pjonaszik/rustbase

features:
  - icon: 📦
    title: Single binary, one data folder
    details: SQLite under the hood. Drop one executable on a server, run the setup wizard, and you have a working backend. No services to install, no migrations to run by hand.
  - icon: 🏢
    title: Multi-tenant by design
    details: System → realm → app. Each app owns its end-user pool, OAuth config, and data; realms group apps under one administrative tenant.
  - icon: 🔐
    title: Auth that fits
    details: Email + password, email OTP, TOTP, OAuth2 (Google, GitHub, any OIDC). Refresh tokens with rotation. Three layers of admins.
  - icon: ⚡
    title: Realtime out of the box
    details: SSE and WebSocket subscriptions on every collection. Records publish to in-process pub/sub on every create / update / delete.
  - icon: 🧩
    title: Extensible without a redeploy
    details: Embedded QuickJS runtime. Drop a .js or .ts file into data/hooks and lifecycle handlers, cron jobs, and custom HTTP routes light up.
  - icon: 🛡️
    title: Hierarchical policies
    details: Master sets bounds. Realm tightens. App picks a value inside both. Auto-clamp + audit when a parent narrows.
  - icon: 📊
    title: Operator-friendly
    details: Per-scope audit log, embedded SvelteKit dashboard, optional Litestream replication. Backups are object storage; restores are a directory copy.
  - icon: 🦀
    title: Built in Rust
    details: axum + sqlx + tokio. Zero unsafe. Property-tested config engine. Comprehensive test suite that runs on every commit.
---

## Why RustBaas?

You want a backend. You don't want to wire up Postgres, Redis, S3, an auth service, a queue, a cron runner, and an admin UI before you can ship your first feature.

RustBaas gives you all of that in one binary, with a `data/` directory you can `scp` or `tar` for backups and a dashboard at `/_/` for everything you'd otherwise need a custom admin panel for.

```sh
# Run it
./rustbase

# Visit the dashboard
open http://localhost:8080/_/

# Make a request
curl http://localhost:8080/api/realms/master/apps/blog/collections/posts/records \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"title":"Hello","body":"first post"}'
```

That's it. The setup wizard creates the master admin on first visit, the API is documented in the [REST reference](/reference/rest-api), and JS hooks under `data/hooks/<realm>/<app>/` extend the runtime without a rebuild.
