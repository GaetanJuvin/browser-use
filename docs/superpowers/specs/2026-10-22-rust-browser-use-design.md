# Rust Browser Use

Build an independent Chrome automation project in GaetanJuvin/browser-use. The user selected Rust and authorized proceeding with the recommended Chrome scope.

The Rust binary provides native messaging relay, CLI, explicit native-host setup, action plans, and stdio MCP. A reusable synchronous Rust SDK provides session-scoped tab/CDP helpers. Adapt the MIT upstream MV3 extension, retaining attribution. No Go or Python runtime. Electron is outside this release.

Approaches considered: direct remote-debugging connection (simpler but does not preserve ordinary Chrome profile access); extension plus Rust relay (selected recommendation, preserves current Chrome workflow); full Electron embedding (additional host integration, deferred).

Use a dedicated native host name com.gaetanjuvin.browser_use and private per-user socket directory. Rewrite client request IDs to prevent collisions, route responses only to their originating client, bound queues/frame sizes, enforce timeouts, clean up registry/socket on exit, and preserve session ownership in the extension. Keep stdout exclusively for wire output in host and MCP modes.

Initial platforms: macOS and Linux (Unix sockets). Explicitly report Windows as unsupported. Setup accepts the unpacked extension ID and registers the current installed binary; it never downloads executables or silently changes browser settings. MCP exposes core actions plus a raw JSON-RPC escape hatch. No claim of full upstream command/SDK parity.

Validation: frame boundary tests; native-host subprocess with two clients using the same ID; notification/error/timeout SDK tests; CLI/action-plan/MCP integration tests; adapted upstream extension tests; formatting, clippy, cargo test and release build. A real interactive Chrome test depends on installing/enabling the extension.
