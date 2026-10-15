# Independent native Realtime TLS initialization

- [x] Initialize the project's ring crypto provider independently of optional proxy startup before native WebSocket TLS setup; preserve an already installed provider and certificate validation.
- [x] Add an isolated-process regression that exercises a real local WSS ClientHello and verifies a malformed TLS peer returns an error without a crypto-provider panic or paid call.
- [x] Plan Review: verify plain WS and secure WSS both use the same native connection helper and existing deadlines/limits/authentication.
- [x] Test Coverage Review: verify the test starts without a global crypto provider so other workspace/proxy tests cannot mask the regression.
- [x] Bug Hunt: inspect provider selection, global initialization races, TLS verification and timeout/cleanup behavior.
- [x] Update stable, confirm compiler paths, and run exact workspace Clippy/fmt/tests plus affected frontend validation.
- [x] Commit and normal push validated changes, preserve unrelated drafts, and monitor CI (CI outcome is reported separately).

Evidence: the workspace links rustls 0.23 with both ring and aws-lc-rs. The
WebSocket library's default TLS configuration calls ClientConfig::builder(),
which cannot choose a crypto provider when neither global initialization nor
one unambiguous compiled provider is available. Only lr-proxy's manager startup
currently installs ring. Native Realtime must also work without that manager.
The existing default/provider selection is retained if already configured.

The isolated WSS regression passed. Test-only aws-lc-rs activation ensures both
crypto choices are present even when running only lr-server, so another
workspace/proxy test cannot mask the original ambiguity. Full parity passed.

Final validation: rustup updated stable to Rust 1.99.1 (58cf8f9c3); PATH and
RUSTC both resolve the stable rustup compiler. Exact workspace Clippy with
warnings denied, formatting check and all 111 workspace test suites passed:
4,523 passed, 69 ignored, zero failures. This includes the isolated WSS
regression and all seven native Realtime endpoint fixtures. All 72 frontend
regressions passed. No paid calls were made. Validation logs are
/tmp/realtime-tls-ci-{clippy,fmt,workspace}.log and
/tmp/realtime-tls-frontend-tests.log. The main Realtime feature commit da73b29c
is green in cloud CI: https://github.com/LocalRouter/LocalRouter/actions/runs/38779717294.
