# Deployment

This guide takes you from the v0.1 binary to a single-instance production
deployment behind TLS with backups and monitoring. Everything assumes
**one machine, one binary** — that's the design.

If you need multi-instance or multi-region, RustBase is the wrong tool today
(see [Positioning](../concepts/positioning)).

## Pick a host

Any Linux VM with ≥ 1 vCPU and ≥ 512 MB RAM. Concrete options:

- **Hetzner CX22** — €4.51/mo, 2 vCPU, 4 GB RAM. Best price/perf for v0.1.
- **DigitalOcean Basic** — $6/mo equivalent.
- **Fly.io shared-cpu-1x** — $1.94/mo + bandwidth. Works but file-system
  ephemerality is a footgun (mount a volume).

Pick Debian 12 / Ubuntu 24.04 LTS. Other distros work; these are the
documented path.

## Get the binary onto the box

Three options, ranked by laziness:

### 1. Download the release tarball

```sh
VERSION=v0.1.1
curl -fsSL -o rustbase.tar.gz \
    https://github.com/pjonaszik/rustbase/releases/download/${VERSION}/rustbase-${VERSION}-linux-x86_64-musl.tar.gz
curl -fsSL -o rustbase.tar.gz.sha256 \
    https://github.com/pjonaszik/rustbase/releases/download/${VERSION}/rustbase-${VERSION}-linux-x86_64-musl.tar.gz.sha256
sha256sum -c rustbase.tar.gz.sha256
tar -xzf rustbase.tar.gz
sudo install -m 0755 rustbase /usr/local/bin/rustbase
```

### 2. Run the Docker image

```sh
docker run -d --name rustbase --restart unless-stopped \
    -p 127.0.0.1:8080:8080 \
    -v /srv/rustbase:/home/rustbase/data \
    ghcr.io/pjonaszik/rustbase:0.1.1
```

Pinning to `0.1.1` (not `:latest`) is intentional — `:latest` will surprise
you on container restart after a release.

### 3. Build from source

If you've forked or you need a non-released commit:

```sh
git clone https://github.com/pjonaszik/rustbase.git
cd rustbase
make build      # ./target/release/rustbase
sudo install -m 0755 target/release/rustbase /usr/local/bin/rustbase
```

Needs Rust ≥ 1.88 and Bun on the build host. The release tarball is the
fastest path; this is the escape hatch.

## Run it as a service

### systemd unit (binary deployment)

```ini
# /etc/systemd/system/rustbase.service
[Unit]
Description=RustBase
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=rustbase
Group=rustbase
WorkingDirectory=/srv/rustbase
ExecStart=/usr/local/bin/rustbase
Restart=on-failure
RestartSec=5

# Hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/srv/rustbase
PrivateTmp=true
PrivateDevices=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
LockPersonality=true
MemoryDenyWriteExecute=true
RestrictRealtime=true
RestrictSUIDSGID=true
SystemCallArchitectures=native
SystemCallFilter=@system-service
SystemCallFilter=~@privileged @resources

# Resource caps — pick to match your host
MemoryMax=512M
TasksMax=128
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
```

Then:

```sh
sudo useradd --system --home-dir /srv/rustbase --shell /usr/sbin/nologin rustbase
sudo install -d -o rustbase -g rustbase -m 0750 /srv/rustbase
sudo systemctl daemon-reload
sudo systemctl enable --now rustbase
sudo systemctl status rustbase
```

## Put TLS in front (Caddy)

The binary listens on plain HTTP. **Don't expose it directly.** Caddy is the
shortest path to automatic TLS via Let's Encrypt.

```sh
sudo apt install -y debian-keyring debian-archive-keyring apt-transport-https
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | sudo gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | sudo tee /etc/apt/sources.list.d/caddy-stable.list
sudo apt update && sudo apt install -y caddy
```

```caddyfile
# /etc/caddy/Caddyfile
your-domain.tld {
    encode zstd gzip
    reverse_proxy 127.0.0.1:8080 {
        # SSE needs flushing, not buffering. Caddy defaults are friendly here
        # but be explicit so a future tweak doesn't surprise you.
        flush_interval -1
    }

    # Security headers — these are the bare minimum until RustBase v0.2 sets
    # them itself.
    header {
        Strict-Transport-Security "max-age=31536000; includeSubDomains"
        X-Content-Type-Options "nosniff"
        Referrer-Policy "strict-origin-when-cross-origin"
        Permissions-Policy "geolocation=(), microphone=(), camera=()"
        # CSP is intentionally restrictive — adjust if you serve your own
        # dashboard or embed third-party scripts.
        Content-Security-Policy "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'"
    }

    log {
        output file /var/log/caddy/rustbase.log {
            roll_size 50mb
            roll_keep 5
        }
        format json
    }
}
```

```sh
sudo systemctl reload caddy
```

Done. `your-domain.tld` now has a working TLS-fronted RustBase.

### Nginx (alternative)

```nginx
server {
    listen 443 ssl http2;
    server_name your-domain.tld;

    ssl_certificate     /etc/letsencrypt/live/your-domain.tld/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/your-domain.tld/privkey.pem;

    # Security headers
    add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;
    add_header X-Content-Type-Options "nosniff" always;
    add_header Referrer-Policy "strict-origin-when-cross-origin" always;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_buffering off;        # SSE needs streaming
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

## Backups

### Tarballs (good enough for v0.1 if you can tolerate hourly RPO)

```ini
# /etc/systemd/system/rustbase-backup.service
[Unit]
Description=RustBase tarball backup

[Service]
Type=oneshot
User=rustbase
ExecStart=/bin/sh -c '\
  DATE=$(date -u +%%Y%%m%%dT%%H%%M%%SZ) ; \
  tar -czf /srv/rustbase-backups/rustbase-$DATE.tar.gz -C /srv rustbase ; \
  find /srv/rustbase-backups -name "rustbase-*.tar.gz" -mtime +14 -delete'
```

```ini
# /etc/systemd/system/rustbase-backup.timer
[Unit]
Description=Hourly RustBase backup

[Timer]
OnCalendar=hourly
Persistent=true

[Install]
WantedBy=timers.target
```

```sh
sudo install -d -o rustbase -g rustbase -m 0750 /srv/rustbase-backups
sudo systemctl daemon-reload
sudo systemctl enable --now rustbase-backup.timer
```

Sync the backups off-box with `restic`, `rclone`, or `aws s3 sync` on a
separate timer. **Always test the restore** before you need it.

### Litestream (continuous, lag-bounded RPO)

```toml
# rustbase.toml
[litestream]
enabled = true
bucket = "s3://my-rustbase-backups"
prefix = "prod"
replicate_interval_sec = 10
```

RustBase auto-generates `litestream.yml` at boot. Run Litestream as a sidecar:

```ini
# /etc/systemd/system/litestream.service
[Unit]
Description=Litestream
After=rustbase.service
Requires=rustbase.service

[Service]
Type=simple
User=rustbase
ExecStart=/usr/local/bin/litestream replicate -config /srv/rustbase/litestream.yml
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

The restore command is `litestream restore -config litestream.yml /path/to/the/db.db`.
You restore each `*.db` separately — `system.db`, every realm's `realm.db`,
every app's `data.db`.

::: warning
The Litestream integration ships in v0.1 but is **not yet covered by an
end-to-end test in CI**. Set up a quarterly drill that restores from your
S3 bucket to a fresh box and verifies the boot.
:::

## Monitoring

### Logs

`rustbase` uses `tracing`. Default level is `info`. Bump via env:

```sh
sudo systemctl edit rustbase
# add:
#   [Service]
#   Environment="RUST_LOG=rustbase=debug,info"
sudo systemctl restart rustbase
```

For structured logs, scrape `journalctl -u rustbase -o json` into Loki /
Datadog / whatever your shop runs.

### Health check

```sh
curl -fsS http://127.0.0.1:8080/healthz
```

Returns `{"initialized": true|false}`. Wire that to your uptime monitor.
A 200 just means the process is alive; `"initialized": false` means the
setup wizard hasn't been completed.

### Metrics

v0.1 has no `/metrics` endpoint. Until v0.2 ships one, scrape host metrics
(CPU, RAM, disk) via Prometheus node_exporter.

### Disk

Watch `/srv/rustbase` size. Each `*.db-wal` file can grow under sustained
writes before WAL checkpoints — usually self-limiting, but worth alerting on
if it exceeds 100 MB per file.

## Upgrades

### Binary deployment

```sh
sudo systemctl stop rustbase
sudo install -m 0755 ./rustbase /usr/local/bin/rustbase
sudo systemctl start rustbase
```

The migration system auto-applies pending migrations on boot. **Back up first**
before a minor-version upgrade.

### Docker deployment

```sh
docker pull ghcr.io/pjonaszik/rustbase:0.1.2
docker stop rustbase
docker rm rustbase
docker run -d --name rustbase --restart unless-stopped \
    -p 127.0.0.1:8080:8080 \
    -v /srv/rustbase:/home/rustbase/data \
    ghcr.io/pjonaszik/rustbase:0.1.2
```

## Sizing

RustBase's footprint is small. Single-node defaults on a Hetzner CX22 or a
DigitalOcean $6 droplet are enough for tens of thousands of records and
a handful of low-traffic apps. The bottleneck before CPU is almost always
SQLite write throughput — if a single app sustains >100 writes/sec on hot
collections, profile before scaling out.

The pool caps (`realm_pool_cap`, `app_pool_cap`) bound how many SQLite
connections sit open. Each pool is one connection. RAM cost per pool is
tiny (~1 MB).

## Hardening checklist

The v0.2 release fills several of these gaps. Until then, you do them.

- [ ] `ufw` allows only 80, 443, and your SSH port.
- [ ] Caddy / nginx terminates TLS — RustBase does not.
- [ ] Backups run on a schedule and are tested by restoring to a scratch host.
- [ ] Litestream (or equivalent) replicates to an off-box bucket.
- [ ] systemd unit has `MemoryMax=` and `TasksMax=` set.
- [ ] `RUST_LOG` is set to `info` (not `debug`) in production.
- [ ] The master admin's password is in a password manager, not in shell history.
- [ ] You've reviewed [`SECURITY.md`](https://github.com/pjonaszik/rustbase/blob/main/SECURITY.md) and know how to report a vulnerability.
- [ ] You have a runbook for "the binary won't start" — typically: bad config,
      missing data dir, exhausted file descriptors.

## Built-in security layer

As of the security-hardening pack (post-0.1.1), the binary ships with
the following enabled by default — no extra reverse-proxy config
required:

- **Per-IP rate limit** at the HTTP entry layer (token bucket, 50 r/s,
  100 burst by default — tunable via `[rate_limit]` in
  `rustbase.toml`). Rejections return `429 too_many_requests` +
  `Retry-After`.
- **Per-subject auth lockout** (5 failures inside 5 min → 5 min
  lockout). Failures share a budget across password / TOTP /
  email-OTP for the same identity. Audit rows: `login_success`,
  `login_failed`, `login_locked`.
- **Security headers** on every response: HSTS (`max-age=63072000;
  includeSubDomains`), `X-Content-Type-Options: nosniff`,
  `Referrer-Policy: strict-origin-when-cross-origin`, `X-Frame-Options:
  DENY`, a baseline `Content-Security-Policy`, and a restrictive
  `Permissions-Policy`. Disable in `[http]` only if your reverse proxy
  already injects them.
- **CORS allowlist** under `[cors]`. Empty (default) means same-origin
  only — the dashboard is same-origin, so most installs need nothing
  here.
- **Body size cap** (`[http].max_body_bytes`, default 8 MiB).

If you have Caddy/nginx in front and it's already setting headers,
flip `http.security_headers = false` to avoid duplicate headers (most
proxies tolerate them, but it's noisy in logs).

See [Configuration → rate_limit / lockout / http / cors](configuration.html#full-reference)
for every knob, and [Error codes → 429](../reference/errors.html#status--code-mapping)
for the response shape.

## JWT verification by external systems

RustBase issues **RS256** access tokens and publishes the public
verification key at:

```
GET https://<your-domain>/.well-known/jwks.json
GET https://<your-domain>/_/auth/jwks.json
```

Both endpoints return the same JSON Web Key Set (`Content-Type:
application/jwk-set+json`). The response carries `Cache-Control:
public, max-age=3600`, so a downstream service should cache the key
for up to one hour and refresh on cache miss.

Standard JWT libraries (jose, jsonwebtoken, oidc-client-ts, etc.)
consume this format without any custom config — point them at the
`/.well-known/jwks.json` URL and they pick the right key by `kid`.

The RSA-2048 keypair is generated once at first boot and persisted as
PKCS#8 DER under `system.db._secrets`. The `kid` is deterministic
(SHA-256 of the public key, truncated) so it does not change across
restarts; rotation will arrive in a later milestone.

::: warning HS256 transition
A server upgraded from v0.1.x keeps a legacy HS256 verification key
loaded so already-issued symmetric tokens continue to validate until
their access-token TTL expires. Once the TTL window has elapsed, the
fallback path is effectively dormant; fresh installs never touch it.
:::

## What's still on the v0.2+ roadmap

- `/metrics` endpoint (Prometheus exposition format).
- PKCE on the OAuth flows.
- Cookie-based session for the dashboard (httpOnly, SameSite=Strict).
- Operator-driven key rotation flow (regenerate RSA key + dual-key
  serving window).

This guide will be updated as each lands.
