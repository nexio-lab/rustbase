# Getting started

This page walks you from zero to a logged-in admin in under five minutes.

## Prerequisites

- A working **Rust toolchain** (only needed if building from source — pre-built binaries are also available on the [releases page](https://github.com/pjonaszik/rustbase/releases)).
- **bun** ≥ 1.0 if you plan to hack on the dashboard. The `rustbase` binary already embeds a pre-built dashboard.
- Optional: **Docker** for the shared dev infra (MailHog SMTP, MinIO S3 emulator).

## Install

::: code-group

```sh [Pre-built binary]
# Linux / macOS — adjust for your platform
curl -L https://github.com/pjonaszik/rustbase/releases/latest/download/rustbase-x86_64-unknown-linux-gnu.tar.gz \
  | tar -xz
chmod +x rustbase
```

```sh [From source]
git clone https://github.com/pjonaszik/rustbase.git
cd rustbase
./scripts/install-hooks.sh        # wire up git hooks
cargo build --release
# The binary is at target/release/rustbase
```

:::

## First run

```sh
./rustbase
```

Output:

```
INFO rustbase_server: server starting on 0.0.0.0:8080
INFO rustbase_server: dashboard at http://localhost:8080/_/
INFO rustbase_server: uninitialized — visit /_/setup to create the first master admin
```

Visit **http://localhost:8080/_/** in your browser. The server auto-seeded a master admin row at first boot with username `admin` and no password, so you'll be redirected to the **setup wizard**. Enter a password and submit.

That's it — you're now signed in as the `admin` master admin.

## Create your first workspace and app

From the dashboard:

1. Click **Workspaces** in the top nav.
2. Click **+ New workspace**, give it an id (`acme`) and a name (`Acme Inc.`). The setup wizard already created the special **master workspace**, but production data goes into the workspaces you create.
3. Open the new workspace, click **+ New app**, give it an id (`web`) and a name (`Web`).
4. Open the app, click **+ New collection**. Pick `base` for plain records or `auth` for end-users.
5. Add fields to the schema using the inline editor. Required fields and types validate at write time.

You can do every step above through the REST API instead — see the [first app](/guide/first-app) walkthrough for the curl equivalents.

## Where data lives

After your first writes, the working directory looks like this:

```
.
├── rustbase                    # the binary
├── rustbase.toml               # optional config
└── data/
    ├── system.db               # workspaces registry, master admins
    └── workspaces/
        ├── master/
        │   └── workspace.db
        └── acme/
            ├── workspace.db        # apps, workspace/app admins, admin refresh tokens, policies
            └── apps/
                └── web/
                    ├── data.db # collections, records, access rules, users, oauth, refresh tokens
                    └── storage/
```

Back up the entire `data/` directory and you've backed up the whole server. Restore it on another machine and everything keeps working.

## Configure

Drop a `rustbase.toml` next to the binary to override defaults:

```toml
data_dir = "./data"
bind = "0.0.0.0:8080"
workspace_pool_cap = 32
app_pool_cap = 64

[smtp]
host = "smtp.example.com"
port = 587
username = "alerts@example.com"
password = "${SMTP_PASSWORD}"
from = "RustBase <noreply@example.com>"

[storage]
backend = "local"               # or "s3"

[litestream]
enabled = false                 # set to true to back up to S3 continuously
```

Every key is also overridable as `RUSTBASE_*` env vars — `RUSTBASE_BIND="0.0.0.0:9000"` wins over the file.

See the [configuration reference](/guide/configuration) for the full list.

## Next steps

- Build an end-to-end feature: [first app walkthrough](/guide/first-app)
- Wire OAuth: [authentication](/guide/authentication)
- Extend the server with JS: [hooks](/guide/hooks)
- Ship to production: [deployment](/guide/deployment)
