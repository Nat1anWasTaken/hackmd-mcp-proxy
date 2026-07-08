# HackMD MCP Proxy

Rust remote MCP proxy that lets ChatGPT authenticate with this server via OAuth,
while each human user signs in with GitHub and stores their own encrypted HackMD
API key.

## Flow

1. ChatGPT registers as an OAuth client with `POST /register`.
2. ChatGPT opens `GET /authorize`.
3. The proxy redirects the browser through GitHub OAuth if there is no local
   session.
4. The user enters a HackMD API key once. The proxy verifies it against HackMD
   MCP, encrypts it, and stores it for that GitHub user.
5. ChatGPT receives an authorization code and exchanges it at `POST /token`.
6. Later `GET|POST|DELETE /mcp` calls use ChatGPT's proxy access token. The
   proxy decrypts the GitHub user's HackMD key and forwards the request to
   `https://mcp.hackmd.io`.

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

## Development

```bash
rtk cargo run
rtk cargo test
```

The server listens on `127.0.0.1:3000` by default and exposes `GET /health`.
