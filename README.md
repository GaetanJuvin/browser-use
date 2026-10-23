# Browser Use · Rust

Control your real Chrome tabs from a Rust CLI, Rust SDK, or MCP client. Uses your existing browser profile and logged-in sessions through a Chrome extension.

Inspired by [iFurySt/open-browser-use](https://github.com/iFurySt/open-browser-use). The native host, CLI, SDK, and MCP server are written in Rust. The MV3 extension is adapted from the MIT-licensed upstream implementation; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

**Initial release: macOS and Linux.** No Go, Python, Node, or browser-driver runtime is needed to use the installed binary. Rust 1.85+ is required to build it. Node 22+ is used only for extension tests. Windows and Electron integration are outside this release.

## Install

```sh
git clone https://github.com/GaetanJuvin/browser-use.git
cd browser-use
cargo install --path . --locked
```

1. Open `chrome://extensions` in Chrome and enable **Developer mode**.
2. Choose **Load unpacked** and select this repository's `extension/` folder. Keep that folder in its permanent location.
3. Copy the extension's 32-character ID, then register the installed binary:

```sh
browser-use setup --extension-id YOUR_EXTENSION_ID
```

4. Reload the extension, or restart Chrome. Its popup should say **Connected**.

```sh
browser-use ping
browser-use info
browser-use user-tabs
```

The extension uses the dedicated native host `com.gaetanjuvin.browser_use`; it can coexist with upstream Open Browser Use. Setup writes a Chrome native-messaging manifest pointing at the binary you run. Use the installed binary for setup, rather than a temporary build. Re-run setup if you move the binary or extension ID changes. Chromium/Chrome Beta require copying the manifest into their native-messaging host directory; `setup --output /absolute/path/host.json` lets you select that destination. Flatpak/Snap Chrome packages can impose extra native-host restrictions.

## Use the CLI

All browser commands return JSON. Pass a unique `--session-id` per agent task. The manual CLI default is `browser-use-cli`; unrelated tasks should not share it.

```sh
browser-use --session-id research name-session 'Research'
browser-use --session-id research open-tab https://example.com
# Use the numeric id returned by open-tab in subsequent commands:
browser-use --session-id research page-info --tab-id 123
browser-use --session-id research snapshot --tab-id 123
browser-use --session-id research eval --tab-id 123 'document.title'
browser-use --session-id research click --tab-id 123 'a'
browser-use --session-id research fill --tab-id 123 'input[name=q]' 'rust browser automation'
browser-use --session-id research screenshot --tab-id 123
browser-use --session-id research finalize-tabs --keep '[]'
```

`snapshot` returns visible document text. `click`/`fill` are DOM operations on the first CSS match, not Playwright actionability or trusted pointer events; use CDP input commands when those are needed. Screenshots return CDP `{ "data": "<base64 PNG>" }`. Frames are limited to 1 MiB, including screenshots; capture a smaller CDP clip for large pages.

For an existing user tab, list `user-tabs`, then `claim-tab --tab-id ID` before operating on it. The extension enforces session ownership. Finalization closes agent-created tabs and releases claimed user tabs. Retain selected tabs with:

```sh
browser-use --session-id research finalize-tabs \
  --keep '[{"tabId":123,"status":"deliverable"}]'
```

Use `handoff` instead of `deliverable` for a task that should continue later.

### Action plans

One action per line, shell-style quotes, full-line `#` comments. The full plan is parsed before execution. Execution stops at the first failed action; earlier actions remain applied. `open-tab` and `claim-tab` select the default tab for later actions.

```sh
browser-use --session-id example run -c '
name-session "Example task"
open-tab https://example.com
wait-load domcontentloaded
page-info
snapshot
finalize-tabs []
'
```

Use `run --file plan.txt` for a saved plan. Each successful action prints one JSON line. After a failed plan, inspect the session and explicitly finalize it when appropriate.

### Raw CDP and extension APIs

```sh
browser-use --session-id research cdp --tab-id 123 \
  --method Runtime.evaluate \
  --params '{"expression":"document.title","returnByValue":true}'

browser-use --session-id research call --method getUserHistory \
  --params '{"queries":["rust","browser"],"limit":20}'
```

`call` exposes all extension handlers, including file choosers, downloads, clipboard, and WebMCP. See [the extension source](extension/background.js) for parameter contracts. The Rust convenience API covers the common tab, navigation, evaluation, DOM input, and CDP workflows. This release does not promise every upstream CLI or SDK helper.

## Rust SDK

Add the Git dependency until a crates.io release is published:

```toml
[dependencies]
browser-use = { git = "https://github.com/GaetanJuvin/browser-use" }
serde_json = "1"
anyhow = "1"
```

```rust,no_run
use browser_use::{Browser, Options};
use serde_json::json;

fn main() -> anyhow::Result<()> {
    let mut browser = Browser::connect(Options {
        session_id: "my-agent-task".into(),
        ..Options::default()
    })?;
    let mut tab = browser.new_tab()?;
    tab.goto("https://example.com")?;
    tab.wait_for_load("domcontentloaded", std::time::Duration::from_secs(15))?;
    println!("{}", tab.title()?);
    browser.client.finalize_tabs(json!([]))?;
    Ok(())
}
```

The SDK is synchronous; use a blocking worker when integrating into an async runtime. `Client::request` is the unrestricted extension API. `Client::cdp` exposes CDP. `Client::drain_notifications` returns the most recent 256 notifications observed while waiting for responses; it is not a background subscription. SDK sessions and turns get unique IDs by default. Set explicit IDs to join a known task.

Calls have an absolute deadline (default 15 seconds). Transport failures close the client connection. Create a new client to reconnect; requests are never automatically replayed because a timed-out mutation may already have happened. The relay expires pending requests after 120 seconds. Use `--timeout-ms` for the CLI or `Options::timeout` for the SDK.

See [examples/read_page.rs](examples/read_page.rs).

## MCP

Configure your agent's local stdio MCP server:

```toml
[mcp_servers.browser_use]
command = "browser-use"
args = ["mcp", "--session-id", "unique-task-id"]
```

The server supports initialization, tool listing, and tool calls over newline JSON-RPC. It exposes tab listing/opening/claiming, navigation, evaluation, page text, screenshots, DOM click/fill, history, session naming/finalization, raw CDP, raw extension calls, and action plans. Page tools require explicit `tab_id`; action plans can reuse their default tab. It performs no browser connection until a tool actually needs one.

If a long-lived MCP client starts another task, restart the server with a new session ID. After a transport failure, a later explicit MCP action establishes a fresh connection; the failed action is never replayed. MCP EOF disconnects the SDK; it does not silently close pages. Finalize deliberately before ending the task.

## How it works

```text
CLI / Rust SDK / MCP
    ↕ private Unix socket (length-prefixed JSON-RPC)
Rust native host
    ↕ Chrome Native Messaging stdin/stdout
MV3 Chrome extension
    ↕ chrome.tabs / chrome.debugger / history / downloads
Your Chrome profile
```

The host uses `/tmp/browser-use-<uid>/`, an owner-only directory, with mode-0600 sockets and an `active.json` discovery record. Explicit `--socket` and `--socket-dir` options are available. Without an explicit socket, discovery tries the active record and then other sockets in newest-first order. To target multiple Chrome profiles deterministically, select their socket explicitly; this release does not implement upstream's named profile discovery.

The relay replaces request IDs so concurrent clients cannot collide, restores original IDs on replies, broadcasts notifications, and drops unknown responses. Slow clients are disconnected rather than blocking other clients. The same OS user can access the local sockets; the host is not an authorization boundary against other processes running as you. Site restrictions and approval rules belong to the consuming agent.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
node --test extension/*.test.mjs
cargo build --release --locked
```

Rust tests exercise actual subprocesses, pipes, Unix sockets, concurrent ID routing, SDK timeouts/errors, CLI plans, and MCP messages. Extension tests use controlled Chrome API fixtures; they do not substitute for an installed-extension check in real Chrome. To verify Chrome locally, follow installation above and run the example action plan.

## Troubleshooting

- **No live Chrome connection:** inspect the extension popup, verify the copied ID, rerun setup with the installed binary, then reload the extension or restart Chrome.
- **Wrong profile selected:** use `--socket` for the intended host instance.
- **Permission denied binding a test socket:** sandboxed development environments may need permission for local Unix sockets.
- **Frame too large:** reduce the CDP response or screenshot clip; messages above 1 MiB are rejected.
- **Debugger detached:** close Chrome DevTools for that tab, then retry with a freshly connected client if necessary.

MIT license. See [LICENSE](LICENSE) and [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
