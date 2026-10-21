# Allow initialization when optional HTTP GET stream stays silent

- [x] Record live proxy-free GET/POST evidence and isolate startup dependency.
- [x] Allow HTTP POST initialization after the GET readiness deadline when no HTTP/network error occurred; retain errors for rejected requests.
- [x] Regression test a silent GET with a working initialize POST.
- [x] Plan Review: preserve legacy endpoint discovery and connection error reporting.
- [x] Test Coverage Review: verify silent GET, rejected GET and inline fallback.
- [x] Bug Hunt: inspect background cancellation and transport health.
- [x] Validate live transport, run stable workspace Clippy/fmt/tests.
- [ ] Commit and push only task changes and generated catalog updates.

## Evidence and implementation review

The configured production URL was changed through the running app to https://mcp.atlassian.com/v1/mcp/authv2 at the user's request. This initially returned initialize in 1.74 seconds and GET 400 in 4.09 seconds, but the running app subsequently reproduced the GET timeout there too. No claim of reliable restoration is justified.

With all proxy environment variables removed, direct requests also stalled. One valid initialize eventually succeeded with a session after 68.35 seconds; an unauthenticated-session tools/list validation response took 72.28 seconds. Patched Rust transport probes reached POST on both endpoints, then hit the existing 30-second HTTP request deadline. Therefore the confirmed LocalRouter bug (optional GET incorrectly required for startup) is distinct from the remaining intermittent slow upstream POST behavior. Increasing every timeout would mask, not resolve, this service degradation.

Implementation preserves real GET errors, legacy endpoint discovery, background notifications, and cancellation. A silent GET now permits POST initialization after the existing readiness deadline. The regression test proves initialization succeeds against a server that never answers GET. All 16 focused SSE tests and stable 1.99.1 workspace Clippy, formatting, and workspace tests passed. No Tauri command or frontend type changes are needed.
