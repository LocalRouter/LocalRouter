- [x] Confirm the discovery launch differs from saved MCP startup.
- [x] Merge resolved shell environment into STDIO discovery, preserving explicit overrides.
- [x] Add regression coverage for a GUI-like restricted PATH, child executable resolution, and environment overrides.
- [x] Plan Review: compare implementation with intended behavior and fix omissions.
- [x] Test Coverage Review: verify default PATH and user override behavior.
- [x] Bug Hunt: reread launch, environment precedence, and process cleanup paths.
- [x] Validate with updated rustup stable: workspace Clippy, formatting, and tests with explicit stable RUSTC and sysroot PATH.
- [ ] Commit and push changes and any automatic catalog update to current upstream, preserving unrelated files.

Discovery currently passes only custom environment variables to StdioTransport, unlike both saved-server manager launch paths. Reuse lr_utils::binary::shell_env without changing transport behavior or Tauri interfaces.

Review: discovery now uses shell PATH then explicit environment overrides, matching saved MCP startup. The transport, HTTP discovery, working directory, and cleanup semantics are unchanged. An isolated subprocess regression exercises restricted GUI PATH, a shell-only executable with an env shebang, custom PATH precedence, and custom variable propagation. All 12 discovery tests passed. Full stable workspace checks remain in progress.

Final validation: updated rustup stable 1.99.1 with stable sysroot bin prepended to PATH and explicit stable RUSTC. Workspace Clippy (-D warnings), formatting check, and workspace tests passed (4642 passed, 0 failed, 69 ignored, including doc tests). All 12 focused discovery tests passed. The build automatically refreshed modelsdev_raw.json from 8902 to 8917 models; include it per repository instructions. Existing untracked release plans are untouched.
