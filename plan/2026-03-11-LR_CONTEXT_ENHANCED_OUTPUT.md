# lr-context: Enhanced Output Formatting, Long Line Protection, Batch Operations

## Summary
- Smart truncation utility (60% head + 40% tail)
- Read: long line protection with sub-line offsets, output cap
- Index: rich TOC summary with pruning
- Search: multi-match window snippets, line-numbered output, output cap
- Search: combined query + queries entry point
- Chunk density enforcement
- Batch search+read operation

## Files Modified
- `crates/lr-context/src/truncate.rs` — NEW: smart truncation
- `crates/lr-context/src/types.rs` — LineOffset, ChunkToc, updated structs/Display
- `crates/lr-context/src/chunk.rs` — density enforcement
- `crates/lr-context/src/search.rs` — multi-match snippets, line-numbered output
- `crates/lr-context/src/lib.rs` — read/search/batch APIs, index TOC
