<!--
Thanks for sending a PR. A few asks before you submit:

- For non-trivial changes, please open an issue first so we can agree on the
  shape of the change. Drive-by refactors and "I rewrote part of the codebase
  to my taste" PRs are unlikely to be merged.
- Keep the PR focused. Split unrelated changes into separate PRs.
- Follow the conventions in CONTRIBUTING.md.
-->

## Summary

<!-- 1-3 sentences describing what changed and why. -->

## Linked issue

<!-- Closes #N — or "n/a, this is a small fix". -->

## Changes

<!-- Bullet list of the concrete changes. Cite file paths or symbols when relevant. -->

-
-

## Test plan

<!-- How did you verify this works? Which tests cover it? -->

- [ ] `cargo fmt --all --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] If UI changed: `bun --cwd ui run build`
- [ ] If docs changed: `bun --cwd docs run build`
- [ ] CHANGELOG.md updated under `## [Unreleased]`
- [ ] Docs updated if the change is user-visible

## Notes for the reviewer

<!-- Trade-offs, follow-ups, things you punted on. Optional. -->
