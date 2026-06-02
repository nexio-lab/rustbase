# Contributing to RustBaas

Thanks for taking an interest. This doc covers how to set up a working tree,
the conventions the code expects, and how to send work upstream.

## Code of conduct

Participation is governed by the [Contributor Covenant 2.1](CODE_OF_CONDUCT.md).
Be excellent to each other.

## Ground rules

Before opening a non-trivial PR, **open an issue first** so we can discuss the
shape of the change. Drive-by refactors are unlikely to be merged. Small,
focused PRs are.

By submitting a contribution, you agree to license it under the project's
dual licence (**MIT OR Apache-2.0**).

## Dev environment

You need:

- **Rust ≥ 1.85** (stable). Install via [`rustup`](https://rustup.rs/).
- **Bun** for the dashboard / docs (`curl -fsSL https://bun.sh/install | bash`).
- **Docker** if you want to exercise the optional MailHog integration test
  (`infra/docker-compose.yml`).

Clone and wire the git hooks once:

```sh
git clone git@github.com:pjonaszik/rustbase.git
cd rustbase
./scripts/install-hooks.sh
```

That sets `core.hooksPath` to `.githooks/`. The `pre-commit` hook runs
`cargo fmt --check`, `cargo clippy -- -D warnings`, an architectural grep
suite, and a no-AI-attribution scan on the staged diff. The `pre-push` hook
runs the full test suite (and `cargo audit` if it's installed).

You can skip the hooks for a single commit / push with `--no-verify`, but CI
will run the same checks, so it's usually faster to fix locally.

## Running things

```sh
# build the workspace
cargo build --workspace

# run the full test suite (≈ 20 s)
cargo test --workspace

# linters
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings

# audit transitive deps
cargo install cargo-audit          # one-off
cargo audit

# dashboard dev server (proxies API to :8080)
bun --cwd ui run dev

# docs dev server (VitePress)
bun --cwd docs run dev
```

To run the server itself:

```sh
cargo run -p rustbase-server
# → 0.0.0.0:8080
# → dashboard at http://localhost:8080/_/
```

The first dashboard visit walks the master-admin setup wizard.

## Coding conventions

- **No `unwrap()` / `expect()` outside `#[cfg(test)]`.** The pre-commit grep
  enforces this — when it trips, propagate the error with `?` instead.
- **`rustbase-core` stays IO-free.** It only holds domain types + the filter
  parser. The architectural grep enforces it; don't add `sqlx`/`tokio`/`reqwest`
  there.
- **No raw SQL with interpolated user input.** Use `sqlx::query!`, the
  workspace's parameterized builder, or the `FilterNode → SQL` translator. The
  filter translator parameterizes every literal.
- **Use `thiserror` for crate-level error enums, `anyhow` only at binary boundaries.**
- **Don't add AI attribution** anywhere — commits, code comments, doc strings,
  identifiers, trailers. RustBaas is a human-maintained project.
- **Comments explain *why*, not *what*.** Skip comments that paraphrase the
  code. Note hidden constraints, surprising invariants, workarounds, and
  references to issues / RFCs.

## Commit messages

Use the [Conventional Commits](https://www.conventionalcommits.org/) shape:

```
<type>(<scope>): <subject>

<body — what changed and why, not how>
```

Common types: `feat`, `fix`, `refactor`, `docs`, `chore`, `ci`, `test`,
`build`. Scopes are crate names (`rustbase-db`, `ui`, `docs`, etc.) or
broader areas (`auth`, `hooks`, `release`).

Bug-fix subjects must say what's fixed, not just where: `fix(auth): reject
empty password in user_register` beats `fix: bug in register`.

## PR checklist

Before opening a PR:

- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` all pass locally
- [ ] If you touched the dashboard or docs, `bun --cwd ui run build` / `bun --cwd docs run build` succeed
- [ ] Updated CHANGELOG.md under the `## [Unreleased]` section
- [ ] Updated docs (`docs/`) when the change is visible to users — endpoints, config keys, mental model, etc.
- [ ] Added or updated tests for the change

GitHub Actions runs the same checks on every PR. Merging requires green CI.

## Tests

- **Unit tests** live in the same file under `#[cfg(test)] mod tests { ... }`.
- **DB tests** use `sqlite::memory:` for speed. Apply the right migration set:
  - `SYSTEM_MIGRATIONS` for system-DB tests.
  - `REALM_MIGRATIONS` for realm-DB tests (apps + admin tiers + policies + audit).
  - `APP_MIGRATIONS` for app-DB tests (collections, records, users, OAuth, …).
- **Property tests** live in `tests/` of each crate where they exist (notably
  the auto-clamp engine in `rustbase-db`).

When adding new behavior, prefer a failing test first — the codebase has a
strong test-driven habit and reviews lean on it.

## Reporting bugs / requesting features

Use the issue templates:

- 🐛 [Bug report](https://github.com/pjonaszik/rustbase/issues/new?template=bug_report.yml)
- 💡 [Feature request](https://github.com/pjonaszik/rustbase/issues/new?template=feature_request.yml)

For open-ended questions and design discussions, prefer
[GitHub Discussions](https://github.com/pjonaszik/rustbase/discussions).

For security issues, **do not file a public issue** — see [SECURITY.md](SECURITY.md).

## Release process (maintainers)

Releases are driven by [**release-please**](https://github.com/googleapis/release-please).
Conventional-commit messages on `main` (`feat:`, `fix:`, `perf:`, etc.)
accumulate; release-please opens an auto-updated **`release: x.y.z`** PR.

When you're ready to ship:

1. Review the open release-please PR. Edit the body / CHANGELOG if you want
   to add context the commits didn't capture.
2. Merge the PR. release-please:
   - bumps `workspace.package.version` in `Cargo.toml`,
   - moves the `[Unreleased]` content into a new dated section in `CHANGELOG.md`,
   - pushes the tag `vX.Y.Z`,
   - publishes a GitHub Release at that tag.
3. The `release` workflow fires on the `release: published` event:
   - cross-compiles `linux-x86_64-musl`, `linux-x86_64-gnu`, `macos-arm64` binaries,
   - builds + pushes the multi-arch Docker image to `ghcr.io/pjonaszik/rustbase`,
   - attaches the binaries + sha256 sums to the release.

### Cutting a release without release-please

Ad-hoc releases (security fixes, repository surgery) can be cut manually:

1. Bump `workspace.package.version` in `Cargo.toml`.
2. Move `## [Unreleased]` entries into `## [vX.Y.Z] — YYYY-MM-DD` in CHANGELOG.
3. `git commit -m "release: vX.Y.Z"` on `main`.
4. `git tag -s vX.Y.Z -m "vX.Y.Z"` (signed tag) and push.
5. The `release` workflow fires on the tag push and produces the same artefacts.

Manual cuts produce **signed** tags. release-please tags are unsigned —
that's an acceptable tradeoff for the convenience of automation; the
commits the tag points at are still verified.
