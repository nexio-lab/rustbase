# Roadmap

A rough sketch of where RustBase is heading. **Nothing here is a
commitment.** Order and scope shift based on contributor interest,
sponsor support, and what feedback comes in via
[Discussions](https://github.com/pjonaszik/rustbase/discussions) and
[Issues](https://github.com/pjonaszik/rustbase/issues).

Concrete, in-flight work lives in the [milestones][m].

[m]: https://github.com/pjonaszik/rustbase/milestones

## v0.2 — quality + hardening (next minor)

Theme: *make v0.1 production-grade.*

- **Dashboard SPA fixes** — `paths.base = "/_"` is not consistently honoured;
  redirects from the route guard hit the bare `/login` instead of
  `/_/login`. Tracked in #TBA.
- **Cross-platform CI** — add macOS-arm64 and Windows runners to `ci.yml`
  so the release matrix isn't the first place these targets see compilation.
- **OpenAPI spec generation** — emit a stable `openapi.yaml` from the
  axum routes so SDK generators can target RustBase.
- **Coverage signal** — `cargo-llvm-cov` + a Codecov badge in the README.
- **Docker GHCR image** — already wired in `release.yml`; verify it works
  end-to-end on the first patch release.
- **Litestream documentation** — a hands-on guide that walks through
  enabling replication, backing up to R2, restoring on a new host.

## v0.3 — DX + ergonomics

Theme: *make RustBase pleasant to build against.*

- **JS / TS SDK** — official client generated from the OpenAPI spec.
- **CLI tool** (`rustbase admin ...`) — for prod ops without the
  dashboard (e.g. `rustbase admin reset-password <username>`).
- **Dashboard polish** — full keyboard navigation, dark mode, schema
  diff before save, audit timeline view.
- **Hook API surface** — `$app.db`, `$app.http.fetch`, scheduled `$app.cron`
  with timezone hints, `$app.realtime.publish` examples.
- **Per-collection rate limits** — hierarchical policies on
  `rate_limit.requests_per_minute`.

## v0.4 — multi-region + scale

Theme: *let RustBase hold up under load.*

- **Read replicas** via Litestream + per-app replica routing.
- **Background job queue** in the hook runtime, with retries + dead-letter
  inspection from the dashboard.
- **Better metrics** — Prometheus endpoint exposing pool sizes, request
  latency histograms, broker subscriber counts.
- **Dart and Go SDKs.**

## v1.0 — stable surface

Theme: *commit to the API contract.*

- API + storage layout frozen for the duration of `1.x`.
- Documented migration path from `0.x`.
- Security audit (depends on sponsor budget — see the **Support us**
  section of the README).
- Long-term support policy: every minor receives security fixes for at
  least 12 months after the next minor ships.

## Beyond v1.0 (speculative)

- **Plugin SDK** for native Rust extensions (alternative to JS hooks).
- **Multi-tenant Kubernetes operator** for orchestrating many RustBase
  instances.
- **Hosted RustBase Cloud** — optional managed deployment for users who
  want the OSS without the ops.

## Out of scope

- A relational query language beyond the filter expression syntax. If
  you need joins or window functions, use SQLite directly.
- A general-purpose ORM. The filter parser + access rules engine is the
  layer, not a query builder for arbitrary statements.
- Hot-reload of native Rust code. Use JS hooks for that — the runtime
  exists precisely so plugin-style customization doesn't require a Rust
  recompile.

## How to influence the roadmap

- File a feature request via the issue template.
- Comment on a milestone if you'd like to take an issue.
- Sponsor via the link at the top of the README — sponsorship lets the
  maintainer commit more hours, which moves items off this page and
  into shipped releases.
