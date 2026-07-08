# HackMD MCP OAuth Wrapper 規格書

目標：建立一個可被 ChatGPT Web 連接的 OAuth-powered remote MCP server，內部直接 proxy HackMD 官方 MCP，並提供 UI 讓每位使用者提交自己的 HackMD API key。

## 1. 背景與已確認事實

HackMD 官方已提供遠端 MCP server，端點為 `https://mcp.hackmd.io`，傳輸協定為 Streamable HTTP，驗證方式為 `Authorization: Bearer YOUR_API_TOKEN`。HackMD 文件也提醒，API token 可用來讀寫 HackMD 筆記，因此必須視為高權限憑證處理。

ChatGPT Developer Mode 支援 remote MCP，支援的 MCP protocols 包含 SSE 與 streaming HTTP，authentication 支援 OAuth、No Authentication、Mixed Authentication。

ChatGPT Apps SDK / MCP auth 流程中，authenticated MCP server 預期實作 OAuth 2.1 authorization-code flow + PKCE。ChatGPT 會在 OAuth 完成後，將 access token 放在後續 MCP request 的 `Authorization: Bearer <token>` header 中；MCP server 必須自行驗證 token、resource / audience、expiration 與 scopes。

MCP Streamable HTTP transport 要求 MCP endpoint 支援 POST 與 GET；POST request 必須帶 `Accept: application/json, text/event-stream`，server 可回傳 JSON 或 SSE stream。若 server 在 initialize response 回傳 `MCP-Session-Id`，client 後續 request 必須帶回同一個 session id。

本專案採用 Rust backend，推薦使用 Axum，因為 Axum 是 Rust HTTP routing / request-handling library，並設計為與 Tokio、Hyper 生態搭配。 Frontend 採用 Vite + React + shadcn/ui，shadcn/ui 官方提供 Vite 安裝路徑。 Database 使用 PostgreSQL + SQLx；SQLx 支援 Tokio runtime，適合此專案的 async Rust server 架構。

## 2. 產品目標

建立一個 Hosted MCP Wrapper，讓使用者可以在 ChatGPT Web 中新增此 MCP connector，完成 OAuth 授權，並在授權流程或設定頁中貼上自己的 HackMD API key。

之後 ChatGPT 對 wrapper 的所有 MCP request，都由 wrapper 驗證 ChatGPT OAuth access token，再把 request proxy 到 HackMD 官方 MCP，並用該使用者的 HackMD API key 取代 Authorization header。

核心資料流：

```text
ChatGPT Web
  → OAuth / MCP request
  → Rust Axum HackMD MCP OAuth Wrapper
  → verify wrapper OAuth access token
  → decrypt user HackMD API key
  → proxy request to https://mcp.hackmd.io
  → stream upstream response back to ChatGPT
```

## 3. 非目標

第一版不重做 HackMD API tools。

第一版不實作 HackMD OAuth，因為目標是 BYOK：Bring Your Own Key，讓使用者自行提交 HackMD API key。

第一版不做筆記同步、快取、搜尋索引、版本管理或多人協作管理。

第一版不修改 HackMD MCP 的核心工具行為，只做必要的 auth translation、security guard、session binding 與 proxy compatibility。

第一版不需要 ChatGPT iframe widget。Vite React frontend 是 wrapper 自己的設定 UI，不是 ChatGPT Apps SDK widget。

## 4. 推薦技術棧

### Backend

```text
Language:
  Rust

Web framework:
  axum

Async runtime:
  tokio

HTTP client:
  reqwest 或 hyper

Database:
  PostgreSQL

Database client:
  sqlx

Crypto:
  chacha20poly1305 或 aes-gcm
  hmac
  sha2
  rand

OAuth / token:
  自行實作窄範圍 OAuth provider
  opaque access tokens stored hashed in PostgreSQL
  PKCE S256
  Dynamic Client Registration, DCR

Middleware:
  tower
  tower-http

Observability:
  tracing
  tracing-subscriber
  opentelemetry, optional

Config:
  figment 或 envy
  dotenvy for local development

Testing:
  cargo test
  integration tests with testcontainers
  MCP Inspector
  ChatGPT Developer Mode
```

### Frontend

```text
Framework:
  Vite
  React
  TypeScript

UI:
  shadcn/ui
  Tailwind CSS
  Radix UI primitives

Forms:
  react-hook-form
  zod

Data fetching:
  TanStack Query

Routing:
  TanStack Router 或 React Router

Build:
  pnpm
```

### Deployment

```text
Container:
  Docker

Runtime target:
  Fly.io / Render / Railway / VPS / Kubernetes

Database:
  Managed PostgreSQL

TLS:
  必須使用 HTTPS
  可由 reverse proxy、load balancer 或 platform TLS 終止

Local development:
  docker-compose
  ngrok / cloudflared tunnel / localhost.run for ChatGPT testing
```

## 5. 系統架構

```text
hackmd-mcp-oauth-wrapper/
  apps/
    server/
      Cargo.toml
      migrations/
      src/
        main.rs
        config.rs
        state.rs

        routes/
          mod.rs
          health.rs
          oauth.rs
          auth_session.rs
          connect.rs
          settings.rs
          mcp.rs
          static_assets.rs

        oauth/
          mod.rs
          metadata.rs
          dcr.rs
          authorize.rs
          token.rs
          pkce.rs
          scopes.rs
          clients.rs
          grants.rs

        auth/
          mod.rs
          web_session.rs
          magic_link.rs
          csrf.rs

        hackmd/
          mod.rs
          validate_token.rs
          proxy.rs
          policy.rs
          sessions.rs

        crypto/
          mod.rs
          token_vault.rs
          fingerprint.rs
          random.rs

        db/
          mod.rs
          models.rs
          repositories.rs

        observability/
          mod.rs
          logger.rs
          redaction.rs

    web/
      package.json
      vite.config.ts
      components.json
      src/
        main.tsx
        app.tsx
        routes/
          login.tsx
          connect.tsx
          settings.tsx
          consent.tsx
          success.tsx
        components/
          ui/
          app-shell.tsx
          token-form.tsx
          access-mode-select.tsx
          consent-card.tsx
        lib/
          api.ts
          schemas.ts
          query-client.ts
```

## 6. Backend HTTP endpoints

### 6.1 Public metadata

```text
GET /.well-known/oauth-protected-resource
GET /.well-known/oauth-authorization-server
GET /health
```

### 6.2 OAuth

```text
POST /register
GET  /authorize
POST /token
POST /revoke
```

### 6.3 Web auth session

```text
GET  /api/session
POST /api/auth/magic/start
GET  /api/auth/magic/callback
POST /api/logout
```

MVP 可以先用 email magic link。若第一版只給自己使用，可先做 `SINGLE_USER_MODE=true`，以一組 admin password 或 invite code 登入。

### 6.4 OAuth consent flow API

```text
GET  /api/oauth/authorization-requests/:id
POST /api/oauth/authorization-requests/:id/approve
POST /api/oauth/authorization-requests/:id/deny
```

### 6.5 HackMD connection UI API

```text
GET  /api/hackmd/connection
POST /api/hackmd/token
POST /api/hackmd/token/verify
POST /api/hackmd/disconnect
GET  /api/settings
POST /api/settings/access-mode
```

### 6.6 MCP proxy

```text
POST   /mcp
GET    /mcp
DELETE /mcp
```

`/mcp` 是唯一對 ChatGPT 暴露的 MCP endpoint。

## 7. 使用者流程

### 7.1 首次連接

1. 使用者在 ChatGPT Web 新增此 remote MCP connector。
2. ChatGPT 讀取 `/.well-known/oauth-protected-resource`。
3. ChatGPT 讀取 `/.well-known/oauth-authorization-server`。
4. 若使用 DCR，ChatGPT 呼叫 `POST /register` 取得 `client_id`。
5. ChatGPT 啟動 OAuth authorization-code + PKCE flow，導向 `GET /authorize`。
6. Rust backend 驗證 OAuth request params，建立一筆 `oauth_authorization_requests`。
7. 如果使用者尚未登入 wrapper，backend 導向 frontend `/login`。
8. 使用者用 magic link 或 single-user login 登入。
9. 如果使用者尚未設定 HackMD API key，frontend 導向 `/connect`。
10. 使用者貼上 HackMD API key，選擇 access mode。
11. Backend 呼叫 HackMD MCP 驗證 token，例如透過 initialize + `get-me`。
12. 驗證成功後，backend 加密 HackMD API key，存入 PostgreSQL。
13. Frontend 顯示 consent page，列出 ChatGPT 要求的 scopes。
14. 使用者同意。
15. Backend 產生 authorization code，redirect 回 ChatGPT 的 redirect URI。
16. ChatGPT 呼叫 `POST /token`，用 code + code_verifier 換 access token。
17. ChatGPT 後續呼叫 `/mcp` 時帶上 `Authorization: Bearer <wrapper_access_token>`。
18. Backend 驗證 access token，解密該使用者的 HackMD token，proxy 到 `https://mcp.hackmd.io`。

### 7.2 後續連接

如果使用者已有有效 web session 且已設定 HackMD credential，`/authorize` 可以直接顯示 consent page。

如果使用者已同意相同 client + scopes，可以選擇自動 approve；MVP 建議仍顯示 consent，以便使用者理解 ChatGPT 將透過此 wrapper 存取 HackMD。

### 7.3 更新 HackMD API key

1. 使用者進入 `/settings`。
2. 使用者貼上新的 HackMD API key。
3. Backend 驗證新 key。
4. 驗證成功後覆寫舊 credential。
5. 舊 token 不保留。

### 7.4 斷開連接

1. 使用者點擊 Disconnect。
2. Backend 刪除加密 HackMD API key。
3. Backend revoke 使用者相關 OAuth access tokens。
4. Backend 刪除或標記 MCP session mappings。
5. 後續 `/mcp` request 回傳 401 或 setup-required 錯誤。

## 8. OAuth 設計

### 8.1 Supported OAuth flow

MVP 支援：

```text
Authorization Code Flow
PKCE S256
Dynamic Client Registration, DCR
Public client token exchange: token_endpoint_auth_method = none
Opaque access tokens
No refresh token
```

暫不支援：

```text
client_credentials
password grant
device code flow
JWT bearer grant
machine-to-machine OAuth
custom API key auth from ChatGPT
```

ChatGPT 不會替使用者直接提交 HackMD API key；HackMD API key 必須只存在於 wrapper backend 的 encrypted token vault 中。

### 8.2 Protected resource metadata

`GET /.well-known/oauth-protected-resource` 回傳：

```json
{
  "resource": "https://your-domain.example/mcp",
  "authorization_servers": ["https://your-domain.example"],
  "scopes_supported": [
    "hackmd.read",
    "hackmd.write",
    "hackmd.delete"
  ],
  "resource_documentation": "https://your-domain.example/docs"
}
```

如果 `/mcp` 收到未授權 request，回傳：

```http
HTTP/1.1 401 Unauthorized
WWW-Authenticate: Bearer resource_metadata="https://your-domain.example/.well-known/oauth-protected-resource"
```

### 8.3 Authorization server metadata

`GET /.well-known/oauth-authorization-server` 回傳：

```json
{
  "issuer": "https://your-domain.example",
  "authorization_endpoint": "https://your-domain.example/authorize",
  "token_endpoint": "https://your-domain.example/token",
  "registration_endpoint": "https://your-domain.example/register",
  "revocation_endpoint": "https://your-domain.example/revoke",
  "response_types_supported": ["code"],
  "grant_types_supported": ["authorization_code"],
  "code_challenge_methods_supported": ["S256"],
  "token_endpoint_auth_methods_supported": ["none"],
  "scopes_supported": [
    "hackmd.read",
    "hackmd.write",
    "hackmd.delete"
  ]
}
```

### 8.4 Redirect URI policy

DCR 時必須驗證 client metadata 中的 redirect URI。

允許：

```text
https://chatgpt.com/connector/oauth/*
```

可選擇保留 legacy support：

```text
https://chatgpt.com/connector_platform_oauth_redirect
```

其他 redirect URI 預設拒絕。

### 8.5 Scopes

```text
hackmd.read
  允許讀取個人筆記、團隊資訊、團隊筆記、搜尋結果與使用者資訊。

hackmd.write
  允許建立與更新個人 / 團隊筆記。

hackmd.delete
  允許刪除個人 / 團隊筆記。

offline_access
  MVP 不支援。
```

### 8.6 Scope 與 HackMD tools 對應

```text
hackmd.read:
  get-me
  list-notes
  get-note
  get-history
  list-teams
  get-team
  list-team-notes
  get-team-note
  search-notes
  resources/read

hackmd.write:
  create-note
  update-note
  create-team-note
  update-team-note

hackmd.delete:
  delete-note
  delete-team-note
```

### 8.7 Access token format

MVP 使用 opaque bearer token，而不是 JWT。

理由：

```text
- 容易 revoke
- 不需要 JWKS
- 不需要處理 key rotation
- server 與 authorization server 是同一個 Rust backend
- ChatGPT 不需要解析 token，只需要原樣帶回
```

Access token 產生方式：

```text
access_token = base64url(random 32 bytes)
stored_hash = HMAC-SHA256(access_token, ACCESS_TOKEN_HASH_KEY)
```

`oauth_access_tokens` table 儲存：

```text
token_hash
user_id
client_id
resource
scopes
expires_at
revoked_at
```

每次 `/mcp` request 必須檢查：

```text
access token exists
not expired
not revoked
resource == https://your-domain.example/mcp
required scope exists
user exists
HackMD credential exists
```

### 8.8 Authorization code

Authorization code 產生方式：

```text
code = base64url(random 32 bytes)
code_hash = HMAC-SHA256(code, AUTH_CODE_HASH_KEY)
```

Authorization code 必須：

```text
single-use
short-lived, e.g. 5 minutes
bound to client_id
bound to redirect_uri
bound to resource
bound to code_challenge
bound to code_challenge_method = S256
```

### 8.9 State handling

OAuth `state` 是 ChatGPT client 提供的 opaque value。Backend 不應修改它，也不應把它當作 server-side CSRF token。

Backend 應：

```text
保存 state
redirect 回 ChatGPT 時原樣帶回 state
另外使用自己的 CSRF token 保護 consent form 與 UI POST request
```

## 9. Web login / 使用者身份

### 9.1 MVP login method

MVP 推薦 email magic link：

```text
使用者輸入 email
backend 產生短效 magic link token
token hash 存 PostgreSQL
寄送 magic link
使用者點擊 callback
backend 建立 web session
```

### 9.2 Single-user mode

為了加速個人部署，允許：

```text
SINGLE_USER_MODE=true
```

在此模式下：

```text
- 不開放 public signup
- 只有一個 owner user
- 可用 OWNER_INVITE_CODE 或 ADMIN_PASSWORD 建立 session
- 仍然保留 OAuth / HackMD token vault / policy guard
```

### 9.3 Session cookie

Web session 使用 cookie：

```text
HttpOnly
Secure
SameSite=Lax
Path=/
```

不可使用 localStorage 儲存 session token 或 HackMD token。

## 10. HackMD API key 儲存

### 10.1 加密要求

HackMD API key 不可明文存入 PostgreSQL。

推薦使用：

```text
XChaCha20-Poly1305
```

或：

```text
AES-256-GCM
```

加密資料：

```text
encryption_key: TOKEN_ENCRYPTION_KEY, 32 bytes base64, from environment secret
nonce: random nonce per encryption
ciphertext: BYTEA
key_version: integer
```

### 10.2 Token fingerprint

為了顯示與比對 token，不存 raw hash，改用 HMAC fingerprint：

```text
token_fingerprint = HMAC-SHA256(token, TOKEN_FINGERPRINT_KEY)
display_fingerprint = first 8 chars of token_fingerprint
```

UI 顯示：

```text
Token fingerprint: hmd_••••a1b2c3d4
```

### 10.3 不可記錄的資料

以下資料不得進入 logs：

```text
HackMD API key
Authorization header
完整 MCP request body
完整 MCP response body
筆記內容 Markdown
OAuth access token
authorization code
magic link token
session cookie
```

### 10.4 可記錄的資料

```text
request_id
user_id
client_id
tool_name
method
status_code
duration_ms
upstream_status
error_code
access_mode
scope_check_result
```

## 11. PostgreSQL schema

```sql
CREATE TABLE users (
  id UUID PRIMARY KEY,
  email TEXT UNIQUE,
  display_name TEXT,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE web_sessions (
  id UUID PRIMARY KEY,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  session_token_hash TEXT NOT NULL UNIQUE,
  expires_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE magic_links (
  id UUID PRIMARY KEY,
  email TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  expires_at TIMESTAMPTZ NOT NULL,
  consumed_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE hackmd_credentials (
  user_id UUID PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  token_ciphertext BYTEA NOT NULL,
  token_nonce BYTEA NOT NULL,
  token_fingerprint TEXT NOT NULL,
  key_version INTEGER NOT NULL DEFAULT 1,
  mode TEXT NOT NULL DEFAULT 'read_write_no_delete',
  status TEXT NOT NULL DEFAULT 'valid',
  hackmd_user_id TEXT,
  hackmd_email TEXT,
  hackmd_name TEXT,
  verified_at TIMESTAMPTZ NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE oauth_clients (
  client_id TEXT PRIMARY KEY,
  client_name TEXT,
  redirect_uris JSONB NOT NULL,
  client_metadata_json JSONB NOT NULL,
  token_endpoint_auth_method TEXT NOT NULL DEFAULT 'none',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE oauth_authorization_requests (
  id UUID PRIMARY KEY,
  user_id UUID REFERENCES users(id) ON DELETE CASCADE,
  client_id TEXT NOT NULL REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
  redirect_uri TEXT NOT NULL,
  state TEXT,
  resource TEXT NOT NULL,
  scopes TEXT NOT NULL,
  code_challenge TEXT NOT NULL,
  code_challenge_method TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'pending',
  expires_at TIMESTAMPTZ NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE oauth_authorization_codes (
  id UUID PRIMARY KEY,
  code_hash TEXT NOT NULL UNIQUE,
  authorization_request_id UUID NOT NULL REFERENCES oauth_authorization_requests(id) ON DELETE CASCADE,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  client_id TEXT NOT NULL REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
  redirect_uri TEXT NOT NULL,
  resource TEXT NOT NULL,
  scopes TEXT NOT NULL,
  code_challenge TEXT NOT NULL,
  code_challenge_method TEXT NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL,
  consumed_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE oauth_access_tokens (
  id UUID PRIMARY KEY,
  token_hash TEXT NOT NULL UNIQUE,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  client_id TEXT NOT NULL REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
  resource TEXT NOT NULL,
  scopes TEXT NOT NULL,
  expires_at TIMESTAMPTZ NOT NULL,
  revoked_at TIMESTAMPTZ,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE mcp_sessions (
  id UUID PRIMARY KEY,
  user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  client_id TEXT NOT NULL REFERENCES oauth_clients(client_id) ON DELETE CASCADE,
  upstream_session_id_hash TEXT NOT NULL UNIQUE,
  status TEXT NOT NULL DEFAULT 'active',
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  deleted_at TIMESTAMPTZ
);

CREATE TABLE audit_logs (
  id UUID PRIMARY KEY,
  user_id UUID REFERENCES users(id) ON DELETE SET NULL,
  client_id TEXT,
  event_type TEXT NOT NULL,
  tool_name TEXT,
  status TEXT NOT NULL,
  metadata_json JSONB,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

## 12. MCP proxy 行為

### 12.1 基本代理規則

`/mcp` 收到 request 後：

1. 驗證 `Authorization: Bearer <wrapper_access_token>`。
2. 查詢 `oauth_access_tokens`。
3. 驗證 token 未過期、未 revoke、resource 正確。
4. 解析 user。
5. 確認使用者已設定 HackMD credential。
6. 解密該使用者的 HackMD API key。
7. 對 POST JSON-RPC request 進行最低限度 policy inspection。
8. 移除 ChatGPT 傳來的 `Authorization` header。
9. 設定 upstream `Authorization: Bearer <hackmd_api_key>`。
10. 將 request 轉發到 `https://mcp.hackmd.io`。
11. 將 upstream response status、headers、body stream 回傳給 ChatGPT。
12. 若 upstream response 含 `MCP-Session-Id`，將 session id hash 綁定到目前 user + client_id。

### 12.2 POST request inspection

MCP POST body 是 JSON-RPC request / notification / response。

Backend 應讀取 body bytes：

```text
- 不記錄 body
- 不保存 body
- 只解析 JSON-RPC envelope
- 若 method == "tools/call"，讀取 params.name 作為 tool_name
- 完成 policy check 後，用原始 body bytes 轉發 upstream
```

### 12.3 GET request

GET `/mcp` 可能用於 SSE stream。

Backend 應：

```text
- 驗證 OAuth access token
- 驗證 MCP-Session-Id 若存在
- 不 buffer upstream response
- 直接 stream response body
```

### 12.4 DELETE request

DELETE `/mcp` 用於結束 MCP session。

Backend 應：

```text
- 驗證 OAuth access token
- 驗證 MCP-Session-Id 屬於目前 user + client_id
- proxy DELETE 到 HackMD upstream
- 將 mcp_sessions 對應紀錄標記為 deleted
```

### 12.5 必須保留的 request headers

```text
Accept
Content-Type
MCP-Protocol-Version
MCP-Session-Id
Last-Event-ID
User-Agent
```

### 12.6 不可轉發的 request headers

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

`Authorization` 必須改成 HackMD API key，不可把 ChatGPT OAuth token 轉發給 HackMD。

### 12.7 Response headers

應保留：

```text
Content-Type
MCP-Session-Id
MCP-Protocol-Version
Cache-Control
```

應移除或重新設定：

```text
Set-Cookie
Server
Alt-Svc
Transfer-Encoding
Content-Encoding
Content-Length
```

如果 response body 是 stream，不要手動 buffer 全部內容。

## 13. MCP session binding

因為 HackMD upstream 可能回傳 `MCP-Session-Id`，wrapper 必須避免 session id 被不同 user 重用。

### 13.1 建立 session mapping

當 upstream response 含：

```text
MCP-Session-Id: <upstream_session_id>
```

Backend 應儲存：

```text
user_id
client_id
hash(upstream_session_id)
created_at
last_seen_at
```

不可明文保存 upstream session id。

### 13.2 驗證 session mapping

後續 request 若包含 `MCP-Session-Id`：

```text
- hash 該 session id
- 查 mcp_sessions
- 確認 user_id 與 client_id 相同
- 若不同，回傳 403，不轉發 upstream
```

### 13.3 無 session id 的 request

如果 request 沒有 `MCP-Session-Id`：

```text
- initialize request：允許
- 其他 request：可先允許 upstream 判斷，但記錄 debug-level event
```

若之後發現 HackMD upstream 強制要求 session id，再改成更嚴格策略。

## 14. Policy guard

雖然本專案是 direct proxy，第一版仍應攔截 `tools/call`，避免 OAuth scope 與使用者 access mode 被繞過。

### 14.1 使用者 access mode

```text
read_only
  僅允許讀取、搜尋與取得使用者資訊。

read_write_no_delete
  預設模式。
  允許讀取、搜尋、建立、更新。
  不允許 delete。

full_access
  允許讀取、搜尋、建立、更新、刪除。
```

### 14.2 預設值

預設使用：

```text
read_write_no_delete
```

理由：HackMD API key 可能有讀寫權限，但刪除工具風險較高，應要求使用者明確開啟。

### 14.3 攔截規則

若 JSON-RPC method 是 `tools/call`，backend 應 parse request body，取得 tool name。

若 tool name 屬於 read 類工具：

```text
需要 hackmd.read scope
需要 mode != disconnected
```

若 tool name 屬於 write 類工具：

```text
需要 hackmd.write scope
需要 mode in read_write_no_delete 或 full_access
```

若 tool name 屬於 delete 類工具：

```text
需要 hackmd.delete scope
需要 mode == full_access
```

若 tool name 不在已知清單：

```text
MVP 預設允許，但記錄 audit log。
安全優先部署可用 STRICT_TOOL_ALLOWLIST=true 改成預設阻擋。
```

### 14.4 JSON-RPC policy error

Scope 不足：

```json
{
  "jsonrpc": "2.0",
  "id": "<same id>",
  "error": {
    "code": -32003,
    "message": "Insufficient scope for this HackMD tool."
  }
}
```

Access mode 阻擋：

```json
{
  "jsonrpc": "2.0",
  "id": "<same id>",
  "error": {
    "code": -32004,
    "message": "This HackMD tool is disabled by your connector access mode."
  }
}
```

### 14.5 `tools/list` patch

MVP 不 patch `tools/list`，保持 HackMD upstream response 透明。

第二版可選擇 patch `tools/list`：

```text
- 補上 securitySchemes
- 補上 destructive annotations
- 對 delete tools 加上更清楚的 description
```

但 patch 必須小心，不能破壞 MCP schema compatibility。

## 15. Frontend UI 規格

### 15.1 `/login`

用途：

```text
讓使用者登入 wrapper。
```

內容：

```text
- Email input
- Send magic link button
- Single-user mode 下可顯示 invite code / admin password input
```

### 15.2 `/connect`

用途：

```text
讓使用者貼 HackMD API key 並選擇 access mode。
```

頁面內容：

```text
Title:
  Connect HackMD

Description:
  This connector lets ChatGPT access HackMD through your own HackMD API token.
  Your token will be encrypted before storage.
  Anyone with this token may read and modify your HackMD notes, so only paste a token you trust this service to use.

Fields:
  HackMD API Token
  Access mode:
    - Read only
    - Read/write, no delete
    - Full access

Actions:
  Connect HackMD
```

驗證失敗：

```text
The token could not be verified. Please check that it is copied correctly and still active in HackMD Settings → API.
```

驗證成功：

```text
HackMD connected successfully.
You can return to ChatGPT.
```

### 15.3 `/oauth/consent`

用途：

```text
顯示 ChatGPT request 的 scopes，讓使用者同意或拒絕。
```

內容：

```text
ChatGPT wants to access HackMD through this connector.

Requested permissions:
- Read notes
- Create and update notes
- Delete notes

Current connector access mode:
- Read/write, no delete

Note:
Even if ChatGPT requests delete permission, this connector will block delete actions unless Full access is enabled.
```

若 request 包含 `hackmd.delete`，需顯示 destructive warning。

### 15.4 `/settings`

內容：

```text
Connected HackMD account
Current access mode
Last verified time
Token fingerprint
Update API token
Disconnect
```

不顯示完整 API key。

### 15.5 shadcn/ui components

建議使用：

```text
Card
Button
Input
Alert
Select
Badge
Dialog
Toast / Sonner
Form
Separator
```

## 16. 錯誤處理

### 16.1 沒有 HackMD credential

HTTP status：401 或 403
建議：若希望 ChatGPT 重新觸發 OAuth，回 401 + `WWW-Authenticate`；若使用者已授權但未設定 HackMD token，回 MCP JSON-RPC error。

```json
{
  "jsonrpc": "2.0",
  "id": "<same id>",
  "error": {
    "code": -32010,
    "message": "HackMD API token is not configured. Please connect your HackMD account first."
  }
}
```

### 16.2 HackMD credential 失效

行為：

```text
- 標記 hackmd_credentials.status = invalid
- 記錄 upstream_auth_failed
- 要求使用者重新進入 /connect 更新 token
```

### 16.3 Upstream timeout

MVP timeout：

```text
普通 JSON response: 30 seconds
SSE stream: no short hard timeout
connect timeout: 10 seconds
```

錯誤：

```json
{
  "jsonrpc": "2.0",
  "id": "<same id if available>",
  "error": {
    "code": -32005,
    "message": "HackMD MCP upstream timed out."
  }
}
```

### 16.4 OAuth token invalid

HTTP status：401

```http
WWW-Authenticate: Bearer resource_metadata="https://your-domain.example/.well-known/oauth-protected-resource"
```

### 16.5 MCP session mismatch

HTTP status：403

```json
{
  "jsonrpc": "2.0",
  "id": "<same id if available>",
  "error": {
    "code": -32006,
    "message": "MCP session does not belong to this user."
  }
}
```

## 17. 安全要求

### 17.1 HTTPS

Production 必須全站 HTTPS。

OAuth metadata、authorization endpoint、token endpoint、MCP endpoint 都必須使用 HTTPS。

### 17.2 Origin validation

對 `/mcp` request，若 `Origin` header 存在，必須驗證 allowlist。

Allowlist：

```text
https://chatgpt.com
https://chat.openai.com
local development origin, only in development
```

MCP transport specification 對 Streamable HTTP server 有 Origin validation 的安全提醒，目的是降低 DNS rebinding 類攻擊風險。

### 17.3 CSRF

所有 frontend POST routes 必須使用 CSRF protection。

OAuth `state` 不等於 server CSRF token；它是 OAuth client 的 opaque state，必須原樣保存與回傳。

### 17.4 Token handling

```text
不要把 HackMD token 回傳給 frontend。
不要把 HackMD token 寫入 logs。
不要把 HackMD token 放入 URL query string。
不要用 localStorage 保存 HackMD token。
不要把 ChatGPT access token 轉發給 HackMD。
不要把 HackMD token 放進 MCP tool output。
```

### 17.5 Least privilege

OpenAI Apps SDK security guidance 建議 connector 採取 least privilege、明確 user consent、defense in depth，並對 write / destructive actions 保持謹慎。

本專案落實方式：

```text
- scopes 分為 read / write / delete
- 使用者 access mode 預設不允許 delete
- tools/call 做 server-side policy guard
- 所有高風險事件寫入 audit log
- 不記錄筆記內容
```

### 17.6 Rate limiting

MVP 可先用 PostgreSQL-backed sliding window 或 in-memory limiter。Production 建議 Redis。

建議限制：

```text
/api/auth/magic/start:
  5 attempts / 10 minutes / IP + email

/api/hackmd/token:
  10 attempts / 10 minutes / user + IP

/token:
  60 requests / 10 minutes / client_id + IP

/mcp:
  120 requests / minute / user
```

### 17.7 Audit log

記錄：

```text
login_success
login_failed
connect_success
connect_failed
disconnect
mode_changed
oauth_client_registered
oauth_authorization_approved
oauth_authorization_denied
oauth_token_issued
oauth_token_revoked
mcp_tool_call_allowed
mcp_tool_call_blocked
mcp_session_created
mcp_session_mismatch
upstream_auth_failed
```

不記錄：

```text
筆記內容
HackMD token
OAuth token
Magic link token
Session cookie
完整 request / response body
```

## 18. 測試計畫

### 18.1 Unit tests

```text
PKCE S256 verification
OAuth client registration validation
redirect URI allowlist
scope parsing
scope matching
opaque token hashing and verification
authorization code single-use behavior
XChaCha20-Poly1305 encrypt/decrypt
token fingerprint generation
CSRF verification
header filtering
JSON-RPC envelope parsing
tools/call policy mapping
MCP session hash binding
```

### 18.2 Integration tests

```text
Submit valid HackMD API key → get-me succeeds
Submit invalid HackMD API key → rejected
ChatGPT-style DCR → client created
Authorization code + PKCE → access token issued
Wrong code_verifier → token request rejected
Missing access token → /mcp rejected
Expired access token → /mcp rejected
read_only mode → update-note blocked
read_write_no_delete mode → delete-note blocked
full_access mode → delete-note forwarded
MCP-Session-Id from upstream → session mapping created
MCP-Session-Id from another user → request blocked
```

### 18.3 Streaming tests

```text
POST /mcp initialize → preserve MCP-Session-Id
POST /mcp tools/list → response passthrough
POST /mcp tools/call get-note → response passthrough
GET /mcp with text/event-stream Accept → response stream passthrough
DELETE /mcp with MCP-Session-Id → upstream DELETE passthrough
```

### 18.4 Manual acceptance tests

```text
Connect wrapper in ChatGPT Web
Complete OAuth
Paste HackMD API key
ChatGPT can list HackMD notes
ChatGPT can read a note
ChatGPT can create a note
ChatGPT can update a note
Delete is blocked by default
Full access mode allows delete
Disconnect removes access
```

## 19. MVP acceptance criteria

MVP 完成條件：

```text
1. Rust Axum backend exposes /mcp over HTTPS.
2. Vite React frontend provides login, connect, consent, settings pages.
3. ChatGPT Web can complete OAuth authorization-code + PKCE flow.
4. Dynamic Client Registration works with ChatGPT.
5. User can submit HackMD API key through wrapper UI.
6. HackMD API key is encrypted at rest.
7. /mcp validates wrapper OAuth access token before proxying.
8. /mcp replaces Authorization header with user HackMD API key.
9. Streamable HTTP response is passed through without buffering.
10. list-notes and get-note work from ChatGPT.
11. create-note and update-note work when mode allows write.
12. delete-note is blocked by default.
13. MCP-Session-Id is bound to user + client_id.
14. User can disconnect and remove stored credential.
15. Logs never contain HackMD token, OAuth token, or note content.
```

## 20. Implementation phases

### Phase 0：Single-user local proof of concept

```text
- Create Rust Axum server
- Add /health
- Add /mcp raw proxy
- Use HACKMD_API_TOKEN from environment
- Verify MCP Inspector can call HackMD through proxy
- No OAuth yet
```

### Phase 1：Streaming proxy correctness

```text
- Implement request header filtering
- Implement response header filtering
- Preserve MCP-Protocol-Version
- Preserve MCP-Session-Id
- Preserve Last-Event-ID
- Ensure SSE response is not buffered
- Add basic integration tests
```

### Phase 2：OAuth skeleton

```text
- Add protected resource metadata endpoint
- Add authorization server metadata endpoint
- Add POST /register for DCR
- Add GET /authorize
- Add POST /token
- Implement PKCE S256
- Implement opaque access tokens
- Protect /mcp with Bearer token validation
```

### Phase 3：Frontend setup UI

```text
- Create Vite React app
- Add shadcn/ui
- Add /login
- Add /connect
- Add /oauth/consent
- Add /settings
- Add CSRF protection
```

### Phase 4：Per-user HackMD token vault

```text
- Add PostgreSQL migrations
- Add SQLx repositories
- Add encrypted HackMD token storage
- Add token verification flow
- Bind HackMD credential to user_id
- Replace fixed env token with per-user credential
```

### Phase 5：Policy guard

```text
- Parse tools/call
- Implement scope checks
- Implement access mode checks
- Block delete by default
- Add audit logs
```

### Phase 6：MCP session binding

```text
- Capture upstream MCP-Session-Id
- Store session hash bound to user + client_id
- Validate session id on subsequent requests
- Handle DELETE /mcp
```

### Phase 7：Production hardening

```text
- Add rate limiting
- Add token revocation
- Add better error pages
- Add structured logs
- Add deployment config
- Add integration tests with real HackMD token in CI secret or staging only
```

## 21. Open questions

1. 是否要一開始支援 refresh token？

   MVP 建議不要。Access token 過期後讓 ChatGPT 重新走 OAuth，比 refresh token 更簡單也更容易 revoke。

2. 是否要支援多個 HackMD account？

   MVP 建議一位 wrapper user 綁一個 HackMD API key。

3. 是否要 patch `tools/list`？

   MVP 建議不 patch，保持 proxy 透明。第二版再考慮加 securitySchemes / destructive annotations。

4. 是否要支援 CIMD？

   MVP 建議先支援 DCR。等 connector 穩定後，再考慮 CIMD。

5. 是否要開放 public signup？

   MVP 建議先用 invite-only 或 single-user mode。因為此服務會儲存高權限 HackMD API key，不適合一開始就公開註冊。

6. 未知 HackMD tools 要預設允許還是阻擋？

   MVP 建議預設允許並記錄 audit log，以免 HackMD upstream 新增工具後立即壞掉。若部署給非自己使用，建議開啟 `STRICT_TOOL_ALLOWLIST=true`。

## 22. 工程原則

本專案的最高原則是：proxy 行為要穩定、透明、可觀測，但不能犧牲 credential safety。

具體原則：

```text
Prefer pass-through over rewriting.
Never expose HackMD API key to ChatGPT.
Never expose HackMD API key to the frontend.
Never log note content.
Block destructive actions by default.
Keep OAuth scope and actual tool permissions aligned.
Bind MCP sessions to user identity.
Avoid buffering SSE streams.
Fail closed on auth errors.
Fail closed on session mismatch.
Keep the UI small and boring.
```
