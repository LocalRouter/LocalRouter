<!-- @entry content-safety-scanning -->

GuardRails scans request and response content for safety threats before forwarding to providers or returning to clients. The scanning pipeline runs multiple detectors in parallel, checking for prompt injection, jailbreak attempts, PII leakage, and code injection.

If any detector flags content above its confidence threshold, the request is blocked with a descriptive error. Scanning adds minimal latency since detectors run concurrently.

<!-- @entry detection-types -->

GuardRails supports four primary detection categories, each targeting a different class of safety threat. Multiple detectors can be active simultaneously, and each has independent sensitivity thresholds.

Detection results include a confidence score and the specific pattern or rule that triggered the alert.

<!-- @entry detect-prompt-injection -->

Prompt injection detection identifies attempts to override system instructions or manipulate the model's behavior through crafted user input. Detectors look for patterns like "ignore previous instructions", role-switching attempts ("you are now..."), delimiter injection (closing/reopening system blocks), and encoded payloads.

Both direct injection (in user messages) and indirect injection (in tool results or retrieved content) are scanned.

<!-- @entry detect-jailbreak -->

Jailbreak detection identifies attempts to bypass the model's safety training and content policies. This includes techniques like DAN (Do Anything Now) prompts, character roleplay that removes restrictions, hypothetical framing ("in a fictional world where..."), and multi-turn manipulation.

Detectors use pattern matching on known jailbreak templates and heuristic analysis of prompt structure.

<!-- @entry detect-pii -->

PII (Personally Identifiable Information) detection scans content for sensitive personal data that should not be sent to external LLM providers. Detected categories include email addresses, phone numbers, social security numbers, credit card numbers, physical addresses, and names.

When PII is detected, the request can be blocked entirely or the PII can be redacted before forwarding, depending on configuration.

<!-- @entry detect-code-injection -->

Code injection detection identifies potentially dangerous code patterns in prompts or responses. This includes SQL injection patterns (`'; DROP TABLE`), shell command injection (`; rm -rf /`), script injection (`<script>` tags), and path traversal attempts (`../../etc/passwd`).

This is particularly important when LLM responses are used to generate or execute code downstream.

<!-- @entry detection-sources -->

GuardRails supports multiple detection backends that can be combined for defense-in-depth. Each source provides different detection capabilities and trade-offs between speed, accuracy, and coverage.

<!-- @entry source-builtin -->

The built-in detection engine uses hand-crafted regex patterns and heuristic rules compiled into LocalRouter. It runs entirely locally with zero latency overhead and no external dependencies.

Coverage focuses on common, well-known attack patterns with high precision (low false positives). Rules are updated with each LocalRouter release.

<!-- @entry source-presidio -->

Microsoft Presidio integration provides enterprise-grade PII detection using named entity recognition (NER) models. When configured, LocalRouter sends content to a local Presidio instance for analysis.

Presidio supports 30+ PII entity types across multiple languages and can be customized with domain-specific recognizers. Requires running Presidio as a separate service (Docker container recommended).

<!-- @entry source-llm-guard -->

LLM Guard integration connects to the LLM Guard service for ML-based content scanning. LLM Guard provides neural network-based detectors for prompt injection, jailbreaks, toxicity, and other threats that are difficult to catch with regex patterns alone.

It runs as a separate service and provides higher accuracy than pattern-based detection at the cost of additional latency (~50-100ms per scan).

<!-- @entry custom-regex-rules -->

Custom regex rules allow you to define your own content patterns to detect or block. Each rule specifies a regex pattern, a severity level, and an action (block, warn, or redact). Rules are evaluated against both request and response content.

This is useful for organization-specific policies — for example, blocking requests that mention internal project names, detecting proprietary code patterns, or flagging specific terminology.

<!-- @entry parallel-scanning -->

All configured detection sources run in parallel for each request, minimizing the impact on request latency. The scanning pipeline uses Tokio's async tasks to execute all detectors concurrently and collects results as they complete.

The overall scan time equals the slowest individual detector rather than the sum of all detectors. If any detector exceeds a configurable timeout, it is skipped and the request proceeds with partial scan results.
