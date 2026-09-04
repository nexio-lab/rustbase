#!/usr/bin/env bash
# Cut a release locally.
#
#   V=0.1.2 scripts/release.sh
#       or
#   scripts/release.sh 0.1.2
#
# What it does:
#   1. validate V and a clean working tree
#   2. run fmt + clippy + tests (same as CI's `check` job)
#   3. bump `workspace.package.version` in Cargo.toml
#   4. rotate `## [Unreleased]` in CHANGELOG.md → `## [vX.Y.Z] — YYYY-MM-DD`
#      and prepend a fresh empty `## [Unreleased]`
#   5. refresh Cargo.lock via `cargo check`
#   6. `git commit -m "release: vX.Y.Z"`
#   7. `git tag -s vX.Y.Z -m "vX.Y.Z"` (SSH-signed)
#
# It does NOT push. Run `make release-push V=X.Y.Z` (or the two git push
# lines printed at the end) once you've reviewed the commit.
set -euo pipefail

V="${V:-${1:-}}"
TODAY="$(date -u +%Y-%m-%d)"
GIT_NAME="pjonaszik"
GIT_EMAIL="lionus1@live.fr"

# ----- arg + repo sanity -----------------------------------------------
[[ -n "$V" ]] \
    || { echo "usage: V=X.Y.Z $0   (or: $0 X.Y.Z)" >&2; exit 1; }
[[ "$V" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.-]+)?$ ]] \
    || { echo "V must be X.Y.Z (got: $V)" >&2; exit 1; }
[[ -z "$(git status --porcelain)" ]] \
    || { echo "working tree is dirty; commit or stash first" >&2; exit 1; }

# ----- preflight: same checks CI runs ----------------------------------
echo "▶ cargo fmt --all --check"
cargo fmt --all --check
echo "▶ cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --workspace --all-targets -- -D warnings
echo "▶ cargo test --workspace"
cargo test --workspace --quiet

# ----- bump workspace.package.version ---------------------------------
echo "▶ bumping Cargo.toml → version = \"$V\""
# Replace only the FIRST `^version = "..."` — that's workspace.package.version
# at the top of the file. `sed -i.bak` works on both GNU and BSD sed.
sed -i.bak '0,/^version = "[^"]*"$/{s//version = "'"$V"'"/}' Cargo.toml
rm -f Cargo.toml.bak

# ----- has the changelog kept up with the code? -----------------------
# Rotating `[Unreleased]` publishes whatever it happens to contain. A
# non-empty block is no proof it is complete: a release can carry code
# whose entries were never written, and the omission becomes permanent
# the moment the tag is pushed — the content of a file at a tag cannot
# be corrected without moving the tag.
#
# The check is deliberately not textual. It asks one question a script
# can answer exactly: did any code land AFTER the changelog was last
# touched? If so the changelog cannot describe it.
#
# Escape hatch for the honest case (a refactor worth no entry):
#   SKIP_CHANGELOG_CHECK=1 make release V=X.Y.Z
if [[ "${SKIP_CHANGELOG_CHECK:-0}" != "1" ]]; then
    LAST_TAG="$(git describe --tags --abbrev=0 2>/dev/null || true)"
    RANGE="${LAST_TAG:+$LAST_TAG..}HEAD"
    CHANGELOG_COMMIT="$(git log -1 --format=%H "$RANGE" -- CHANGELOG.md || true)"
    SINCE="${CHANGELOG_COMMIT:-$LAST_TAG}"
    CODE_AFTER="$(git log --format='  %h %s' "${SINCE:+$SINCE..}HEAD" \
        -- crates ui/src sdks 2>/dev/null || true)"
    if [ -n "$CODE_AFTER" ]; then
        echo "✗ code landed after CHANGELOG.md was last updated:" >&2
        echo "$CODE_AFTER" >&2
        echo >&2
        echo "  Their entries are missing from [Unreleased], and rotating it" >&2
        echo "  now would publish a release that documents less than it" >&2
        echo "  contains. Write them, or re-run with" >&2
        echo "  SKIP_CHANGELOG_CHECK=1 if none of them warrants an entry." >&2
        exit 1
    fi
    echo "▶ changelog: up to date with the code since ${LAST_TAG:-the first commit}"
fi

# ----- optionally regenerate [Unreleased] from git log ---------------
# Default: leave the hand-written `## [Unreleased]` block alone. A
# crafted changelog that explains *why* each change happened reads much
# better than the cold `type(scope): subject (sha)` bullets the auto-
# generator emits.
#
# Opt in with REGEN_CHANGELOG=1 if you want a starting draft synthesised
# from commit messages — useful for paths where you've kept conventional
# commits clean and don't want to write it by hand.
if [[ "${REGEN_CHANGELOG:-0}" == "1" ]]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    "$SCRIPT_DIR/changelog.sh"
else
    echo "▶ keeping hand-written [Unreleased] (pass REGEN_CHANGELOG=1 to autogen)"
fi

# ----- rotate CHANGELOG.md --------------------------------------------
echo "▶ rotating CHANGELOG.md: [Unreleased] → [$V] — $TODAY"
V="$V" DATE="$TODAY" python3 - <<'PY'
import os
v = os.environ["V"]
date = os.environ["DATE"]
with open("CHANGELOG.md") as f:
    content = f.read()
if "## [Unreleased]" not in content:
    raise SystemExit("CHANGELOG.md is missing a '## [Unreleased]' section")
new_header = (
    "## [Unreleased]\n"
    "\n"
    "(nothing yet)\n"
    "\n"
    f"## [{v}] — {date}"
)
content = content.replace("## [Unreleased]", new_header, 1)
with open("CHANGELOG.md", "w") as f:
    f.write(content)
PY

# ----- refresh Cargo.lock so the bump propagates ----------------------
echo "▶ cargo check --workspace (refresh Cargo.lock)"
cargo check --workspace > /dev/null

# ----- commit + sign tag ----------------------------------------------
echo "▶ git commit -m \"release: v$V\""
git add Cargo.toml Cargo.lock CHANGELOG.md
git -c "user.name=$GIT_NAME" -c "user.email=$GIT_EMAIL" \
    commit -m "release: v$V"

echo "▶ git tag -s v$V"
git -c "user.name=$GIT_NAME" -c "user.email=$GIT_EMAIL" \
    tag -s "v$V" -m "v$V"

# ----- next steps ------------------------------------------------------
cat <<EOF

✓ Local release prepared. Review with:

    git show v$V
    git log -1

Push when ready:

    make release-push V=$V

  or manually:

    git push origin main
    git push origin v$V

To revert (before pushing):

    git tag -d v$V
    git reset --hard HEAD~1
EOF
