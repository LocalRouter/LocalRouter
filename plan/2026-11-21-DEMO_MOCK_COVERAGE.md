# Demo mock coverage

The website demo embeds the real frontend from `src/` and answers Tauri
`invoke` calls from `mockHandlers`
(`website/src/components/demo/TauriMockSetup.ts`). A command without a handler
shows a "Demo: not implemented" toast and returns `null`, which breaks the
view that asked for it. Nothing compared the three command sets, so the gap
could open silently.

## Goal

Every command the frontend can invoke has a typed demo mock, no mock survives
its command, and a unit test keeps it that way.

## Measured state

`node scripts/check-demo-mock-coverage.mjs` (TypeScript-AST based, see below).

| | Before | After |
|---|---|---|
| Registered (`generate_handler!`) | 471 | 471 |
| Invoked by the frontend | 382 | 381 |
| Mocked | 416 | 413 |
| Invoked but unmocked | 8 | 0 |
| Invoked but not registered | 1 (`get_client`) | 0 |
| Mocked but not registered (stale) | 11 | 0 |
| Registered, never invoked by the frontend | 90 | 90 |
| Mocked and registered, never invoked | 32 | 32 |
| `invoke` call sites with an unresolvable name | 0 | 0 |

Unmocked before: `get_server_status`, `get_sampling_approval_details`,
`submit_sampling_approval`, `get_elicitation_details`,
`submit_elicitation_response`, `cancel_elicitation`,
`debug_trigger_sampling_approval_popup`, `debug_trigger_elicitation_form_popup`.

Stale before: `add_skills_path`, `remove_skills_path`, `copy_to_clipboard`,
`get_client`, `get_marketplace_config`, `get_mcp_server`,
`get_provider_instance`, `show_main_window`, `hide_main_window`,
`search_marketplace`, `test_safety_check_single_model`.

## Checklist

- [x] Save this plan
- [x] `scripts/check-demo-mock-coverage.mjs`: registered / invoked / mocked sets
  - registered: the `generate_handler![...]` list, Rust comments stripped
  - mocked: keys of the `mockHandlers` object literal, following spreads into
    sibling modules (`mockReplay`), `plugin:*` keys excluded
  - invoked: `invoke(...)` calls bound to `@tauri-apps/api/core`, parsed with
    the TypeScript compiler API; names resolved through literals, ternaries,
    constants, reassigned `let`s, returned objects (`metrics-chart.tsx`) and
    option-object wrappers (`useModelDownload`); anything unresolved is
    reported as a dynamic call site and fails the check
- [x] Mocks for the eight unmocked commands, typed from
      `src/types/tauri-commands.ts`, stateful where the action has an effect
  - [x] `get_server_status` follows `healthCache.server_running`; start/stop
        emit `server-status-changed` as the backend does
  - [x] sampling approval and elicitation: pending requests kept in memory,
        created by the debug triggers, consumed by submit/cancel
- [x] Response and params types for those commands in `tauri-commands.ts`;
      the two popup views use them instead of private copies
- [x] `firewall-approval.tsx` reads the client from `list_clients`; it called
      `get_client`, which is not a registered command, so the model dropdown
      of the approval popup was always empty
- [x] Remove the eleven stale mocks
- [x] Unit test `tests/e2e/unit/demo-mock-coverage.spec.ts`, with the commented
      allowlist `DELIBERATELY_UNMOCKED` in the script (empty)
- [x] Registered-but-never-invoked list with usage notes (report only; no Rust
      changes in this task)
- [x] Validation: `npx tsc --noEmit`, `npm run test:unit`, `npm run build`,
      `npm --prefix website run build:check`
- [x] Plan Review: compare this plan with the implementation, implement
      anything missed
- [x] Test Coverage Review: tests for new paths and edge cases
- [x] Bug Hunt: re-read the new code looking for defects
- [x] Commit and push `fix/demo-mock-coverage`

## Out of scope

- Deleting registered-but-unused Rust commands (handled separately).
- The 32 mocks for commands that exist but that the frontend no longer
  invokes: they are not stale, and go away with their commands.
- `/Users/matus/dev/winXP`.
