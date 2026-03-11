# Plan: `lr-context` — Native Content Indexing, Search & Read

## Summary

Reimplement the core indexing, search, and content retrieval capabilities from the Node.js
`context-mode` process as a standalone Rust crate (`lr-context`). Provides three operations:
**index**, **search**, and **read** using SQLite FTS5 with BM25 ranking.

## Key Decisions

- In-memory SQLite, session-scoped, ephemeral
- Three-layer search: Porter stemming → Trigram substring → Fuzzy correction
- Store original content in `sources` table for `read()` pagination
- `parking_lot::Mutex` (sync only, never across `.await`)
- Standalone crate with zero coupling to other lr-* crates

## Files

| File | Purpose |
|------|---------|
| `crates/lr-context/Cargo.toml` | Crate manifest |
| `crates/lr-context/src/lib.rs` | ContentStore, schema, public API |
| `crates/lr-context/src/types.rs` | All type definitions + Display impls |
| `crates/lr-context/src/chunk.rs` | Chunking: markdown, plain text, JSON |
| `crates/lr-context/src/search.rs` | Three-layer FTS5 search |
| `crates/lr-context/src/fuzzy.rs` | Levenshtein distance, vocabulary helpers |

## Reference

- Port of `/Users/matus/dev/context-mode/src/store.ts`
- Follows `Arc<Mutex<Connection>>` pattern from `crates/lr-monitoring/src/storage.rs`
