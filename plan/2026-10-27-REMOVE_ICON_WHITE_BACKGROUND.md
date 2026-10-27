# Remove icon white backgrounds

- [x] Remove the shared ServiceIcon dark-mode white background while preserving icon sizing and fallback behavior.
- [x] Plan Review: confirm all ServiceIcon image sources use the transparent wrapper.
- [x] Test Coverage Review: inspect existing coverage and run frontend type checking; this styling-only change needs no new logic test.
- [x] Bug Hunt: review the diff for layout and fallback regressions.
- [x] Run required stable Rust CI parity checks.
- [x] Commit and push the validated change and any automatic catalog updates, preserving unrelated files.

Validation: TypeScript checking, both existing service-icon tests, stable workspace Clippy, formatting, and diff checks passed. Full stable workspace tests ran but stopped at seven unrelated metrics_integration_tests failures (expected one time bucket, received two). No Rust code changed. Plan and bug review confirmed the wrapper is transparent for bundled and discovered images, with sizing, spacing, and fallback behavior preserved.
