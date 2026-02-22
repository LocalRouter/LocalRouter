<!-- @entry approval-flow -->

The firewall intercepts API requests in real-time and presents them to the user for approval before forwarding to the provider. When a request matches a firewall rule, the UI displays the request details (client, model, prompt content) and offers three actions: Allow Once, Allow for Session, or Deny.

The request is held in a pending state until the user responds — the calling client receives the response only after approval. This provides human-in-the-loop oversight for sensitive operations.

<!-- @entry allow-once -->

**Allow Once** approves the specific request and forwards it to the provider immediately. No future requests are affected — each subsequent matching request will trigger the approval flow again.

This is the most restrictive option, suitable for high-sensitivity operations where every invocation should be reviewed.

<!-- @entry allow-session -->

**Allow for Session** approves the current request and automatically approves all future matching requests for the duration of the current session. The approval is stored in memory and cleared when LocalRouter restarts.

This balances oversight with usability — after an initial review, repeated similar requests proceed without interruption.

<!-- @entry deny -->

**Deny** rejects the request and returns an error to the calling client. The provider is never contacted. The error response uses a standard HTTP 403 status with a message indicating the request was denied by the firewall.

Denied requests are logged in the access log for audit purposes.

<!-- @entry request-inspection -->

Before approving or denying, users can inspect the full request payload in the firewall UI. This includes the client identity, target model, the complete message history (system prompt, user messages, assistant messages), any tool calls, and request parameters (temperature, max_tokens, etc.).

Users can also modify the request before approving — for example, redacting sensitive information from the prompt or changing the target model.

<!-- @entry approval-policies -->

Approval policies define which requests trigger the firewall approval flow. Policies can be configured at multiple granularity levels and combined.

When no policy matches a request, it passes through without approval. Policies are evaluated in order, and the first matching policy determines the behavior.

<!-- @entry policy-per-client -->

Per-client policies trigger approval for all requests from a specific client. This is useful for untrusted or new clients where you want to review every request before it reaches a provider.

Configure by setting `firewall: require_approval` on the client's configuration.

<!-- @entry policy-per-model -->

Per-model policies trigger approval when requests target specific models. For example, you might require approval for expensive models (GPT-4, Claude Opus) while allowing cheaper models to pass through freely.

This provides cost control through human oversight.

<!-- @entry policy-per-mcp -->

Per-MCP-server policies trigger approval for tool calls targeting specific MCP servers. This is useful for servers that perform sensitive operations (file system access, database writes, API calls).

Each tool invocation on the protected server requires explicit user approval.

<!-- @entry policy-per-skill -->

Per-skill policies trigger approval when specific skills are invoked. Since skills can orchestrate multiple tool calls, approving at the skill level provides oversight over the entire workflow rather than individual steps.

This is particularly important for skills that perform irreversible actions or access sensitive data.
