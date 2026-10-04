# Wrapper startup recovery

- [x] Diagnose listener restoration and provider relocation across reboot.
- [x] Restore supported local providers before binding wrapper listeners at startup.
- [x] Keep manual providers actionable and avoid restarting healthy upstreams or relocating remote servers.
- [x] Add regression coverage for startup decisions and port recovery.
- [x] Plan review: verify implementation against every planned behavior.
- [x] Test coverage review: cover failure paths and startup edge cases.
- [x] Bug hunt: review races, listener ownership, and recovery errors.
- [x] Run stable workspace clippy, formatting, and tests.
- [ ] Build and install the updated macOS app; verify the local wrapper and startup recovery.
- [ ] Commit and push only task changes to the configured upstream.

macOS launchctl environment is session-only. Existing enabled wrappers should restore automatic relocation before startup listener binding; manual-only providers should expose provider-specific instructions. Recovery must avoid GUI elevation at startup and remote/non-loopback upstreams. Healthy relocated servers should be left running. Preserve the unrelated catalog modification.

## Findings and review

The installed app uses ~/.localrouter/settings.yaml. Its Ollama provider already pointed to localhost:11435, but a standalone /usr/local/bin/ollama serve process held 11434. The old relocation sequence only stopped the GUI. Restoring launchctl configuration, terminating that verified server, opening Ollama and restarting LocalRouter restored both endpoints: HTTP 200 for /api/version and /api/tags, five matching models.

Startup recovery uses each enabled wrapper's relocation plan and aligns linked provider entries. It skips remote and unsupported loopback addresses, leaves responding upstreams alone, and avoids unattended elevation. Manual servers retain their listeners so they can start later. Automatic recovery failures and listener collisions remain visible with setup guidance.

The macOS standalone-server stop is restricted to the original port's owner and verifies the command is ollama serve before signaling. Tests cover command selection and preservation of unrelated port owners, healthy upstreams, manual/elevated plans, relocation errors, successful offline startup, and local-address restrictions. Start listener now aligns provider entries for manual wrappers; retargeting failures abort setup. UI errors include details directly in the toast and retain a panel result; empty responses have a fallback.

## Validation

Stable rustc 1.99.0 was refreshed with rustup update stable. Stable workspace all-target Clippy passed with -D warnings, stable formatting passed, TypeScript noEmit passed, and the frontend production build passed. Workspace tests passed: 3,646 tests, 93 ignored, 107 completed suites (including documentation suites). The final focused wrapper run passed 28 tests, including the newly added unrelated-port-owner protection test.
