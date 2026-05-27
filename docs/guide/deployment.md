# Deployment

The whole point of RustBaas is that there's not much to deploy. One binary, one directory.

## Smallest possible deploy

A VPS, a domain, and `systemd`:

```ini
# /etc/systemd/system/rustbase.service
[Unit]
Description=RustBaas
After=network.target

[Service]
ExecStart=/srv/rustbase/rustbase
WorkingDirectory=/srv/rustbase
Restart=always
User=rustbase

# Don't leak the binary's permission to the whole filesystem.
ProtectSystem=strict
ReadWritePaths=/srv/rustbase/data
ProtectHome=true
PrivateTmp=true
NoNewPrivileges=true

# Limits — tune to your machine
LimitNOFILE=65535

[Install]
WantedBy=multi-user.target
```

```sh
sudo useradd --system --home-dir /srv/rustbase --shell /usr/sbin/nologin rustbase
sudo mkdir -p /srv/rustbase/data
sudo cp rustbase rustbase.toml /srv/rustbase/
sudo chown -R rustbase:rustbase /srv/rustbase
sudo systemctl enable --now rustbase
```

## Reverse proxy

You almost always want a TLS-terminating reverse proxy in front. With Caddy:

```caddyfile
api.example.com {
  reverse_proxy localhost:8080
}
```

With Nginx:

```nginx
server {
  listen 443 ssl http2;
  server_name api.example.com;

  ssl_certificate     /etc/letsencrypt/live/api.example.com/fullchain.pem;
  ssl_certificate_key /etc/letsencrypt/live/api.example.com/privkey.pem;

  location / {
    proxy_pass http://127.0.0.1:8080;
    proxy_http_version 1.1;
    proxy_buffering off;                     # SSE needs streaming
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
  }
}
```

> **SSE note:** make sure your proxy doesn't buffer responses. Caddy doesn't by default. Nginx needs `proxy_buffering off` on the realtime location.

## Docker

A minimal multi-stage build:

```dockerfile
# Builder
FROM rust:1.83-slim AS builder
WORKDIR /src
RUN apt-get update && apt-get install -y --no-install-recommends curl unzip pkg-config libssl-dev
RUN curl -fsSL https://bun.sh/install | bash
ENV PATH="/root/.bun/bin:${PATH}"
COPY . .
RUN cargo build --workspace --release

# Runner
FROM debian:12-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/rustbase /usr/local/bin/rustbase
RUN useradd --system --home /data rustbase
USER rustbase
WORKDIR /data
VOLUME ["/data"]
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/rustbase"]
```

`docker run -v rustbase-data:/data/data -p 8080:8080 rustbase`.

## Backups

The whole state is on disk under `data/`. Three options, in increasing order of operational maturity:

| Strategy | Pros | Cons |
|---|---|---|
| `rsync` / `restic` cron | Trivial; works on any cloud storage. | Snapshot of an in-flight SQLite write can be inconsistent — schedule for low-traffic windows. |
| `sqlite3 .backup` | Consistent snapshot. | One-shot; you script the cadence. |
| [Litestream](/guide/backups) | Continuous replication to S3; sub-second RPO. | Adds a sidecar; extra config. |

For production, Litestream is the answer. See [the backups page](/guide/backups).

## Updating

Stop the binary, replace it, start it:

```sh
sudo systemctl stop rustbase
sudo cp rustbase.new /srv/rustbase/rustbase
sudo systemctl start rustbase
```

Migrations run automatically on boot. There are no "downtime windows" beyond the seconds it takes for the new process to come up.

## Sizing

RustBaas's footprint is small. Single-node defaults on a $5/mo VPS are enough for tens of thousands of records and a handful of low-traffic apps. The bottleneck before CPU is almost always SQLite write throughput — if you're doing >100 writes/sec on hot collections, profile before scaling out.

The pool caps (`realm_pool_cap`, `app_pool_cap`) bound how many SQLite connections sit open. Each pool is one connection. RAM cost per pool is tiny (~1 MB).
