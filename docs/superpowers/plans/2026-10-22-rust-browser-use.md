# Rust Browser Use Implementation Plan

> Execute inline with test-first development; user authorized proceeding.

**Goal:** Chrome extension + Rust host, CLI, SDK and MCP.
**Architecture:** Private Unix sockets bridge native-messaging frames to session-scoped RPC; shared Rust SDK powers CLI/MCP. Adapt upstream extension with new identity.
**Tech Stack:** Rust 2024, Tokio, serde_json, clap, MV3 JavaScript.
**Spec:** ../specs/2026-10-22-rust-browser-use-design.md

## Global Constraints
Rust 1.85+, macOS/Linux, MIT attribution, no Go/Python runtime, bounded frames/queues, stdout reserved for protocol.

## Review Focus
Fragmented/oversized frames; concurrent duplicate request IDs; notifications preceding responses; timeout followed by reconnect; cleanup preserving another host's registry.

### Task 1: Wire codec
Files: src/wire.rs, tests/wire.rs. Interface: read_frame/write_frame over std::io; async equivalents for host.
- [x] Fail tests for fragmented input, EOF, malformed/oversized frames and exact output.
- [x] Implement bounded native-endian framing; rerun cargo test.

### Task 2: Relay and SDK
Files: src/host.rs, src/client.rs, src/browser.rs, tests/transport.rs.
Interfaces: host::run(dir); Client::connect(Options), request(method, Value); Browser/Tab helpers.
- [x] Fail subprocess tests for collision-safe routing, notifications, errors, disconnect and timeouts.
- [x] Implement private discovery, routing, bounded channels, cleanup and synchronous SDK with absolute deadlines.
- [x] Verify full suite.

### Task 3: CLI, setup and MCP
Files: src/main.rs, src/cli.rs, src/mcp.rs, src/setup.rs, tests/cli.rs.
- [x] Fail behavioral tests for command mapping, action-plan default tab and MCP lifecycle/error handling.
- [x] Implement clap commands, setup manifest, shared runner and newline JSON-RPC MCP tools.
- [x] Verify full suite and executable help/setup output.

### Task 4: Extension, documentation and delivery
Files: extension/*, README.md, LICENSE, THIRD_PARTY_NOTICES.md, examples/*, .github/workflows/ci.yml.
- [x] Adapt MIT extension identity; retain and run meaningful upstream tests.
- [x] Document installation, Rust SDK, MCP, feature boundaries and Chrome verification limitations.
- [x] Run fmt, clippy, all tests and release build; review the final code and prepare the initial commit for the requested empty repository.
