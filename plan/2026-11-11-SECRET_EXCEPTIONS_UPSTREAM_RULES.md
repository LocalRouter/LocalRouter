- [x] Evaluate established local secret detectors/rule sources, licenses, embedding and matching semantics; select and pin the upstream source.
- [x] Implement permanent per-client false-positive decisions from stored-secret references using existing salted digests, preserving original mappings and in-flight restoration.
- [x] Make ignored stored originals stop masking immediately on subsequent requests, without weakening other clients or explicit redaction policies.
- [x] Add the Not a secret action, confirmation/state, types, command registration, demo behavior and remembered-rule management.
- [x] Embed the upstream detection rules offline with provenance/license, reproducible updates, supported allowlists and explicit compatibility policy.
- [x] Expand detection positives/negatives, storage/config/restart/concurrency/expiry and protocol/UI regression coverage.
- [x] Plan review: reconcile implementation against every requirement and fix missed cases.
- [x] Test coverage review: cover changed paths and error/recovery behavior.
- [x] Bug hunt: review false-positive precedence, in-flight capabilities, concurrency, bounded work and rule compatibility.
- [x] Run stable Rust/workspace tests, all-target Clippy and formatting; frontend unit/build and website checks; validate actual UI using isolated storage.
- [ ] Commit signed changes and push validated work, preserving concurrent work. No release dispatch without a new release request.

Keep all detection and decisions local. Do not call credential verification endpoints, access production Keychain or copy real secrets into logs. Use synthetic values with mock/file storage. Client-scoped permanent allow decisions use existing digest-only remembered rules; keep restoration capabilities until normal retention/purge rather than deleting them on false-positive dismissal.

Selection: Gitleaks v8.30.1 (MIT), pinned commit 83d9cd684c87d95d656c1458ef04895a7f1cbd8e. Source metadata reports about 31k stars. Embed the rule catalog in Rust, with no Go executable, runtime downloads or credential-verification calls. 201 rules are imported and all 21 exclusions/overrides have explicit reasons. Preserve RE2 ASCII classes/boundaries, braces, capture selection, per-rule entropy, and content allowlist targets/conditions. Compile immutable rules once across scanner instances.

Permanent decisions use existing per-client salted PBKDF2 digests. The UI resolves the selected reference locally, persists the allow rule, and keeps every original/reference. Active and subsequent vaults consult the current decisions; restoration continues to use the retained full map. Failed configuration saves roll back only the attempted decision, preserving concurrent updates and all mappings. Existing remembered rules provide undo; no decision TTL is introduced. Masked previews only pre-filter expensive KDF mismatches and never authorize a value without digest verification.

Manual UI validation on isolated website demo: confirmed Not a secret dialog, retained reference ID and unrelated-client entry, Reveal after the decision, remembered rule listing and undo, and enabled action after undo. Screenshot: /tmp/localrouter-upstream-rules-evidence/not-secret-ui.png. The UI test did not modify production data. Backend tests independently cover actual config save/reload, concurrency, client isolation, in-flight replay, policy reversal/removal, store purge and save failure.

Final local validation: `cargo test --workspace` passed 5,167 tests (86 ignored); the actual installed Codex local TLS/firewall lifecycle regression passed. All-target stable Clippy, formatting, 151 frontend unit tests, desktop build, website build/check and six Python script tests passed. After a mechanical helper-order lint fix, all 185 scanner tests and all-target Clippy passed again; reconstructing the previous helper order reproduced the exact workspace-validated source hash. The final demo-only npm example was followed by another successful website check. No production credentials, native Keychain access or paid LLM calls were used. The full MIT notice is bundled in both desktop and website assets.
