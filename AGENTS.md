# Project Agent Instructions

@/Users/nathan/.codex/RTK.md

## Rust Structure Policy

This crate should stay organized by domain and responsibility, not by file size
alone.

- Keep stable public entry points in `src/lib.rs`: `build_router`, `config`,
  `state::AppState`, `store::Store`, and the narrow HackMD MCP test surface.
- Prefer private or `pub(crate)` modules for implementation details. Do not make
  modules public just to simplify imports.
- Keep HTTP concerns under `src/http/`: route wiring and handlers in `mod.rs`,
  with helpers split into focused modules such as errors, sessions/cookies, and
  page rendering.
- Keep HackMD/MCP concerns under `src/hackmd/`: JSON-RPC protocol handling,
  REST client operations, tool schemas, models, path helpers, auth challenges,
  and errors should remain separate.
- Keep persistence under `src/store/`: public store types and connection setup in
  `mod.rs`, with schema and table/domain operations split into focused
  submodules.
- Keep small cohesive modules such as `config`, `crypto`, `github`,
  `observability`, `oauth`, `patch`, and `state` as single files unless they
  develop multiple clear responsibilities.
- Preserve external behavior unless the task explicitly asks for a behavior
  change: routes, JSON payloads, MCP tool names/schemas, database schema,
  environment variables, and OAuth flow should not drift during structural
  cleanup.

## Rust Implementation Guide

- Follow the existing module boundaries before adding new abstractions.
- Add an abstraction only when it removes real duplication or clarifies a domain
  boundary.
- Prefer typed data structures, `Result`, and focused error enums over stringly
  typed control flow.
- Keep request/response DTOs near the handler or protocol layer that owns their
  wire format.
- Keep database row mapping in store submodules rather than leaking SQLx details
  into HTTP or HackMD logic.
- Preserve current integration-test import paths where practical. If a path must
  change, update tests as part of the same change.
- Run these checks before considering Rust work complete:

```bash
rtk cargo fmt -- --check
rtk cargo test
rtk cargo clippy --all-targets -- -D warnings
```
