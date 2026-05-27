<div align="center">

# RustBaas

**One binary. One data folder. Your backend.**

A multi-tenant Backend-as-a-Service in Rust. Drop one executable on a server, run the setup wizard, and you have realms, apps, collections, auth, realtime, file storage, a dashboard, and a REST API.

<!-- status row -->
[![CI](https://img.shields.io/github/actions/workflow/status/pjonaszik/rustbase/ci.yml?branch=main&label=CI&style=for-the-badge&logo=githubactions&logoColor=white)](https://github.com/pjonaszik/rustbase/actions/workflows/ci.yml)
[![Docs](https://img.shields.io/github/actions/workflow/status/pjonaszik/rustbase/docs.yml?branch=main&label=Docs&style=for-the-badge&logo=readthedocs&logoColor=white)](https://pjonaszik.github.io/rustbase/)
[![Release](https://img.shields.io/github/v/release/pjonaszik/rustbase?include_prereleases&sort=semver&style=for-the-badge&logo=github&logoColor=white)](https://github.com/pjonaszik/rustbase/releases)
[![Licence](https://img.shields.io/badge/licence-MIT%20OR%20Apache--2.0-blue?style=for-the-badge)](#licence)

<!-- platform + community row -->
[![Rust](https://img.shields.io/badge/Rust-1.85%2B-dea584?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![axum](https://img.shields.io/badge/axum-0.8-FFA500?style=for-the-badge&logo=tokio&logoColor=white)](https://github.com/tokio-rs/axum)
[![SQLite](https://img.shields.io/badge/SQLite-WAL-003B57?style=for-the-badge&logo=sqlite&logoColor=white)](https://www.sqlite.org/wal.html)
[![Discussions](https://img.shields.io/badge/Discussions-open-2188ff?style=for-the-badge&logo=github)](https://github.com/pjonaszik/rustbase/discussions)

<!-- vitals + funding row -->
[![Stars](https://img.shields.io/github/stars/pjonaszik/rustbase?style=for-the-badge&logo=github)](https://github.com/pjonaszik/rustbase/stargazers)
[![Last commit](https://img.shields.io/github/last-commit/pjonaszik/rustbase/main?style=for-the-badge&logo=git&logoColor=white)](https://github.com/pjonaszik/rustbase/commits/main)
[![PRs welcome](https://img.shields.io/badge/PRs-welcome-brightgreen?style=for-the-badge&logo=github)](https://github.com/pjonaszik/rustbase/blob/main/CONTRIBUTING.md)
[![Support](https://img.shields.io/badge/Support%20us-PayPal-00457C?style=for-the-badge&logo=paypal&logoColor=white)](https://www.paypal.com/ncp/payment/5L8KUWE8F2PSU)

📖 [Docs](https://pjonaszik.github.io/rustbase/) &nbsp;·&nbsp; 🐛 [Issues](https://github.com/pjonaszik/rustbase/issues) &nbsp;·&nbsp; 💬 [Discussions](https://github.com/pjonaszik/rustbase/discussions) &nbsp;·&nbsp; 🔒 [Security](SECURITY.md) &nbsp;·&nbsp; 📝 [Changelog](CHANGELOG.md)

</div>

---

## Why

You want a backend. You don't want to wire up Postgres, Redis, S3, an auth
service, a queue, a cron runner, and an admin UI before you can ship your first
feature. RustBaas gives you all of that in one binary, backed by SQLite, with a
dashboard at `/_/` for everything you'd otherwise need a custom admin panel
for.

## Features

- **Three-level tenancy** — `System → Realm → App`. Each app owns its own
  schema, end-user pool, OAuth config, files, and hooks.
- **Auth** — email + password, email OTP (passwordless), TOTP second factor,
  and OAuth2 / OIDC (Google, GitHub, Microsoft presets shipped).
- **Three admin tiers** — master, realm, and app admins, each scoped exactly
  to what they manage.
- **Realtime** — SSE subscriptions on every collection. Hooks publish on
  every create / update / delete.
- **File storage** — local disk or any S3-compatible bucket (AWS, R2, MinIO)
  via `object_store`.
- **JS/TS hooks** — embedded QuickJS runtime. Drop a `.js` or `.ts` file into
  `data/hooks/<realm>/<app>/` and lifecycle handlers, cron jobs, and custom
  HTTP routes light up. No Node.js required.
- **Hierarchical policies** — master sets bounds, realms tighten, apps pick
  values. Auto-clamp + audit when a parent narrows.
- **Audit log per scope**, append-only.
- **Embedded SvelteKit dashboard** at `/_/`, served straight from the binary.
- **Optional Litestream replication** to any S3 endpoint.

## Quick start

Download the binary for your platform from the
[latest release](https://github.com/pjonaszik/rustbase/releases/latest), or
build from source:

```sh
git clone https://github.com/pjonaszik/rustbase.git
cd rustbase
cargo build --release
./target/release/rustbase
```

Then:

1. Open <http://localhost:8080/_/> in your browser.
2. The server auto-seeded an `admin` master-admin row at first boot with no
   password — the **setup wizard** asks you to set one. Submit it.
3. You're signed in. Create your first realm and your first app.
4. Hit the REST API: `/api/realms/<realm>/apps/<app>/collections/...`.

The full walkthrough lives at
[**docs / first-app**](https://pjonaszik.github.io/rustbase/guide/first-app).

## Build from source

You need:

- Rust ≥ 1.85 (stable). Install via [`rustup`](https://rustup.rs/).
- [Bun](https://bun.sh/) for the embedded dashboard and the docs site.

```sh
cargo build --release           # produces ./target/release/rustbase
cargo test --workspace          # ≈ 20 s
bun --cwd ui run dev            # dashboard dev server, proxies API on :8080
bun --cwd docs run dev          # docs dev server (VitePress)
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full dev guide, hook setup, and
conventions.

## Architecture in 60 seconds

```
System
  └── Realm  (organization boundary — admins live here)
        └── App  (data product — collections, records, files, end-users, OAuth live here)
```

Storage layout:

```
data/
  system.db                     # realms registry, master admins
  realms/
    <realm_id>/
      realm.db                  # apps, realm/app admins, admin refresh tokens
      apps/
        <app_id>/
          data.db               # collections, records, users, oauth, app audit
          storage/              # file blobs
  hooks/
    <realm_id>/<app_id>/        # JS/TS hook source
```

One binary. One `data/` folder. Backups are object storage; restores are a
directory copy.

Full mental model: <https://pjonaszik.github.io/rustbase/concepts/mental-model>.

## Contributing

Bugs, feature ideas, and PRs are welcome. Please read
[CONTRIBUTING.md](CONTRIBUTING.md) first — it covers the dev workflow,
the conventions the code expects, and how to ship a clean PR.

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

For security issues, see [SECURITY.md](SECURITY.md) — please do **not** open a
public issue for those.

## Support us

RustBaas is built and maintained on personal time. If it helps you ship — or
if you just want to encourage more work on it — contributions are welcome
through PayPal. The link is a payment link, so you enter the amount yourself
(no fixed tiers, no recurring trap, just a one-off transfer of whatever feels
right):

[**→ Support RustBaas on PayPal**](https://www.paypal.com/ncp/payment/5L8KUWE8F2PSU)

<p align="center">
  <a href="https://www.paypal.com/ncp/payment/5L8KUWE8F2PSU">
    <img src="docs/public/donation-qrcode.png" alt="Scan to support RustBaas via PayPal" width="180" />
  </a>
  <br>
  <em>Scan to support — opens the PayPal payment page.</em>
</p>

Every contribution — code, docs, bug reports, PayPal — keeps the project
moving. Thank you.

## Licence

Dual-licensed under either of:

- MIT License ([LICENSE-MIT](LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <https://www.apache.org/licenses/LICENSE-2.0>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
licence, shall be dual licensed as above, without any additional terms or
conditions.
