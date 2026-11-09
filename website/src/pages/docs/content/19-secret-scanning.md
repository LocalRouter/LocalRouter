<!-- @entry secret-scanning-overview -->

Secret Scanning inspects outbound requests for API keys, tokens, passwords, and other credentials before they are forwarded to LLM providers. This helps prevent accidental secret leakage when chat messages contain configuration snippets, code samples, or environment variables. The scanner runs locally with zero external dependencies and adds minimal latency to request processing.

System One decision requests (`POST /systemone`) are scanned the same way as chat messages, covering the `state`, question instructions, and option and level descriptions.

<!-- @entry secret-scan-pipeline -->

Detection uses a three-stage pipeline. First, an Aho-Corasick keyword pre-filter performs a fast scan of the message text to identify which rules are candidates (e.g., looking for prefixes like `ghp_`, `sk-`, `secret`). Only rules whose keywords match proceed to the second stage: full regex evaluation against the message content. Matched values then pass through entropy, placeholder, and structural checks. Specific credential rules take priority over generic assignments, and repeated values are reported once per request.

<!-- @entry secret-scan-categories -->

Detected secrets are grouped into seven categories: **Cloud Provider** (AWS secret access keys and session tokens, GCP service account keys, GCP API keys, Azure storage keys), **AI Service** (OpenAI, Anthropic, Groq, Cohere, HuggingFace keys), **Version Control** (GitHub PATs, GitHub OAuth/App tokens, GitHub fine-grained PATs, GitLab PATs), **Database** (PostgreSQL, MySQL, MongoDB, Redis connection URIs), **Financial** (Stripe secret and restricted keys), **OAuth** (Slack bot tokens, user tokens, webhook URLs), and **Generic** (PEM private keys, JWT tokens, Bearer tokens, and pattern-based detection of `api_key=`, `password=`, `secret=` assignments). Categories are used for display grouping and do not have independent action settings.

<!-- @entry secret-scan-actions -->

Secret scanning supports four actions: **Redact**, **Ask**, **Notify**, and **Off**. **Redact** automatically replaces detected values with restorable, OS-stored secret references on supported text gateway and active native proxy requests. **Ask** presents a popup for a decision. Only an explicit Deny blocks the request; timeout, dismissal or unavailable approval proceeds with best-effort redaction. **Notify** allows the request to proceed but shows an alert about the detected secret. **Off** disables scanning entirely. The action is configured as a global default and can be overridden on a per-client basis, so trusted clients can bypass scanning while others prompt for decisions with a nonblocking fallback.

<!-- @entry secret-scan-approval-flow -->

When the action is set to Ask, the firewall intercepts the request and displays a popup showing a preview of the detected finding, including the rule description, category, and a truncated view of the matched text (long values show the first 6 and last 4 characters; values of 16 characters or fewer are fully masked). Detected secrets are presented individually within a request-wide approval deadline. The user can block, allow, or **Redact once**. Block stops the request immediately, so later secrets are not shown. Allow and Redact advance to the next secret; the request proceeds after the decisions or approval fallback. Secret Scan approval waits at most 60 seconds for the entire request; an unavailable or dismissed approval uses best-effort redaction. Redaction replaces every occurrence of the currently displayed secret with `[REDACTED: secret removed by LocalRouter]` before continuing the request, including repeated values and decoded text attachments. Redact once asks again when the value appears in a later request.

**Always redact this secret**, in the Redact dropdown, redacts the current request and remembers that particular value for this client. Both dropdowns apply only to the secret currently shown. Only a randomly salted, per-client PBKDF2-HMAC-SHA256 hash is saved locally; the secret itself is never stored in the rule. Future matches are automatically redacted while scanning is enabled, including during a temporary approval bypass. New secret values follow the client's selected scanning action. Remembered values can be removed in the client's **Secret Scanning → Remembered Secrets** settings. Notify notifications also offer **Always dismiss this secret** in the Dismiss dropdown. It remembers only the selected value for this client and closes the notification; other secret values continue to notify. **Always allow this secret**, in the Ask popup's Allow dropdown, remembers only the selected value as allowed for this client. Existing ignored values keep this behavior and appear as **Always allow** in settings.

**Allow for 1 Hour** temporarily bypasses approval for other findings. **Notify** popups report requests that have already proceeded, so they cannot offer redaction of that request.

<!-- @entry secret-scan-allowlist -->

The scanner can be tuned with several configuration options. The **entropy threshold** controls the minimum Shannon entropy (bits per character) required for a match to be reported; the default is 3.5, and values below this are treated as placeholders or example keys. **Allowlist regex patterns** let you exclude known-safe strings from detection, such as test keys or internal identifiers that match secret patterns. Each built-in rule can also carry a per-rule entropy override (e.g., database URIs use a threshold of 3.0 since connection passwords tend to have less randomness than API tokens). System messages are excluded from scanning by default but can be included via configuration. Detection is heuristic: arbitrary unlabelled strings, unsupported formats, and non-text media can contain secrets the scanner cannot recognize.


AWS access key IDs (`AKIA…`, `ASIA…`) are identifiers and do not trigger secret prompts, including in generic key or token assignments. Secret access keys are detected when labelled with common AWS environment, INI, JSON, SDK, or YAML field names (snake_case, camelCase, or kebab-case). Temporary AWS session/security tokens are detected by field name or `X-Amz-Security-Token`. These tokens can be long and contain repeated encoded runs; the scanner keeps the whole value for redaction. An unlabelled 40-character string is not automatically treated as an AWS secret.

HTTP Basic authorization headers are decoded locally and checked for a plausible password; redaction removes the complete encoded credential. Bearer tokens are captured in full without a fixed length cap. Generic API-key, token, secret, and password assignments support quoted values and unquoted `.env`, shell, and YAML values. Fields labelled public/publishable and metadata such as pagination cursors, token IDs, password hashes, and file references are excluded from generic rules. Tool-call arguments and results are scanned too, including structured Anthropic tool blocks. Monitor shows each finding’s type, outcome, replacement reference and nearby context. Values selected for redaction are masked before event storage; Notify and allowed findings remain visible locally. Raw scanner objects exclude plaintext from their default serialization and diagnostic formatting.

### Restorable redaction

Choose **Redact** to automatically replace detected secrets with an OS-stored reference on supported text gateway and active native proxy requests. **Ask** offers **Use placeholder** for a finding; **Redact once** remains irreversible. Off, Notify and existing per-client inheritance retain their meanings.

Native HTTP proxies inspect Codex's zstd-compressed requests before scanning. gzip, HTTP deflate and Brotli are also supported, with bounded decoding. Default Codex freeform tools and MCP tool namespaces can coexist with redaction. Hosted tools receive placeholders; originals are restored when the response returns locally. Supported JSON and code-string escaping, Unicode escapes, HTML entities, and URL percent encoding are preserved through bounded nested layers. Both `localrouter-redacted` and `localrouter-secret` references are recognized. Unsupported binary transports pass through unchanged with a Secret Scan warning. Unresolved, malformed, or unsupported reference encodings remain unchanged; a **Secret Not Restored** diagnostic appears under Secret Scan without failing the response. Local response-schema validation limitations also produce diagnostics instead of rejecting unaffected calls.

```xml
<localrouter-redacted id='abc123' usage='Unmasked when returned verbatim'/>
```

The ID above is shortened for illustration. LocalRouter restores a returned reference in response text or tool arguments, including streaming responses. Tag names, ID attribute names and IDs match case-insensitively; `usage` may be omitted. No extra system instruction is added. References are scoped to the authenticated client and remain stable across turns and LocalRouter restarts for fourteen days of client inactivity. Conversation identity is not required. Release builds use the native credential store; debug builds use a separate development file by default. Permitted tools and the requesting client receive restored values; the model receives the reference.

Read [Reversible Secret Redaction](/research/reversible-secret-redaction) for the algorithm, storage boundaries, tests and limitations. Automatic redaction does not inspect every possible binary or encoded secret and does not grant tool permissions.

The popup keeps the selected secret above separate **Event** and **JWT** tabs. **Event** starts with readable context around the match; **View full event** opens the complete request in its own scrolling viewer. **JWT** opens decoded header and payload data independently of **Reveal secret**. The token stays masked until you reveal it.

Secret Scan includes **Secret Redacted** and **Secret Restored** Monitor events in the same feature filter. They share the parent request and conversation, and show reference IDs, locations and replacement counts. Originals stay masked in the event; explicit Reveal retrieves an available owned mapping locally.

Open **Secret Scan → Redacted secrets**, or a client's Secret Scanning settings, to inspect stored references, masked values and expiry. Browsing does not extend retention; client requests and explicit reveals refresh client activity. Purge one secret, all secrets for a client, or the whole store using the confirmation dialog. Purging removes the original and its legacy aliases, leaves existing placeholders unable to resolve, and prevents older requests from recreating the purged value. Purging one entry preserves unrelated in-flight references; purging a whole client invalidates its older vaults. New requests may create fresh references. If native removal fails, affected references are revoked and remain visible as purge pending for retry. These stored originals are separate from remembered allow/dismiss/redact rules.

## Best-effort availability

Secret Scan internal failures never reject an LLM call. Scanner outages, storage errors, unsupported content, processing limits, signed content, schema setup errors and restoration problems produce local diagnostics while content continues. Content that could not be protected may reach the provider. The only Secret Scan 403 is an explicit user Deny, identified by `secret_scan_denied_by_user`.

If OS credential storage becomes unavailable, new mappings use client-scoped memory for the rest of that app run. They retain the client inactivity policy but cannot survive restart. The store UI reports incomplete persistence and never claims a durable purge succeeded when it cannot confirm deletion. Repeated native storage attempts are suppressed during that run to avoid recurring permission prompts.
