# Contributing to RustBase

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

- **Rust ≥ 1.88** (stable). Install via [`rustup`](https://rustup.rs/).
- **Bun** for the dashboard / docs (`curl -fsSL https://bun.sh/install | bash`).
- **Docker** if you want to exercise the optional MailHog integration test
  (`infra/docker-compose.yml`).

Clone, then one command bootstraps the rest:

```sh
git clone git@github.com:pjonaszik/rustbase.git
cd rustbase
make setup-dev
```

`make setup-dev` (a.k.a. `scripts/setup-dev.sh`) checks that the right
toolchain is present (Rust ≥ 1.88, Bun, Python 3, git; Docker if you
want the local image target), installs the git hooks, then warms the
Cargo and Bun caches so the first `make build` doesn't have to download
the world.

The hooks set `core.hooksPath` to `.githooks/`. The `pre-commit` hook
runs `cargo fmt --check`, `cargo clippy -- -D warnings`, an architectural
grep suite, and a no-AI-attribution scan on the staged diff. The
`pre-push` hook runs the full test suite (and `cargo audit` if it's
installed).

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
  identifiers, trailers. RustBase is a human-maintained project.
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

### Writing the CHANGELOG

A polished release entry explains **why** each change happened — the
incident that surfaced the bug, the constraint that drove the refactor,
the tradeoff retained. That texture matters more than completeness; a
cold list of `type(scope): subject` bullets doesn't tell anyone
upgrading what they should pay attention to.

The expected flow is therefore:

1. As you work, **write the `## [Unreleased]` block by hand** —
   grouped into Added / Changed / Fixed / Removed / Security / Build,
   one or two sentences per entry, mention issue/PR numbers when
   relevant. Anchor the entry on the *why* rather than the *what*.
2. If you don't want to draft this yourself every cycle, you can
   delegate to a contributor with full context (e.g. the assistant
   you've been pairing with), then paste the result into `[Unreleased]`.
3. If you genuinely need an autogenerated starting point — say, after a
   long stretch of mechanical Dependabot bumps — run `make changelog`.
   It synthesises `[Unreleased]` from the conventional-commit messages
   since the last tag, bucketed by type. **It is meant as a draft you
   then polish**, not as the final entry.

### Cutting the release

Once `## [Unreleased]` reads the way you want it to:

```sh
make release V=0.1.2
```

`scripts/release.sh` then:

1. runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`
   (the same gate CI enforces) — aborts on any failure;
2. **respects the hand-written `[Unreleased]`** unless you pass
   `REGEN_CHANGELOG=1`;
3. bumps `workspace.package.version` in `Cargo.toml`;
4. rotates `## [Unreleased]` → `## [vX.Y.Z] — YYYY-MM-DD` and seeds a
   fresh empty `[Unreleased]`;
5. refreshes `Cargo.lock` via `cargo check`;
6. commits as `release: vX.Y.Z`;
7. creates a **signed** tag `vX.Y.Z` (uses the SSH signing key
   configured on the repo).

Review the commit, then push:

```sh
make release-push V=0.1.2
```

The push triggers `.github/workflows/release.yml`:

- cross-compiles `linux-x86_64-musl`, `linux-x86_64-gnu`, `macos-arm64` binaries,
- builds + pushes the multi-arch Docker image to `ghcr.io/pjonaszik/rustbase`,
- creates the GitHub Release with the binaries + sha256 sums attached.

If you change your mind before pushing, the script tells you exactly how
to revert: `git tag -d vX.Y.Z && git reset --hard HEAD~1`.
