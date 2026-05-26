# Shared dev infrastructure

Docker services that any local app can talk to — not specific to
RustBase. Currently:

- **MailHog** — capture-only SMTP relay + web UI. Stops mail from
  escaping during development. Inbox is in-memory; restart wipes it.

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

## Wiring RustBase to MailHog

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
