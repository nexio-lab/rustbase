# Observability

RustBase ships a Prometheus-compatible `/metrics` endpoint and structured tracing logs. Both are operator-facing: the endpoint is gated behind a bearer token, the logs default to JSON-ready key-value fields.

## Metrics endpoint

Off by default. Enable it in `rustbase.toml`:

```toml
[observability]
metrics_enabled = true
metrics_token = "change-me-to-a-long-random-string"
```

When `metrics_enabled = true`, boot installs the global Prometheus recorder and mounts `GET /metrics` on the main HTTP listener. The endpoint requires `Authorization: Bearer <metrics_token>`; any other request returns **404** (not 401 — scrapers without the token should not learn that the endpoint exists).

If `metrics_enabled = true` without a non-empty `metrics_token`, boot aborts with:

```
[observability] metrics_enabled = true requires a non-empty metrics_token
```

This is intentional: silently exposing metrics on a misconfigured deployment would be worse than refusing to start.

### What's exported

**HTTP**

```
# TYPE rustbase_http_requests_total counter
rustbase_http_requests_total{method="GET",route="/api/workspaces/{workspace}/apps/{app}/collections/{coll}/records",status="200"} 1432

# TYPE rustbase_http_request_duration_seconds histogram
rustbase_http_request_duration_seconds_bucket{method="GET",route="/healthz",status="200",le="0.005"} 12
…

# TYPE rustbase_build_info gauge
rustbase_build_info{version="0.1.1"} 1
```

**Auth**

```
# TYPE rustbase_auth_logins_total counter
# {kind=master|workspace, outcome=success|failed|locked}
rustbase_auth_logins_total{kind="master",outcome="success"} 12
rustbase_auth_logins_total{kind="workspace",outcome="failed"} 4
rustbase_auth_logins_total{kind="workspace",outcome="locked"} 1

# TYPE rustbase_auth_refresh_total counter
# {kind=master|user|workspace_admin, outcome=success|failed}
rustbase_auth_refresh_total{kind="user",outcome="success"} 87
rustbase_auth_refresh_total{kind="user",outcome="failed"} 2
```

A burst of `outcome="failed"` followed by `outcome="locked"` is the lockout policy at work; an `outcome="failed"` rate-of-change spike with no `locked` follow-up is a brute-force probe spread across distinct usernames. Refresh failures clustered around the access-token TTL are usually clients that lost their refresh token between rotations.

**Database pools**

```
# TYPE rustbase_db_pools_open gauge
# {scope=workspace|app}
rustbase_db_pools_open{scope="workspace"} 8
rustbase_db_pools_open{scope="app"} 14
```

Bounded by `workspace_pool_cap` and `app_pool_cap` in the config (defaults 32 and 64). A gauge sitting at the cap means the LRU is evicting on every new tenant access — bump the cap if your access pattern doesn't have warm-set locality.

**Realtime**

```
# TYPE rustbase_realtime_channels_open gauge
rustbase_realtime_channels_open 17

# TYPE rustbase_realtime_events_published_total counter
# {outcome=delivered|no_subscribers}
rustbase_realtime_events_published_total{outcome="delivered"} 421
rustbase_realtime_events_published_total{outcome="no_subscribers"} 89
```

A high `no_subscribers` rate means the server is doing publishing work nothing observes — typically harmless, but worth checking your hook `$app.realtime.publish` calls against actual client subscriptions if you've optimised hot paths.

Notes:

- The `route` label uses **axum's `MatchedPath`** — the template (`/api/workspaces/{workspace}/...`), not the literal URI. Cardinality stays bounded regardless of how many workspaces/apps/collections you operate.
- The duration histogram uses explicit buckets at `1ms, 5ms, 10ms, 25ms, 50ms, 100ms, 250ms, 500ms, 1s, 2.5s, 5s, 10s`. Anything outside that range almost always means the request was queued behind a busy DB pool.
- Requests that don't match any registered route — for example, dashboard 404s for missing static assets — are labelled `route="<unmatched>"`.
- `rustbase_build_info` is the standard Prometheus shape for shipping the running version alongside the runtime metrics, set to `1` at boot.

### Scrape config example

```yaml
scrape_configs:
  - job_name: rustbase
    metrics_path: /metrics
    scheme: https
    static_configs:
      - targets: ['rustbase.example.com:443']
    authorization:
      type: Bearer
      credentials: change-me-to-a-long-random-string
```

The bearer token can also be provided via `bearer_token_file` for secrets-management workflows.

## Tracing logs

RustBase emits structured tracing logs on stdout from boot through every request. Levels follow the `RUST_LOG` env var, defaulting to `info`:

```
RUST_LOG=info,sqlx=warn ./rustbase
```

Sample boot output:

```
2026-06-05T14:42:04Z  INFO rustbase_db::migrations: applied migration migration="20260521_000002_app_files" elapsed_ms=0
2026-06-05T14:42:04Z  INFO rustbase_api::apps: app created workspace=acme app=mobile
2026-06-05T14:42:04Z  INFO rustbase: jwt: RS256 active key kid=…
2026-06-05T14:42:04Z  INFO rustbase: rustbase: ready listen=127.0.0.1:8080
```

Every log line that names a workspace, app, collection, or record carries those values as **structured fields** rather than baking them into the message. Pipe `RUST_LOG` output to your aggregator of choice (Loki, ELK, Vector → S3) and use the field names in your queries.

The same key names are used across crates so cross-crate correlation works without reformatting:

| Field | Meaning |
|---|---|
| `workspace` | Workspace slug |
| `app` | App slug |
| `collection` | Collection slug |
| `record` | Record ID |
| `kid` | JWT signing-key ID (RS256) |
| `kind` | Mailer/Storage backend variant |
| `error` | The `Display`-formatted error chain |

## What's not (yet) exported

The first wave covered HTTP and the second wave covers auth, DB pools, and the realtime broker (above). Still on the list: hook invocation counts and durations, file upload bytes by app, and mailer dispatch counts. Those will land alongside the next pass on hook + storage instrumentation.
