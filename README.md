# HackMD MCP Proxy

Rust remote MCP proxy that lets ChatGPT authenticate with this server via OAuth,
while each human user signs in with GitHub and stores their own encrypted HackMD
API key. The proxy exposes an agent-friendly local MCP tool surface backed by
HackMD's REST API, so agents use the same tools for personal and team notes.

## Flow

1. ChatGPT registers as an OAuth client with `POST /register`.
2. ChatGPT opens `GET /authorize`.
3. The proxy redirects the browser through GitHub OAuth if there is no local
   session.
4. The user enters a HackMD API key once. The proxy verifies it against HackMD
   REST, encrypts it, and stores it for that GitHub user.
5. ChatGPT receives an authorization code and exchanges it at `POST /token`.
6. Later `POST /mcp` calls use ChatGPT's proxy access token. The proxy decrypts
   the GitHub user's HackMD key and handles MCP JSON-RPC locally.

## Tools

The local MCP server exposes unified note and folder tools for personal and team
workspaces. `hackmd_list_notes` supports metadata filtering with `query`, `tags`,
`folder_id`, `limit`, `offset`, and `sort`.

Use `hackmd_edit_note` for normal note-body edits. It accepts Codex-style patch
blocks that target the `patch_path` returned by `hackmd_get_note`. Use
`hackmd_update_note` only when editing metadata or when patch editing is not
enough.

## Configuration

Copy `.env.example` to `.env` and set real secrets.

Create a GitHub OAuth App with callback URL:

```text
{PUBLIC_BASE_URL}/auth/github/callback
```

Generate secret values with:

```bash
openssl rand -base64 32
```

Use one generated value for each key in `.env`.

`HACKMD_API_URL` defaults to `https://api.hackmd.io/v1`.

## Development

```bash
rtk cargo run
rtk cargo test
```

The server listens on `127.0.0.1:3000` by default and exposes `GET /health`.
