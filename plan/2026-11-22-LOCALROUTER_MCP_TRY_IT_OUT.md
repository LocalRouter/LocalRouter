# LocalRouter MCP Info and Try It Out

- [x] Inspect the standard MCP detail and Try It Out components, the local listener, and the existing client authentication.
- [x] Make LocalRouter use the standard MCP detail layout and the bundled LocalRouter logo.
- [x] Use the existing Direct MCP tester and its authenticated internal test client without a client picker.
- [x] Keep the dedicated endpoint loopback-only and require a configured client bearer credential. Scope the internal test token to the existing Direct MCP tester path.
- [x] Use the shared firewall approval template for LocalRouter tool calls with one-time decisions.
- [x] Review the implementation against the plan.
- [x] Review test coverage for the internal test client, actual MCP transport, credential and grant checks, and the peer restriction.
- [x] Re-read the changes for stale credentials, session lifecycle issues, and unintended access.
- [x] Run stable Rust and frontend checks. Run the debug app and verify the authorized and denied paths.
- [ ] Commit, use a merge commit to integrate with master, and verify the push and automatic website workflow.

Try It Out uses the standard Direct MCP tester and the app’s existing internal test client. Dedicated endpoint checks use the socket peer and the Authorization header. The desktop origin can preflight the dedicated route when shared gateway CORS is disabled. Administrative tools and intercept event discovery are limited to the dedicated endpoint and authenticated desktop Direct MCP sessions. The standard firewall template uses one-time actions for LocalRouter approvals.
