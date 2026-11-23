# LocalRouter MCP Info and Try It Out

- [x] Inspect the standard MCP detail and Try It Out components, the local listener, and the existing client authentication.
- [x] Make LocalRouter use the standard MCP detail layout and the bundled LocalRouter logo.
- [x] Embed the existing MCP tester using the dedicated endpoint and an eligible, explicitly granted client credential.
- [x] Require a real client bearer credential and a loopback peer on the dedicated endpoint. Route management tools exclusively through that endpoint.
- [x] Use the shared firewall approval template for LocalRouter tool calls with one-time decisions.
- [x] Review the implementation against the plan.
- [x] Review test coverage for client selection, actual MCP transport, credential and grant checks, and the peer restriction.
- [x] Re-read the changes for stale credentials, session lifecycle issues, and unintended access.
- [ ] Run stable Rust and frontend checks. Run the debug app and verify the authorized and denied paths.
- [ ] Commit, use a merge commit to integrate with master, and verify the push and automatic website workflow.

The desktop uses the Streamable HTTP SDK transport and a configured gateway-mode client. Dedicated endpoint checks use the socket peer and the Authorization header. The desktop origin can preflight the dedicated route when shared gateway CORS is disabled. Administrative tools and intercept event discovery stay on the dedicated connection. The standard firewall template uses one-time actions for LocalRouter approvals.
