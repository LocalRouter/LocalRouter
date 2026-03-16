<!-- @entry secret-scanning-overview -->

Secret Scanning inspects outbound requests for API keys, tokens, passwords, and other credentials before they are forwarded to LLM providers. This prevents accidental secret leakage when chat messages contain configuration snippets, code samples, or environment variables. The scanner runs locally with zero external dependencies and adds minimal latency to request processing.

<!-- @entry secret-scan-pipeline -->

Detection uses a three-stage pipeline. First, an Aho-Corasick keyword pre-filter performs a fast scan of the message text to identify which rules are candidates (e.g., looking for prefixes like `ghp_`, `sk-`, `AKIA`). Only rules whose keywords match proceed to the second stage: full regex evaluation against the message content. Finally, matched text is passed through a Shannon entropy filter that discards low-entropy placeholder values (like `AKIAIOSFODNN7EXAMPLE`) while retaining high-entropy strings that indicate real secrets.

<!-- @entry secret-scan-categories -->

Detected secrets are grouped into seven categories: **Cloud Provider** (AWS access keys, GCP service account keys, GCP API keys, Azure storage keys), **AI Service** (OpenAI, Anthropic, Groq, Cohere, HuggingFace keys), **Version Control** (GitHub PATs, GitHub OAuth/App tokens, GitHub fine-grained PATs, GitLab PATs), **Database** (PostgreSQL, MySQL, MongoDB, Redis connection URIs), **Financial** (Stripe secret and restricted keys), **OAuth** (Slack bot tokens, user tokens, webhook URLs), and **Generic** (PEM private keys, JWT tokens, Bearer tokens, and pattern-based detection of `api_key=`, `password=`, `secret=` assignments). Categories are used for display grouping and do not have independent action settings.

<!-- @entry secret-scan-actions -->

Secret scanning supports three actions: **Ask**, **Notify**, and **Off**. **Ask** blocks the request and presents a popup for the user to approve or reject before the request is sent. **Notify** allows the request to proceed but shows an alert about the detected secret. **Off** disables scanning entirely. The action is configured as a global default and can be overridden on a per-client basis, so trusted clients can bypass scanning while others require explicit approval.

<!-- @entry secret-scan-approval-flow -->

When the action is set to Ask, the firewall intercepts the request and displays a popup showing a preview of the detected finding, including the rule description, category, and a truncated view of the matched text (first 6 and last 4 characters visible, middle masked). The user can approve or reject the request. Approved requests include a time-based bypass so that repeated matches from the same conversation do not require re-approval within the bypass window.

<!-- @entry secret-scan-allowlist -->

The scanner can be tuned with several configuration options. The **entropy threshold** controls the minimum Shannon entropy (bits per character) required for a match to be reported; the default is 3.0, and values below this are treated as placeholders or example keys. **Allowlist regex patterns** let you exclude known-safe strings from detection, such as test keys or internal identifiers that match secret patterns. Each built-in rule can also carry a per-rule entropy override (e.g., database URIs use a lower threshold of 2.5 since connection passwords tend to have less randomness than API tokens). System messages are excluded from scanning by default but can be included via configuration.
