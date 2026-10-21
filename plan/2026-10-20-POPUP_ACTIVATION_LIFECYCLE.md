# Approval popup activation lifecycle

- [x] Trace popup creation, presentation, close, expiry and tray reopen paths; verify native app activation behavior for hidden and background main windows.
- [x] Centralize approval presentation and record whether it temporarily activates a background macOS app. Keep ordinary windows and Windows/Linux behavior intact.
- [x] Return temporary activation when the final focused approval closes, using AppKit's standard Hide operation (the same as Command-H); preserve foreground main-window work, other approvals and user focus changes.
- [x] Synchronize any new Tauri command, TypeScript params and demo mock; make all popup creation/presentation paths use the same lifecycle.
- [x] Validate focused lifecycle regressions and native behavior, plus frontend types and exact stable workspace Clippy/fmt/tests with pinned compiler.
- [x] Plan Review: compare implementation to all creation/closing paths and fix omissions.
- [x] Test Coverage Review: cover multiple approvals, foreground/background origins, unrelated windows, close and expiry.
- [x] Bug Hunt: inspect focus races, re-entry, stale state and platform differences.
- [x] Commit and push completed validated changes and automatic catalog updates, preserving unrelated release drafts.

Native NSPanel and status-item NSPopover are viable longer-term presentation options, but Tauri's current native window builder does not directly construct them. Avoid Objective-C class swapping, private APIs, delayed main-window hiding or forcing another application to activate. Native probes showed that deactivate() alone leaves the main window key and the app active, so that approach was rejected. The standard Hide operation correctly returns the app to the background. This intentionally returns an approval-only interaction to the tray; if the main window was open behind another app, it is hidden until the user reopens LocalRouter. Choosing any regular LocalRouter window during approval cancels that return-to-tray behavior.


Review findings and validation

- All five popup builder sites (three production listeners, debug firewall trigger and tray creation) now start hidden and unfocused. All three React approval views present through the new command after loading/resizing; existing loaded tray popups use the same native lifecycle.
- The macOS activation session captures application activity before presentation, shares the original state across overlapping approvals and clears it when any regular window gains focus. Background expiry, unknown closes and repeated presentation do not steal focus.
- The standard AppKit Hide operation runs before the last focused popup is hidden. Native AppKit probes verified both an already-hidden main window and an open background main window: afterward appActive=false, mainVisible=false and mainKey=false. The original deactivate-only probe left appActive=true and selected the visible main window.
- Nine lifecycle regressions passed in both library and binary test suites. Frontend regressions: 82 passed. App and website TypeScript checks and the production frontend build passed. Full final workspace checks use stable rustc 1.99.1 with PATH and RUSTC pinned to /Users/matus/.rustup/toolchains/stable-aarch64-apple-darwin/bin/rustc.
- The running installed app was inspected, but the fix was not installed over the user's production application. Native behavior was verified in a dedicated AppKit probe; full installed-app acceptance awaits a build containing the fix.
- Preserve unrelated release-plan drafts and concurrent MCP SSE edits. Include the build's automatic models.dev snapshot refresh as required by AGENTS.md.

Final checks on the integrated v0.0.161 checkout passed: stable workspace Clippy with -D warnings, formatting, and all 111 test suites (4,547 passed, 69 ignored, zero failures). The test run includes concurrent, unstaged MCP SSE work; that work is excluded from the popup commit. App and website type checks, 82 frontend unit regressions and the production frontend build passed. Commit and normal push follow these completed checks.
