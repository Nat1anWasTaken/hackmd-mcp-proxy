# Agent Instructions

@/Users/nathan/.codex/RTK.md

## Source of Truth

`SPEC.md` is the authoritative implementation contract for this repository. Read it before making architectural or behavioral changes.

This repository is for a HackMD MCP OAuth Wrapper:

- Rust Axum backend.
- Vite React TypeScript frontend.
- PostgreSQL with SQLx.
- OAuth 2.1 authorization-code flow with PKCE.
- Dynamic Client Registration for ChatGPT.
- Streamable HTTP MCP proxy to `https://mcp.hackmd.io`.
- Per-user encrypted HackMD API token vault.

If this file conflicts with `SPEC.md`, follow `SPEC.md` unless the user explicitly asks to change the spec.

## Local Command Rule

Prefix shell commands with `rtk` as required by `/Users/nathan/.codex/RTK.md`.

Examples:

```bash
rtk cargo test
rtk pnpm test
rtk git status --short
```

Use `rtk proxy <cmd>` only when the raw command output is required and filtering would hide important information.

## Implementation Priority

Build in the phase order from `SPEC.md`:

1. Phase 0: Rust Axum server, `/health`, raw `/mcp` proxy using `HACKMD_API_TOKEN` from the environment, validate with MCP Inspector.
2. Phase 1: Correct streaming proxy behavior, header filtering, MCP session header preservation, basic integration tests.
3. Phase 2: OAuth metadata, DCR, `/authorize`, `/token`, PKCE S256, opaque access tokens, protect `/mcp`.
4. Phase 3: Vite React UI for login, connect, consent, settings, plus CSRF protection.
5. Phase 4: PostgreSQL migrations, SQLx repositories, encrypted per-user HackMD token storage and verification.
6. Phase 5: `tools/call` policy guard for scope and access mode enforcement.
7. Phase 6: MCP session binding and `DELETE /mcp`.
8. Phase 7: Rate limiting, revocation, structured redacted logs, deployment config, broader integration tests.

Do not skip ahead to a later phase if an earlier phase is missing unless the user specifically requests it.

## Security Invariants

Treat HackMD API tokens as high privilege credentials.

Never:

- Store HackMD API tokens in plaintext.
- Log HackMD API tokens.
- Return HackMD API tokens to the frontend.
- Place HackMD API tokens in URL query strings.
- Store HackMD API tokens or session tokens in `localStorage`.
- Forward ChatGPT OAuth access tokens to HackMD.
- Log full MCP request bodies, response bodies, note Markdown, OAuth tokens, authorization codes, magic link tokens, or session cookies.

Always:

- Encrypt HackMD API tokens at rest.
- Use HMAC hashes for opaque OAuth tokens, authorization codes, web sessions, magic links, and upstream MCP session IDs.
- Replace inbound `Authorization` with the user's HackMD token only on the upstream HackMD request.
- Fail closed on auth errors.
- Fail closed on MCP session mismatch.
- Bind MCP session IDs to both `user_id` and `client_id`.
- Keep delete actions blocked unless the user has `full_access` and the OAuth token has `hackmd.delete`.
- Preserve OAuth `state` exactly as provided by the client.
- Use a separate server-side CSRF token for frontend POST routes.

## OAuth Requirements

MVP supports:

- Authorization code flow.
- PKCE S256.
- Dynamic Client Registration.
- Public client token exchange with `token_endpoint_auth_method = none`.
- Opaque bearer access tokens.
- No refresh token.

Required OAuth endpoints:

```text
GET  /.well-known/oauth-protected-resource
GET  /.well-known/oauth-authorization-server
POST /register
GET  /authorize
POST /token
POST /revoke
```

Validate redirect URIs during DCR. Allow ChatGPT connector redirect URIs from `SPEC.md`; reject others by default.

Every `/mcp` request must validate token existence, expiration, revocation, resource, scopes, user existence, and HackMD credential existence.

## MCP Proxy Requirements

`/mcp` must support:

```text
POST /mcp
GET /mcp
DELETE /mcp
```

Proxy behavior:

- Preserve pass-through behavior over rewriting.
- Do not buffer SSE streams.
- Read POST body bytes only to inspect the JSON-RPC envelope for policy checks.
- Forward the original body bytes upstream after checks.
- Do not log or persist request or response bodies.
- Preserve `MCP-Session-Id` and `MCP-Protocol-Version`.
- Store only a hash of upstream `MCP-Session-Id`.

Preserve request headers:

```text
Accept
Content-Type
MCP-Protocol-Version
MCP-Session-Id
Last-Event-ID
User-Agent
```

Strip request headers:

```text
Authorization
Cookie
Host
X-Forwarded-For
X-Real-IP
Forwarded
CF-Connecting-IP
Fly-Client-IP
```

Preserve response headers:

```text
Content-Type
MCP-Session-Id
MCP-Protocol-Version
Cache-Control
```

Strip or recalculate response headers:

```text
Set-Cookie
Server
Alt-Svc
Transfer-Encoding
Content-Encoding
Content-Length
```

## Policy Guard

For JSON-RPC requests where `method == "tools/call"`, parse `params.name` as `tool_name`.

Read tools require:

- OAuth scope `hackmd.read`.
- User mode not disconnected.

Write tools require:

- OAuth scope `hackmd.write`.
- User mode `read_write_no_delete` or `full_access`.

Delete tools require:

- OAuth scope `hackmd.delete`.
- User mode `full_access`.

Unknown tools are allowed by default for MVP and must be audit logged. If `STRICT_TOOL_ALLOWLIST=true`, block unknown tools.

## Frontend Requirements

The frontend is a small operational UI. Keep it quiet, direct, and task-focused.

Required pages:

- `/login`
- `/connect`
- `/oauth/consent`
- `/settings`

Use shadcn/ui components where appropriate. Do not build a marketing landing page unless the user asks for one.

The UI must never display a full HackMD API token. Show only account metadata, last verification time, current access mode, and token fingerprint.

## Database Requirements

Implement the schema from `SPEC.md` unless the user approves a schema change. Core tables:

- `users`
- `web_sessions`
- `magic_links`
- `hackmd_credentials`
- `oauth_clients`
- `oauth_authorization_requests`
- `oauth_authorization_codes`
- `oauth_access_tokens`
- `mcp_sessions`
- `audit_logs`

Use SQLx migrations. Prefer repository functions over ad hoc SQL spread across route handlers.

## Logging and Audit

Allowed log fields include:

- `request_id`
- `user_id`
- `client_id`
- `tool_name`
- `method`
- `status_code`
- `duration_ms`
- `upstream_status`
- `error_code`
- `access_mode`
- `scope_check_result`

Audit important security events listed in `SPEC.md`, including login results, connect and disconnect events, OAuth approval and denial, token issuance and revocation, MCP tool allow or block decisions, session creation, session mismatch, and upstream auth failures.

## Testing Expectations

Add tests in proportion to the phase being implemented. Security and protocol behavior should have focused tests.

Prioritize tests for:

- PKCE S256.
- Redirect URI allowlist.
- Scope parsing and matching.
- Opaque token hashing.
- Authorization code single-use behavior.
- Token encryption and decryption.
- Header filtering.
- JSON-RPC envelope parsing.
- Tool policy decisions.
- MCP session hash binding.
- Streaming pass-through.

Do not rely on a real HackMD API token in normal automated tests. Use mocks or test fixtures unless the user provides a staging token and explicitly asks for live verification.

## Engineering Style

- Keep behavior aligned with `SPEC.md`.
- Prefer simple modules that match the planned layout in `README.md` and `SPEC.md`.
- Avoid broad abstractions before the phase needs them.
- Keep route handlers thin when possible; put OAuth, crypto, DB, and proxy behavior in dedicated modules.
- Keep proxy behavior transparent unless security or MCP compatibility requires intervention.
- Use structured errors and avoid leaking secrets in error messages.
- Document meaningful deviations from `SPEC.md` in the README or an ADR-style note.

