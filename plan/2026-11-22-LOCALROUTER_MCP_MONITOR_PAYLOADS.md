# LocalRouter MCP input and output in Monitor

- [x] Trace the empty event payloads and the Question preview.
- [x] Restore normal MCP argument, result, and error capture for LocalRouter tools.
- [x] Render absent arguments as a tool call with empty parentheses in the Question column.
- [x] Review the plan against normal desktop retention, metadata-only capture, and the existing secret masking.
- [x] Review test coverage for successful calls, skill activation, failures, absent arguments, and metadata-only clients.
- [x] Re-read the Monitor write paths for payload policy, argument shape, and response lifecycle bugs.
- [x] Run the applicable stable checks, rebuild the local debug app, and verify a fresh event.
- [ ] Commit and push the validated fix.

LocalRouter calls use ordinary MCP Monitor capture and the client retention policy. The gateway records accepted arguments before restoring secret placeholders. Empty or missing arguments render as an empty call in Question. The regression tests cover displayed input and output and metadata-only capture.
