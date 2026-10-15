# Named route versions and deterministic rules

The existing custom auto-model name is a stable named route for its reusable strategy. Clients assigned to that strategy keep sending that alias while its routing configuration changes. Current model permissions, privacy, rate/spend limits and endpoint compatibility apply to every candidate. Publication never grants model permissions or changes auto-router enablement.

Under Models, Named route versions lets you copy active settings into a draft, edit and preview its policy, save the draft and explicitly publish the saved snapshot. The snapshot includes the alias, model pools, policy, adaptive ordering/bounds and session settings. Publication and rollback retain the current alias. The ordinary routing controls remain available; edits made there appear as modifications outside the published revision.

Published revisions are immutable. Activation/rollback selects an existing snapshot and preserves an existing draft. Removing an inactive revision or discarding a draft is deliberate. Each strategy keeps at most 32 revisions and 1 MiB of version state, with a 128 KiB ceiling per snapshot. At the limit, explicitly remove an inactive revision before publishing again. Stale edits are rejected using the full strategy fingerprint, including live permission changes.

Drafts and inactive history do not change active session/cache identities or prepared native-batch policy. A matching active revision is part of effective routing identity. Publication changes that identity; provider-bound continuations can require a fresh session. A precomputed decision is rejected if its routing scope changes during preflight. Requests already dispatched retain their existing lifetime and normal current-policy checks.

Policies support up to 64 ordered deterministic rules. Exact client-mode rules keep their existing precedence; the first matching deterministic rule then runs before semantic/default routing. Conditions combine with AND:

- Authenticated client IDs, supplied by LocalRouter rather than request metadata.
- Presence or absence of requested tools and image input in the routing projection.
- Minimum/maximum message-text character count (Unicode characters; image bytes and tool schemas are excluded).
- A maximum estimated cost for every currently eligible candidate/fallback in the selected branch. Missing prices/eligible candidates do not satisfy the condition. This is a routing condition, not a spending guarantee; actual billing and configured spending limits remain separate.

A rule selects an existing route option. Its branches have positive integer weights totaling 10,000 (100%). A single branch needs no experiment state. Multiple branches use a local SHA-256 bucket over strategy, rule, authenticated client and optional session ID. The same client/session stays in the same bucket for unchanged rules; no traffic is mirrored and no extra inference occurs. Candidate permissions/privacy still apply after selection, including ordinary ordered fallbacks.

Read-only preview uses cached model/pricing/capability information and the complete draft snapshot. When client identity is needed, open that client's tab; a global preview reports unknown identity instead of inventing a branch. Explicit classifier testing remains a user action and uses current permissions. The website demo labels its simulated rule assignment; its buckets are illustrative.

Explanations include the alias, matching published revision (or unpublished-modification state), selected rule/branch and ordinary candidate exclusions. Replay artifacts retain this metadata and effective fingerprints. Chat/Completions/Responses/native decision paths share the existing routing engine; endpoint and signed-continuation restrictions remain authoritative. Native Files/Batches continue to require an explicit fixed provider/model rather than expanding an alias into autonomous jobs.
