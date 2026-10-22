- [x] Inspect the popup typography and identify the oversized secret style.
- [x] Match the secret value to the popup's regular 12px text, keeping it above the smallest 10px metadata.
- [x] Plan Review: verify masked and revealed secrets share the corrected size.
- [x] Test Coverage Review: update the existing size assertion and verify both masked and revealed secrets.
- [x] Bug Hunt: review wrapping and reveal controls for unintended changes.
- [x] Validate frontend types/build and required stable Rust workspace checks.
- [x] Commit and push the validated change and automatic catalog updates, preserving unrelated files.

Review complete: the shared finding value uses 12px for masked and revealed text, matching regular popup labels and exceeding 10px metadata. Wrapping, reveal controls, and approval behavior remain intact. Frontend production build, all 10 popup browser tests, and stable Rust formatting passed.

Stable rustc 1.99.1 was updated/verified and pinned via PATH and RUSTC with RUSTC_WRAPPER disabled. Workspace Clippy (-D warnings), formatting, and workspace tests passed. Tests required an unrestricted rerun because the sandbox blocked local mock-server ports and test backup files; the unrestricted suite passed. All 10 popup browser tests passed in installed Chrome. No catalog update was generated.
