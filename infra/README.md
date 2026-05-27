# Shared dev infrastructure

Docker services that any local app can talk to — not specific to
RustBaas. Currently:

- **MailHog** — capture-only SMTP relay + web UI. Stops mail from
  escaping during development. Inbox is in-memory; restart wipes it.
- **MinIO** — S3-compatible object store + web console. Serves as the
  dev-mode `[storage.s3]` target. A `rustbase-dev` bucket is created
  on first boot by the `minio-bootstrap` one-shot job.

## Bring it up

```sh
docker compose -f infra/docker-compose.yml up -d
```

Tear down with `down`. The compose project name is `rustbase-dev-infra`,
so it won't collide with project-specific containers.

## Endpoints

| Service | Host port | Purpose |
|---|---|---|
| MailHog SMTP | `localhost:1025` | point your app's `[mail.smtp]` here |
| MailHog UI   | <http://localhost:8025> | browse captured messages |
| MinIO S3     | `localhost:9000` | point your app's `[storage.s3].endpoint` here |
| MinIO console| <http://localhost:9001> | browse buckets (login `minioadmin` / `minioadmin`) |

## Wiring RustBaas to MailHog

Drop this into `rustbase.toml` (or set the env vars below):

```toml
[mail.smtp]
host = "localhost"
port = 1025
tls  = "none"
```

Equivalent env: `RUSTBASE_MAIL__SMTP__HOST=localhost`,
`RUSTBASE_MAIL__SMTP__PORT=1025`, `RUSTBASE_MAIL__SMTP__TLS=none`.

After a `verify-email/request` or `password-reset/request`, the
message appears at <http://localhost:8025>.

## Wiring RustBaas to MinIO

```toml
[storage.s3]
bucket            = "rustbase-dev"
region            = "us-east-1"
endpoint          = "http://localhost:9000"
access_key_id     = "minioadmin"
secret_access_key = "minioadmin"
```

Equivalent env: `RUSTBASE_STORAGE__S3__BUCKET=rustbase-dev`,
`RUSTBASE_STORAGE__S3__REGION=us-east-1`,
`RUSTBASE_STORAGE__S3__ENDPOINT=http://localhost:9000`,
`RUSTBASE_STORAGE__S3__ACCESS_KEY_ID=minioadmin`,
`RUSTBASE_STORAGE__S3__SECRET_ACCESS_KEY=minioadmin`.

After an upload via `POST /api/realms/<r>/apps/<a>/files`, the object
appears in the `rustbase-dev` bucket at
<http://localhost:9001/browser/rustbase-dev>.

## Sharing with other apps

Two paths, depending on how the other app runs:

**Other app runs on the host** — just point it at `localhost:1025`.
Nothing else to configure.

**Other app runs in its own docker compose** — attach to the
`dev-shared` network from its compose file and address the relay by
container name:

```yaml
# other-app/docker-compose.yml
services:
  api:
    # ...
    networks:
      - dev-shared
    environment:
      SMTP_HOST: mailhog
      SMTP_PORT: "1025"

networks:
  dev-shared:
    external: true
```

The first time you need the cross-app network, promote it to a
hand-managed Docker network so its lifecycle isn't tied to either
compose project:

```sh
docker network create dev-shared
```

Then flip `dev-shared` in `infra/docker-compose.yml` from the implicit
bridge to `external: true` (uncomment the marker in the file).
