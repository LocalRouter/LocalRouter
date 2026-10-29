<!-- @entry secret-scanning-overview -->

Secret Scanning inspects outbound requests for API keys, tokens, passwords, and other credentials before they are forwarded to LLM providers. This helps prevent accidental secret leakage when chat messages contain configuration snippets, code samples, or environment variables. The scanner runs locally with zero external dependencies and adds minimal latency to request processing.

System One decision requests (`POST /systemone`) are scanned the same way as chat messages, covering the `state`, question instructions, and option and level descriptions.

<!-- @entry secret-scan-pipeline -->

Detection uses a three-stage pipeline. First, an Aho-Corasick keyword pre-filter performs a fast scan of the message text to identify which rules are candidates (e.g., looking for prefixes like `ghp_`, `sk-`, `secret`). Only rules whose keywords match proceed to the second stage: full regex evaluation against the message content. Matched values then pass through entropy, placeholder, and structural checks. Specific credential rules take priority over generic assignments, and repeated values are reported once per request.

<!-- @entry secret-scan-categories -->

Detected secrets are grouped into seven categories: **Cloud Provider** (AWS secret access keys and session tokens, GCP service account keys, GCP API keys, Azure storage keys), **AI Service** (OpenAI, Anthropic, Groq, Cohere, HuggingFace keys), **Version Control** (GitHub PATs, GitHub OAuth/App tokens, GitHub fine-grained PATs, GitLab PATs), **Database** (PostgreSQL, MySQL, MongoDB, Redis connection URIs), **Financial** (Stripe secret and restricted keys), **OAuth** (Slack bot tokens, user tokens, webhook URLs), and **Generic** (PEM private keys, JWT tokens, Bearer tokens, and pattern-based detection of `api_key=`, `password=`, `secret=` assignments). Categories are used for display grouping and do not have independent action settings.

<!-- @entry secret-scan-actions -->

Secret scanning supports three actions: **Ask**, **Notify**, and **Off**. **Ask** blocks the request and presents a popup for the user to approve or reject before the request is sent. **Notify** allows the request to proceed but shows an alert about the detected secret. **Off** disables scanning entirely. The action is configured as a global default and can be overridden on a per-client basis, so trusted clients can bypass scanning while others require explicit approval.

<!-- @entry secret-scan-approval-flow -->

When the action is set to Ask, the firewall intercepts the request and displays a popup showing a preview of the detected finding, including the rule description, category, and a truncated view of the matched text (long values show the first 6 and last 4 characters; values of 16 characters or fewer are fully masked). Each detected secret is shown in its own blocking popup. The user can block, allow, or **Redact once**. Block stops the request immediately, so later secrets are not shown. Allow and Redact advance to the next secret; the request is forwarded only after every secret has a decision. Redaction replaces every occurrence of the currently displayed secret with `[REDACTED: secret removed by LocalRouter]` before continuing the request, including repeated values and decoded text attachments. Redact once asks again when the value appears in a later request.

**Always redact this secret**, in the Redact dropdown, redacts the current request and remembers that particular value for this client. Both dropdowns apply only to the secret currently shown. Only a randomly salted, per-client PBKDF2-HMAC-SHA256 hash is saved locally; the secret itself is never stored in the rule. Future matches are automatically redacted while scanning is enabled, including during a temporary approval bypass. New secret values still follow the client's Ask or Notify policy. Remembered values can be removed in the client's **Secret Scanning → Remembered Secrets** settings. **Always allow this secret**, in the Allow dropdown, remembers only the selected value as allowed for this client. Existing ignored values keep this behavior and appear as **Always allow** in settings.

**Allow for 1 Hour** temporarily bypasses approval for other findings. **Notify** popups report requests that have already proceeded, so they cannot offer redaction of that request.

<!-- @entry secret-scan-allowlist -->

The scanner can be tuned with several configuration options. The **entropy threshold** controls the minimum Shannon entropy (bits per character) required for a match to be reported; the default is 3.5, and values below this are treated as placeholders or example keys. **Allowlist regex patterns** let you exclude known-safe strings from detection, such as test keys or internal identifiers that match secret patterns. Each built-in rule can also carry a per-rule entropy override (e.g., database URIs use a threshold of 3.0 since connection passwords tend to have less randomness than API tokens). System messages are excluded from scanning by default but can be included via configuration. Detection is heuristic: arbitrary unlabelled strings, unsupported formats, and non-text media can contain secrets the scanner cannot recognize.


AWS access key IDs (`AKIA…`, `ASIA…`) are identifiers and do not trigger secret prompts, including in generic key or token assignments. Secret access keys are detected when labelled with common AWS environment, INI, JSON, SDK, or YAML field names (snake_case, camelCase, or kebab-case). Temporary AWS session/security tokens are detected by field name or `X-Amz-Security-Token`. These tokens can be long and contain repeated encoded runs; the scanner keeps the whole value for redaction. An unlabelled 40-character string is not automatically treated as an AWS secret.

HTTP Basic authorization headers are decoded locally and checked for a plausible password; redaction removes the complete encoded credential. Bearer tokens are captured in full without a fixed length cap. Generic API-key, token, secret, and password assignments support quoted values and unquoted `.env`, shell, and YAML values. Fields labelled public/publishable and metadata such as pagination cursors, token IDs, password hashes, and file references are excluded from generic rules. Tool-call arguments and results are scanned too, including structured Anthropic tool blocks. Secret-scan monitor excerpts are masked before storage, and plaintext findings are excluded from serialization and diagnostic formatting.
