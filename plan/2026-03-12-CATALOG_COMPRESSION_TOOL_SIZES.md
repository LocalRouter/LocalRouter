# Include Tool Definition Sizes in Catalog Compression

## Summary
Redesign catalog compression to include tool definition sizes in threshold calculations,
use synchronous indexing with summary+TOC output, change paths from `catalog:` to `mcp/`,
and implement a 4-phase compression algorithm.

## Key Changes
1. `IndexResult`: add `summary()` and `toc(max_depth)` methods
2. New `batch_index` method + `BatchIndexResult` on ContentStore
3. Add `item_definition_sizes` to `InstructionsContext`
4. Helper functions: `compute_item_definition_sizes`, `format_tool_as_markdown`, `compress_tool_definition`
5. Redesign `CatalogCompressionPlan` with 4 phases
6. Move indexing to synchronous (before compression plan)
7. Restructure compression algorithm (4 phases)
8. Rewrite `build_context_managed_instructions`
9. Update path scheme from `catalog:` to `mcp/`
10. Update preview command with new fields
11. Update TypeScript types + frontend
12. Update mocks

## Date: 2026-03-12
