#!/usr/bin/env bash
# One-shot environment bootstrap for new contributors.
#
#   ./scripts/setup-dev.sh
#       or
#   make setup-dev
#
# What it does:
#   1. Check the required toolchain is available (Rust 1.88+, Bun,
#      Python 3, git, docker). Missing dependencies print an actionable
#      install hint and the script exits non-zero before doing anything
#      destructive.
#   2. Install the local git hooks (.githooks/) so pre-commit + pre-push
#      checks fire on every commit.
#   3. Warm up the dependency caches: `cargo fetch`, `bun install` in
#      `ui/` and `docs/`.
#   4. Tell the contributor what to run next.
#
# It deliberately does NOT install anything system-wide — that's the
# contributor's call. The script exists so a fresh clone can be made
# build-ready with one command instead of a paragraph of setup notes.
set -euo pipefail

# ----- helpers --------------------------------------------------------

cyan()  { printf '\033[36m%s\033[0m' "$*"; }
green() { printf '\033[32m%s\033[0m' "$*"; }
red()   { printf '\033[31m%s\033[0m' "$*"; }
hr()    { printf '%s\n' "----------------------------------------"; }

MISSING=0

need() {
    local name="$1" check="$2" install_hint="$3"
    if eval "$check" > /dev/null 2>&1; then
        printf '  %s %s\n' "$(green '✓')" "$name"
    else
        printf '  %s %s\n     %s\n' "$(red '✗')" "$name" "$install_hint"
        MISSING=$((MISSING + 1))
    fi
}

require_rust_min() {
    local min="$1"
    if ! command -v rustc > /dev/null 2>&1; then
        printf '  %s rustc (≥ %s)\n     install via https://rustup.rs/\n' \
            "$(red '✗')" "$min"
        MISSING=$((MISSING + 1))
        return
    fi
    local current
    current="$(rustc --version | awk '{print $2}')"
    if [[ "$(printf '%s\n%s\n' "$min" "$current" | sort -V | head -1)" \
            != "$min" ]]; then
        printf '  %s rustc %s (need ≥ %s)\n     run: rustup update stable\n' \
            "$(red '✗')" "$current" "$min"
        MISSING=$((MISSING + 1))
    else
        printf '  %s rustc %s\n' "$(green '✓')" "$current"
    fi
}

# ----- 1. toolchain check ---------------------------------------------

printf '\n%s\n' "$(cyan '▶ Checking toolchain')"
hr
require_rust_min "1.88.0"
need "cargo"   "command -v cargo"   "ships with rustup; see https://rustup.rs/"
need "bun"     "command -v bun"     "install: curl -fsSL https://bun.sh/install | bash"
need "python3" "command -v python3" "install your distro's python3 package"
need "git"     "command -v git"     "install your distro's git package"
need "docker"  "command -v docker"  "optional — only used by 'make docker' and the smoke test"

if [[ $MISSING -gt 0 ]]; then
    printf '\n%s %d required tool(s) are missing. Install them and re-run.\n' \
        "$(red '✗')" "$MISSING"
    exit 1
fi

# ----- 2. git hooks ---------------------------------------------------

printf '\n%s\n' "$(cyan '▶ Installing git hooks (.githooks/)')"
hr
"$(dirname "$0")/install-hooks.sh"

# ----- 3. warm dependency caches --------------------------------------

printf '\n%s\n' "$(cyan '▶ Warming Cargo cache (cargo fetch)')"
hr
cargo fetch

printf '\n%s\n' "$(cyan '▶ Installing UI dashboard dependencies (bun install)')"
hr
bun install --cwd ui

printf '\n%s\n' "$(cyan '▶ Installing docs dependencies (bun install)')"
hr
bun install --cwd docs

# ----- 4. next steps --------------------------------------------------

cat <<EOF

$(green '✓ Dev environment ready.')

Next steps:

  $(cyan 'make check')        — same gate CI runs (fmt, clippy, tests)
  $(cyan 'make build')        — release binary at target/release/rustbase
  $(cyan 'make ui-dev')       — SvelteKit dashboard on :5173 with API proxy
  $(cyan 'make docs-dev')     — VitePress docs site dev server
  $(cyan 'make help')         — list every target

To run the server end-to-end:

  $(cyan 'make build && ./target/release/rustbase')
  $(cyan 'open http://localhost:8080/_/   # finish the setup wizard')

If you'll be cutting releases, see CONTRIBUTING.md → Release process.
EOF
