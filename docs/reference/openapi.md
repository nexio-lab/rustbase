# OpenAPI spec

RustBase ships an OpenAPI 3.1 spec covering the SDK-facing slice of the API — health, end-user auth, records CRUD, and files. The same document is served by the binary at runtime so client codegen has a single source of truth.

## Where to find it

- **Live**, served by your running RustBase instance: `GET /openapi.yaml` (no auth — it's a spec). Replace `localhost:8080` with your host:

```sh
curl -s http://localhost:8080/openapi.yaml > openapi.yaml
```

- **Source**, on GitHub: [`docs/reference/openapi.yaml`](https://github.com/pjonaszik/rustbase/blob/main/docs/reference/openapi.yaml). The file under `docs/reference/` is embedded into the binary via `include_str!`, so the served document and the GitHub source are always the same string.

## Scope today

The spec covers the **end-user** surface — the operations a typical client app needs:

| Tag | Operations |
|---|---|
| Server | `getHealth`, `getOpenApiSpec` |
| Auth | `userRegister`, `userLogin`, `userLoginTotp`, `userRefresh`, `userLogout`, `verificationRequest`, `verificationConfirm` |
| Records | `listRecords`, `createRecord`, `getRecord`, `updateRecord`, `deleteRecord` |
| Files | `uploadFile`, `serveFile` |

Admin routes (`/_/auth/admin/login`, workspace management, OAuth provider config, hierarchical policies, hook reload, audit, schema patch) are intentionally **not** in this spec yet. They will land as the API surface grows utoipa annotations.

## How it's authored

The spec is hand-authored under `docs/reference/openapi.yaml`. This trades some drift risk for ergonomic editing and a single-pass migration when utoipa goes in.

To keep the spec honest:

1. Every recipe in `cookbook/` exercises a path the spec documents — the curl snippets there are reality checks.
2. The smoke test (`ui/tests/e2e/smoke.spec.ts`) walks workspace → app → collection creation through the dashboard, which calls the same handlers the spec documents.
3. PRs that change a request or response shape are expected to update the YAML in the same diff.

A future pass will replace this with utoipa-generated output. Until then, this file is the **canonical** SDK input.

## Generating a client

Most OpenAPI generators accept this file directly. For the bundled JS / TS SDK:

```sh
bun create @rustbase/client@latest
# or, manual:
bunx openapi-typescript http://localhost:8080/openapi.yaml -o ./openapi.d.ts
```

The Dart and Go SDKs follow the same pattern.

## Versioning

The spec carries the running RustBase version in `info.version`. Breaking changes to a request or response shape will bump the major component; additive changes (new optional field, new operation) will not.
