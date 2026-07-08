# Goal Progress

Objective: fully implement the HackMD MCP OAuth Wrapper described in `SPEC.md`.

## Milestone Checklist

- [x] Stage 0: Create the Rust workspace, Axum server shell, health endpoint, configuration, observability, Docker/dev config, and baseline tests.
- [ ] Stage 1: Implement Streamable HTTP `/mcp` proxy correctness with request/response header filtering, streaming passthrough, upstream timeout handling, and local-token proof-of-concept support.
- [ ] Stage 2: Implement OAuth protected-resource metadata, authorization-server metadata, Dynamic Client Registration, authorization-code + PKCE, opaque bearer tokens, revocation, and `/mcp` bearer validation.
- [ ] Stage 3: Implement PostgreSQL schema, SQLx repositories, encrypted HackMD token vault, token fingerprinting, web sessions, single-user login, CSRF, HackMD token verification, connection/settings APIs, disconnect behavior, and audit logs.
- [ ] Stage 4: Implement policy guard for JSON-RPC `tools/call`, scope checks, access-mode checks, default delete blocking, strict allowlist option, and policy tests.
- [ ] Stage 5: Implement MCP session binding for upstream `MCP-Session-Id`, GET/POST validation, DELETE session teardown, and mismatch handling.
- [ ] Stage 6: Create the Vite React TypeScript UI with shadcn-style components for login, connect, OAuth consent, success, settings, API client, schemas, and production build integration.
- [ ] Stage 7: Add production hardening: rate limits, security headers/origin validation, redacted structured logs, README/deployment documentation, and final validation.

## Relevant Codebase Areas

- Root project config: Cargo workspace, pnpm workspace, Docker Compose, README, validation config.
- `apps/server`: Rust Axum backend, SQLx migrations, OAuth provider, web auth/session routes, HackMD credential vault, MCP proxy, tests.
- `apps/web`: Vite React frontend, Tailwind/shadcn-style UI, route screens, API client, form schemas.

The repository currently contains only `SPEC.md` and an empty `README.md`; there are no existing package configs, source files, tests, or framework conventions beyond the structure prescribed by `SPEC.md`.

## Intended Implementation Approach

- Follow the `SPEC.md` directory layout rather than inventing a different architecture.
- Keep server modules aligned to the spec: `routes`, `oauth`, `auth`, `hackmd`, `crypto`, `db`, and `observability`.
- Use SQLx runtime queries rather than compile-time macros so local validation does not require a live database at build time.
- Use small route handlers backed by repository/domain modules; keep business logic out of frontend components.
- Use encrypted BYOK storage with XChaCha20-Poly1305, HMAC fingerprints, and opaque OAuth/session tokens stored only as HMAC hashes.
- Preserve upstream MCP bodies and streaming behavior while inspecting only minimal JSON-RPC envelopes needed for policy.
- Use focused unit/integration tests per stage and run the smallest relevant validation after each coherent change.

## Assumptions

- `PUBLIC_BASE_URL` must be HTTPS in production; local development may use `http://127.0.0.1`.
- MVP uses single-user login via `OWNER_INVITE_CODE` by default, while retaining magic-link table/schema for future email delivery.
- Real HackMD token verification is implemented through upstream MCP `initialize` plus `tools/call get-me`; tests use a mock upstream.
- Frontend uses shadcn-compatible local components rather than running the shadcn generator, keeping files explicit and reviewable.
- Integration tests avoid requiring a real PostgreSQL or HackMD account unless corresponding environment variables are provided.

## Validation Commands

- Server: `rtk cargo fmt --all --check`
- Server: `rtk cargo test --workspace`
- Server: `rtk cargo clippy --workspace --all-targets -- -D warnings`
- Web: `rtk pnpm --dir apps/web install --frozen-lockfile`
- Web: `rtk pnpm --dir apps/web lint`
- Web: `rtk pnpm --dir apps/web typecheck`
- Web: `rtk pnpm --dir apps/web build`
- Full: run all commands above before marking the goal complete.

## Maintainability Risks To Watch

- OAuth route logic growing too large instead of staying in OAuth modules.
- Proxy behavior accidentally buffering SSE responses.
- Header filtering missing sensitive hop-by-hop or credential headers.
- Token or note content appearing in logs, errors, frontend responses, or tests.
- Scope/access-mode policy diverging from the HackMD tool map.
- Web UI forms duplicating backend enum/schema behavior.
- SQLx repository code becoming coupled to Axum extractors.

## Commit Boundaries

- Stage 0 commit: project scaffold, config, health endpoint, progress tracker, baseline docs/tests.
- Stage 1 commit: MCP proxy transport correctness and tests.
- Stage 2 commit: OAuth provider and bearer-token enforcement.
- Stage 3 commit: database, sessions, token vault, connection/settings APIs.
- Stage 4 commit: policy guard and audit behavior.
- Stage 5 commit: MCP session binding and DELETE support.
- Stage 6 commit: frontend UI and build integration.
- Stage 7 commit: hardening, docs, final validation updates.

## Validation Log

- Stage 0:
  - `rtk cargo fmt --all --check` -> passed
  - `rtk cargo test --workspace` -> passed, 1 test
  - `rtk cargo clippy --workspace --all-targets -- -D warnings` -> passed

## Commits

- Pending.

## Remaining Follow-Up

- Pending until implementation stages begin.
