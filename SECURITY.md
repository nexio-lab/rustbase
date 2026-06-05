# Security policy

## Reporting a vulnerability

**Do not open a public issue for a security report.**

Use GitHub's private vulnerability reporting:

1. Go to <https://github.com/pjonaszik/rustbase/security/advisories/new>.
2. Fill in the form. A maintainer receives the report; nothing is public until
   we publish an advisory.

If GitHub's flow is unreachable for you, you can also fall back to opening a
draft GitHub Discussion in a private repo, or reaching out to a maintainer
through their public profile. Avoid sending exploit details over public
channels.

## What we treat as a security issue

- Auth bypass (master / workspace / app / end-user).
- Path traversal in hooks, files, or storage.
- SQL injection (every dynamic clause should already be parameterized — proof
  of a missed spot is a security issue).
- Secret leakage (e.g. echoing back an OAuth client secret or a refresh token
  hash through any endpoint).
- Sandbox escape from the JS/TS hook runtime.
- Privilege escalation (user → admin, app-admin → workspace-admin, etc.).

Bugs that are *not* security issues:

- Crashes on malformed input that only DoS the requester.
- Reproducible 5xx that don't leak data.
- Performance / DoS amplification against a single tenant.

These are still appreciated as regular bug reports.

## Supported versions

Until v1.0, only the latest minor receives security fixes. Once v1.0 ships,
the policy will be documented here explicitly.

## Disclosure timeline

When a report is accepted:

1. Acknowledgement within ~7 days.
2. A private fix branch + reviewer.
3. Coordinated release of a patched version + GitHub Security Advisory
   (CVE requested when the impact warrants).
4. Public credit to the reporter (unless they prefer anonymity).

## Verifying release artefacts

Every release tarball, the container image, and the SBOM are
**signed with [Sigstore Cosign](https://docs.sigstore.dev/cosign/overview/)**
keylessly — the workflow's GitHub OIDC identity (instead of a long-lived
maintainer key) is what proves provenance.

### Tarballs

```sh
TARBALL=rustbase-v0.X.Y-linux-x86_64-musl.tar.gz
cosign verify-blob \
    --certificate $TARBALL.pem \
    --signature   $TARBALL.sig \
    --certificate-identity-regexp 'https://github.com/pjonaszik/rustbase/.*' \
    --certificate-oidc-issuer     'https://token.actions.githubusercontent.com' \
    $TARBALL
```

### Container image

```sh
cosign verify \
    --certificate-identity-regexp 'https://github.com/pjonaszik/rustbase/.*' \
    --certificate-oidc-issuer     'https://token.actions.githubusercontent.com' \
    ghcr.io/pjonaszik/rustbase:v0.X.Y
```

### SBOM

CycloneDX 1.5 JSON, one document per release, sits next to the tarballs
as `rustbase-v0.X.Y-sbom.cdx.json` (signature + cert as `.sig` /
`.pem` siblings). Verify the same way as a tarball.

## Static analysis & scanning

- **clippy** runs with `-D warnings` on every PR.
- **`cargo audit`** scans the Rust dep tree against the RustSec advisory
  DB on every PR.
- **Trivy** scans the workspace filesystem (Rust + Bun lockfiles,
  Dockerfile mis-configs, accidentally committed secrets) on every PR
  and the released container image post-publish; findings land on the
  GitHub Security tab.
- **CodeQL** analyses the dashboard's JavaScript / TypeScript / Svelte
  source on every push to `main`, every PR, and weekly.
- **Dependabot** keeps Rust, GitHub Actions, and Bun deps moving.
