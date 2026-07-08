# HackMD MCP OAuth Wrapper

Hosted MCP wrapper for HackMD that lets ChatGPT connect through OAuth while each user supplies their own encrypted HackMD API token.

## Local Development

```bash
rtk cargo test --workspace
rtk cargo run -p hackmd-mcp-server
```

The server listens on `127.0.0.1:3000` by default and exposes `GET /health`.
