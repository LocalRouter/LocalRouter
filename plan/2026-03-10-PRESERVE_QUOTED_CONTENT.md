# Preserve Quoted & Code Content During Compression

**Date**: 2026-03-10
**Status**: In Progress

## Goal
Add protection layer to LLMLingua-2 compression that force-keeps words inside quoted strings and code blocks, plus an optional `[abridged]` compression notice prefix.

## Approach
- Word-level state machine detects quoted/code regions after BERT scoring
- Protected words are force-kept regardless of BERT score (no budget adjustment)
- Supports 14+ delimiter types including Unicode quotes, guillemets, CJK corners
- Compression notice optionally prepends `[abridged]` to compressed messages

## Files Modified
1. `crates/lr-compression/src/protection.rs` - New: detection function + tests
2. `crates/lr-compression/src/lib.rs` - Register module
3. `crates/lr-compression/src/model.rs` - Add protected_mask to selection logic
4. `crates/lr-compression/src/engine.rs` - Thread preserve_quoted + compression_notice
5. `crates/lr-config/src/types.rs` - Config fields
6. `crates/lr-server/src/routes/chat.rs` - Pass new params through pipeline
7. `src-tauri/src/ui/commands.rs` - test_compression params
8. `src/types/tauri-commands.ts` - TypeScript types
9. `src/views/compression/index.tsx` - UI toggles + visualization
10. `src/views/clients/tabs/compression-tab.tsx` - Per-client overrides
11. `website/src/components/demo/TauriMockSetup.ts` - Demo mock
