#!/usr/bin/env bash
# Regenerate the `## [Unreleased]` block of CHANGELOG.md from the
# Conventional Commits landed since the last `vX.Y.Z` tag.
#
# Used standalone (`make changelog`) or as the first step of
# `scripts/release.sh`. Idempotent: safe to re-run.
#
# Categorisation:
#   feat:       → Added
#   fix:        → Fixed
#   perf:       → Performance
#   refactor:   → Changed
#   docs:       → Documentation
#   build:      → Build
#   ci:         → CI
#   chore: / test: / style: → skipped (kept silent)
#
# `feat(scope): subject` is rendered as `- **scope:** subject (sha)`.
# Commits with `!` after the type (e.g. `feat!: …`) are bubbled into a
# `### BREAKING` section at the top.
set -euo pipefail

LAST_TAG="$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)"
RANGE="${LAST_TAG:+$LAST_TAG..}HEAD"

if [[ -z "$LAST_TAG" ]]; then
    echo "▶ No previous vX.Y.Z tag found; scanning the whole history."
else
    echo "▶ Scanning commits since $LAST_TAG"
fi

LAST_TAG="$LAST_TAG" RANGE="$RANGE" python3 - <<'PY'
import os
import re
import subprocess
import sys

RANGE = os.environ["RANGE"]

# Pull commits as: <sha><TAB><subject><RECORD>
RECORD_SEP = "\x1e"
out = subprocess.run(
    ["git", "log", RANGE, "--no-merges",
     f"--pretty=format:%h\t%s{RECORD_SEP}"],
    check=True, capture_output=True, text=True,
).stdout.strip(RECORD_SEP + "\n")

if not out:
    print("▶ no new commits since the last tag — leaving CHANGELOG alone")
    sys.exit(0)

SECTIONS = {
    "feat":     "Added",
    "fix":      "Fixed",
    "perf":     "Performance",
    "refactor": "Changed",
    "docs":     "Documentation",
    "build":    "Build",
    "ci":       "CI",
}
SKIP = {"chore", "test", "style"}

# Order in the rendered block — keep predictable for diff-reading humans.
ORDER = ["BREAKING", "Added", "Fixed", "Performance", "Changed",
         "Documentation", "Build", "CI"]

# `feat(scope)!: subject` — group(1) = type, (2) = scope, (3) = bang,
# (4) = subject
COMMIT_RE = re.compile(
    r"^(?P<type>[a-z]+)(?:\((?P<scope>[^)]+)\))?(?P<bang>!?):\s*(?P<subject>.+)$"
)

grouped: dict[str, list[str]] = {k: [] for k in ORDER}

for entry in out.split(RECORD_SEP):
    entry = entry.strip()
    if not entry:
        continue
    sha, _, subject = entry.partition("\t")
    m = COMMIT_RE.match(subject)
    if not m:
        # Not a conventional commit — skip silently. The user can amend
        # the CHANGELOG manually if needed.
        continue
    ctype = m["type"]
    scope = m["scope"]
    bang = bool(m["bang"])
    subj = m["subject"].strip()

    section = SECTIONS.get(ctype)
    if section is None and ctype not in SKIP:
        continue
    if ctype in SKIP and not bang:
        continue

    # `(#NN)` footers added by GitHub's squash-merge — already a link,
    # let GitHub linkify it. Drop the trailing `(#NN)` to avoid a double
    # link with the `(sha)` we append below.
    subj = re.sub(r"\s*\(#\d+\)\s*$", "", subj)

    bullet = f"- {'**' + scope + ':** ' if scope else ''}{subj} ({sha})"

    if bang:
        grouped["BREAKING"].append(bullet)
    elif section:
        grouped[section].append(bullet)

# Render the new [Unreleased] body
out_lines: list[str] = []
for section in ORDER:
    if not grouped[section]:
        continue
    out_lines.append(f"### {section}")
    out_lines.extend(grouped[section])
    out_lines.append("")  # blank line between sections

if not out_lines:
    body = "(nothing yet)\n"
else:
    body = "\n".join(out_lines).rstrip() + "\n"

# Replace the existing [Unreleased] block — from the heading through the
# blank line before the next `## [`.
with open("CHANGELOG.md") as f:
    content = f.read()

m = re.search(r"^## \[Unreleased\][^\n]*\n(.*?)(?=^## \[)",
              content, flags=re.MULTILINE | re.DOTALL)
if not m:
    print("CHANGELOG.md has no [Unreleased] section to update.", file=sys.stderr)
    sys.exit(1)

new_block = "## [Unreleased]\n\n" + body + "\n"
content = content[:m.start()] + new_block + content[m.end():]

with open("CHANGELOG.md", "w") as f:
    f.write(content)

# tiny summary
total = sum(len(v) for v in grouped.values())
sections_used = [s for s in ORDER if grouped[s]]
print(f"✓ [Unreleased] regenerated: {total} entries across "
      f"{len(sections_used)} sections ({', '.join(sections_used) or 'none'}).")
PY
