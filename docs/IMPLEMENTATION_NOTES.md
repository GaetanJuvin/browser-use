# Implementation record

2026-10-22: User requested a Rust version of iFurySt/open-browser-use in GaetanJuvin/browser-use and authorized proceeding with the recommended Chrome scope.

Decisions: adapt the existing MIT JavaScript Chrome extension; newly implement native layers in Rust; target macOS/Linux Unix sockets; omit Electron and upstream packaging/profile-selector features. This is an initial implementation, not a full upstream parity claim. Work uses a separate clone because the ChatGPT project mirror's sources are read-only. No existing destination code or history was replaced.

Validation: frame tests were observed failing against codec stubs before implementation; host/SDK tests were observed failing against missing implementations; CLI/MCP/setup tests were observed failing before command implementation. Socket tests require an environment that permits local Unix sockets. Adapted extension behavior tests pass using Chrome API fixtures.

A separate code reviewer found four issues. Each was reproduced before fixing: unsafe symlink-directory discovery, incomplete cancellation of evicted client tasks, MCP's inability to reconnect on a subsequent explicit action, and load waits exceeding their deadline. All four have regression coverage. The SDK's macOS final-response handling was also tested against a server that closes immediately after replying.

Actual interactive Chrome installation remains a user setup step. No extension was installed into the user's Chrome, and no native-host manifest was written outside controlled test directories. Linux behavior is covered by the CI configuration but was not locally run on this macOS development machine.

Final contract check: verified the extension rejects executeCdp before attach. Added a failing regression, then made the Rust CDP helper attach idempotently before each command within the same absolute deadline. This covers initial navigation and reconnection after service-worker/debugger state resets.

Final local verification: 16 Rust tests passed, 17 Node extension test entries passed, strict clippy passed, formatting passed, and optimized release build passed on macOS. CI is configured for macOS and Linux. Interactive Chrome remains unverified until the extension is loaded and the native host registered.
