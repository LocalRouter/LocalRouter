- [x] Reproduce missed private-key detection across actual request extraction and encoded tool content using synthetic fixtures.
- [x] Audit built-in patterns and post-match filters for high-confidence misses and false positives.
- [x] Improve matching with bounded processing, exact redaction spans and no new request rejection paths.
- [x] Add positive/negative regression matrices, redaction/restoration round trips and relevant protocol tests.
- [x] Plan review: implement any missed requirements or edge cases.
- [x] Test coverage review: cover changed paths and errors.
- [x] Bug hunt: re-read detection, overlap handling, encoding and performance behavior.
- [x] Run stable Rust workspace tests, all-target Clippy and formatting, plus applicable UI checks.
- [ ] Commit signed changes and push the validated branch, preserving concurrent work.

Use only synthetic credentials, mock/file credential storage and isolated build artifacts. Never print private material from user logs or credential stores. Do not restart production or cut a release without a new request.

Audit evidence: locally generated Ed25519, RSA and ECDSA OpenSSH keys reproduced zero findings through nested JSON. Escaped slashes yielded partial matches, and encrypted legacy PEM was missed in all tested representations. The shared armor grammar now captures complete blocks, escaped line breaks/slashes and legacy encryption metadata. Key material is validated structurally instead of rejected for low entropy. Generic rules reject descriptive multiword camel/Pascal identifiers, retain long credentials beyond 256 characters, and no longer discard otherwise random values because of one short repeated/counting run. No value-specific exception or production-store mutation is included.

Tests include entire-block masking, raw/nested JSON, CRLF, slash escaping, legacy encrypted metadata, truncated keys, public certificates/keys, documentation placeholders, descriptive identifiers, incidental repetition and long credentials. AppFirewall tests exercise MCP tool results as they enter Codex Responses, OpenAI Chat and Anthropic Messages, plus exact restoration and no mapping for descriptive names. Existing mappings remain until purged; changing a heuristic does not silently delete saved originals.

The new production firewall regression exposed an additional Anthropic MCP-result gap: extraction inspected serialized JSON while masking parsed the nested JSON first, so the stored escaped match did not equal the value being masked. Normalize JSON text in Anthropic tool-result strings/parts and Responses string input through the shared extractor, matching the masking layer. Cover this with direct extractor tests and the three-protocol firewall round trip.

Final classification review: the Google service-account rule retains its PKCS#8/RSA label restriction while sharing the improved body grammar. OpenSSH keys in an arbitrary private_key JSON field use the general private-key rule instead of being mislabeled as Google credentials.

Manual generated-key validation passed all 16 combinations of Ed25519/RSA/ECDSA OpenSSH and encrypted traditional PEM keys with raw, JSON, nested JSON and escaped-slash forms, asserting a single exact complete-key span. Generated credentials are isolated fixtures, never read from the production store. Evidence: /tmp/localrouter-matching-evidence/generated-key-validation.log. No frontend, command schema, endpoint or UI changes are included, so frontend builds and hosted CI/release dispatches are not needed for this change.

Final validation passed on unchanged source: 5,156 workspace tests, zero failures, 86 ignored; focused scanner/extractor tests; explicit installed Codex standalone-search lifecycle regression; all-target Clippy with warnings denied; and formatting. The production firewall tests include exact MCP result masking/restoration for Codex Responses, Chat and Anthropic Messages. Matching changes affect model-bound content; this task does not add a new scanner on the raw local MCP response transport or remove existing stored mappings. Final evidence: /tmp/localrouter-matching-evidence/final/results.json and generated-key-validation.log. No native Keychain access, external model calls, frontend changes, hosted CI dispatch or release dispatch.
