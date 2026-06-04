# Configuration

RustBase reads its config from `rustbase.toml` in the working directory. Every key is also overridable by an `RUSTBASE_*` environment variable — env wins over file.

## Bare minimum

You don't need a config file. The defaults are:

```toml
listen        = "0.0.0.0:8080"
data_dir      = "./data"
realm_pool_cap = 32
app_pool_cap   = 64
```

…with no SMTP relay, no S3 storage, and no Litestream replication. The server uses a `LogMailer` (captures + traces mails but doesn't deliver) and the local-disk storage backend.

## Full reference

```toml
# HTTP listener address.
listen = "0.0.0.0:8080"

# Path to the data directory. Created if missing.
data_dir = "./data"

# LRU caps for per-realm and per-app SQLite pools.
realm_pool_cap = 32
app_pool_cap = 64

# ------------------------------------------------------------------
# Optional SQLite → S3 replication via Litestream. Off by default.
# ------------------------------------------------------------------
[litestream]
enabled = false
# bucket = "s3://my-rustbase-backups"
# prefix = "prod"
# replicate_interval_sec = 10

# ------------------------------------------------------------------
# Optional SMTP relay. Without [mail.smtp], all mail goes to the
# LogMailer (traced but never delivered) — fine for dev. Set it
# before you ship to prod, otherwise verify-email / password-reset
# flows can never complete.
# ------------------------------------------------------------------
[mail.smtp]
host = "smtp.example.com"
port = 587
tls  = "start_tls"   # "none" | "start_tls" | "implicit"
username = "alerts@example.com"
password = "${SMTP_PASSWORD}"   # env interpolation works in strings
from = "RustBase <noreply@example.com>"

# ------------------------------------------------------------------
# Optional S3-compatible file storage. Absent → files live under
# data_dir/realms/<realm>/apps/<app>/storage/. The DB metadata is
# unchanged whether you pick local or S3.
# ------------------------------------------------------------------
[storage.s3]
bucket            = "rustbase-prod"
region            = "us-east-1"
endpoint          = "https://s3.example.com"    # omit for AWS
access_key_id     = "AKIA..."
secret_access_key = "${S3_SECRET}"
virtual_hosted_style_request = false             # set false for MinIO, true for AWS
```

## Environment variables

Every key is reachable as `RUSTBASE_<UPPERCASE_PATH_JOINED_BY_UNDERSCORES>`. Examples:

| Setting | Env var |
|---|---|
| `listen` | `RUSTBASE_LISTEN` |
| `data_dir` | `RUSTBASE_DATA_DIR` |
| `realm_pool_cap` | `RUSTBASE_REALM_POOL_CAP` |
| `mail.smtp.host` | `RUSTBASE_MAIL_SMTP_HOST` |
| `mail.smtp.password` | `RUSTBASE_MAIL_SMTP_PASSWORD` |
| `litestream.bucket` | `RUSTBASE_LITESTREAM_BUCKET` |
| `storage.s3.bucket` | `RUSTBASE_STORAGE_S3_BUCKET` |

`12-factor`-style — set secrets via env in production; keep `rustbase.toml` in source control with non-secret defaults.

## Runtime override for the dashboard

By default, the SvelteKit dashboard is **embedded** in the binary at build time via `include_dir!`. If you want to test a custom build of the dashboard against an already-running server, set:

```sh
RUSTBASE_DASHBOARD_PATH=/path/to/ui/build ./rustbase
```

The server will serve files from that directory instead of the embedded ones, with the same SPA fallback (every unknown path under `/_/` falls through to `index.html` for client-side routing).

## Logging

Logging follows the `RUST_LOG` env convention (`tracing-subscriber`):

```sh
RUST_LOG=info ./rustbase                # default
RUST_LOG=debug,rustbase_db=trace ./rustbase
```

Per-request access logs come out of `tower_http::trace::TraceLayer`, with the request method, path, status, and elapsed time.

## Hierarchical policies vs. static config

Anything in `rustbase.toml` is **static** — it's the same for every realm and app and changes only when you restart the server (or HUP if/when that lands).

**Hierarchical policies** (password rules, token TTLs, rate limits, hook capabilities, etc.) live in the database. Master sets bounds; realms tighten; apps pick. Edit them in the dashboard under the **Policies** tab, or via the [REST API](/reference/rest-api#policies).

The rule of thumb: if it's about *the server* (where it listens, what backend it uses) it's in `rustbase.toml`. If it's about *a realm or app* (what users can do) it's a policy.
