---
name: active-intercept
description: Temporarily hold selected requests and listen for updates.
---

Call `lr_intercept_start` with client IDs, categories `llm` or `mcp`, and a duration of at most 300 seconds. Approve the rule in LocalRouter. Subscribe to `_localrouter_admin__intercept.changed` with MCP Events if supported, or use the Push Relay event tools or `lr_intercept_list_pending`. Push carries status and request IDs. Inspect a pending item and use `lr_intercept_resolve` with `allow` or `deny` to handle it once. Stop the rule using the returned rule ID. Stopping the rule leaves existing approvals in the desktop queue.
