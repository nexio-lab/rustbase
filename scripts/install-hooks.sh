#!/usr/bin/env bash
#
# Point this repo's git hooks at `.githooks/` so the tracked pre-commit
# and pre-push scripts run on every commit/push. Idempotent: re-running
# is a no-op.
#
# Usage:  ./scripts/install-hooks.sh

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

git config core.hooksPath .githooks
chmod +x .githooks/*

echo "hooks installed: git core.hooksPath = $(git config core.hooksPath)"
echo
echo "to bypass for one commit:  git commit --no-verify"
echo "to disable entirely:       git config --unset core.hooksPath"
