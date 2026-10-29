- [x] Audit detection rules, filtering, request extraction, approval, redaction, and privacy paths.
- [x] Stop flagging AWS access key IDs; improve AWS secret access key and session-token detection across common config formats.
- [x] Improve generic credential assignments and false-positive handling, address confirmed masking/privacy defects, and synchronize examples/docs/demo fixtures.
- [x] Add regression coverage for positive/negative cases, complete-value redaction, repeated findings, and safe diagnostic output.
- [x] Plan Review: compare implementation with intended behavior and address missed cases.
- [x] Test Coverage Review: cover new branches, boundary conditions, and error handling.
- [x] Bug Hunt: reread detection, masking, and redaction with particular attention to missed secrets and accidental identifier prompts.
- [x] Validate TypeScript and exact stable workspace Clippy, formatting and tests with stable compiler pinned after rustup update stable.
- [x] Commit and push task changes and automatic catalog updates to the current upstream, preserving unrelated changes.

Scope: scan locally, avoid validating credentials against services, preserve configured action/system-message/remembered-secret policies. Unlabelled AWS secret material has no unique prefix; require contextual evidence rather than flagging every random string. Keep false-positive regressions from prior improvements.


Review findings and implementation:
- Removed the AWS ID rule and exempted AWS IDs from generic matches. Added AWS secret-key spelling variants and contextual, uncapped session-token detection. STS encoded runs no longer trigger random-token placeholder heuristics.
- Added unquoted generic assignments, consistent metadata/public-field filtering, a viable entropy floor for eight-character passwords, HTTP Basic decoding/validation, and complete long Bearer capture.
- Replaced placeholder substring checks with component boundaries so random credentials containing words such as test/your/here remain detectable. Existing documented false positives stay covered.
- Resume regex matching at the captured value's end so shared delimiters do not lose adjacent assignments.
- Added Anthropic structured tool inputs and text tool results without scanning image payloads.
- Fully masked short previews, removed plaintext from finding/approval Debug output, and masked secret-scan monitor excerpts before remembered-secret filtering. Existing historical monitor records and other request-monitor settings are not modified.
- Synchronized the desktop sample, native sample popup, website mock/preview, browser fixtures, and documentation. Existing action, system-message, bypass, salted remembered-value, and redaction policies are preserved.
- Coverage now includes AWS identifiers/config spellings, long STS/Bearer values, generic bare delimiters, encoded arguments, Basic valid/invalid/default values, every vendor pattern's positive path, placeholder substrings, adjacency, Unicode/short masking, debug/serde privacy, tool extraction, and repeated-value redaction.

Known detection boundary: unlabelled random strings have no reliable AWS secret signature; detection remains contextual/heuristic. Existing deliberate default/placeholder suppression remains in place.

Final bug-hunt addition: render structured Anthropic tool-input field context while preserving raw string characters. JSON serialization had escaped multiline PEM material, preventing redaction of the original JSON string value. A regression now asserts the captured value equals the original PEM, the popup masks its material, and redaction replaces the entire original value.

An initial workspace run hit an unrelated lr-engines supervisor fake-process bind error (Address already in use). That test passed in the retry; final validation reruns the entire workspace on the completed implementation.

Final validation: rustup stable 1.99.1 with stable sysroot bin prepended to PATH and explicit stable RUSTC; full workspace Clippy and formatting passed. Full workspace tests passed: 4638 passed, 0 failed, 69 ignored (including doc tests). Both frontend production builds passed; desktop TypeScript, 106 frontend unit checks, and all 14 secret approval/redaction browser regressions passed. Browser checks used installed Google Chrome after the default Playwright browser was unavailable. Unrelated release-plan files are preserved. No catalog snapshot change was generated.
