#!/usr/bin/env bash
#
# Boot a release build of `rustbase` against a throw-away `data/`
# directory for Playwright end-to-end tests. The dashboard is served
# from the binary's embedded copy (no `RUSTBASE_DASHBOARD_PATH`
# override), so the same artifact you ship to prod is what's under
# test.
#
# This script `exec`s the binary in the foreground; Playwright's
# `webServer` driver hands it SIGTERM on teardown. The temp `data/`
# directory is wiped on exit so re-running the suite always starts
# fresh.

set -euo pipefail

# Repo root regardless of where the script is invoked from.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT"

PORT="${E2E_PORT:-8989}"
DATA_DIR="$(mktemp -d -t rustbase-e2e-XXXXXX)"
trap 'rm -rf "$DATA_DIR"' EXIT INT TERM

# Build the dashboard with VITE_BASE=/_ so the SvelteKit runtime
# `base` matches the embedded mount path. We always run this — it's
# cheap (~3s with rolldown) compared with re-running the binary
# build, and it sidesteps the cargo-build-rs cache when the
# developer flipped `ui/src` between cargo invocations.
echo "e2e: building dashboard with VITE_BASE=/_ …" >&2
VITE_BASE=/_ bun --cwd ui run build >&2

# Build the binary if it isn't already there (or stale rebuild is
# requested via E2E_REBUILD=1). The release build is what we test;
# `cargo build --release` is the gating step everyone hits once.
if [ "${E2E_REBUILD:-0}" = "1" ] || [ ! -x target/release/rustbase ]; then
    echo "e2e: building rustbase (release)…" >&2
    cargo build --release -p rustbase-server
fi

echo "e2e: boot port=$PORT data=$DATA_DIR" >&2

# Disable the per-IP rate limit and lockout policy so a spec hammering
# the auth endpoints doesn't trip the global guards. Security headers
# + body cap stay on (they're what we want to keep verified in CI).
#
# `RUSTBASE_DASHBOARD_PATH` points at the freshly-built dashboard so
# the binary serves it from disk instead of the embedded snapshot —
# that snapshot was taken whenever the binary was last compiled and
# may lag the current `ui/src` tree.
exec env \
    RUSTBASE_LISTEN="127.0.0.1:$PORT" \
    RUSTBASE_DATA_DIR="$DATA_DIR" \
    RUSTBASE_DASHBOARD_PATH="$ROOT/ui/build" \
    RUSTBASE_RATE_LIMIT__ENABLED=false \
    RUSTBASE_LOCKOUT__ENABLED=false \
    RUST_LOG="${RUST_LOG:-warn,rustbase=info}" \
    target/release/rustbase
