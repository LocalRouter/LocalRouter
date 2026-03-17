# Fix: IndexRead should activate deferred tools/resources/prompts

## Context

When catalog compression defers tools/resources/prompts, clients discover them via `IndexSearch` which automatically activates any deferred items that appear in search results (sends `tools/list_changed`, updates session state). However, **`IndexRead` does NOT activate deferred items** — if a client reads a deferred tool's indexed content directly via `IndexRead`, the tool remains hidden from `tools/list`.

This is a gap: a client could `IndexSearch` → get a hit for a deferred tool → call `IndexRead` to read its full definition → but the tool stays deferred if search didn't already activate it (e.g., if the search hit was for a server welcome, not the tool itself, or if the client calls `IndexRead` with a known label directly).

## Approach

Modify `handle_index_read_blocking` in `context_mode.rs` to check whether the label being read corresponds to a deferred catalog item, and if so, activate it (same pattern as `handle_ctx_search_blocking`).

## Files to modify

**`crates/lr-mcp/src/gateway/context_mode.rs`**

### 1. Update `handle_index_read_blocking` signature

Add `catalog_sources`, `activated_tools`, `activated_resources`, `activated_prompts` parameters (same as `handle_ctx_search_blocking`).

### 2. Add activation logic to `handle_index_read_blocking`

After a successful read, check if `label` exists in `catalog_sources`. If it does and the item is not already activated, return `SuccessWithSideEffects` with the same activation pattern used in search.

```rust
fn handle_index_read_blocking(
    store: &ContentStore,
    arguments: Value,
    catalog_sources: &HashMap<String, CatalogItemType>,
    activated_tools: &HashSet<String>,
    activated_resources: &HashSet<String>,
    activated_prompts: &HashSet<String>,
) -> VirtualToolCallResult {
    // ... existing label/offset/limit parsing ...

    match store.read(&label, offset.as_deref(), limit) {
        Ok(read_result) => {
            let formatted = read_result.to_string();
            let result = json!({
                "content": [{"type": "text", "text": formatted}]
            });

            // Check if the read label is a catalog item that needs activation
            if let Some(item_type) = catalog_sources.get(&label) {
                let name = extract_item_name_from_source(&label);
                let already_active = match item_type {
                    CatalogItemType::Tool => activated_tools.contains(name),
                    CatalogItemType::Resource => activated_resources.contains(name),
                    CatalogItemType::Prompt => activated_prompts.contains(name),
                    CatalogItemType::ServerWelcome => true,
                };
                if !already_active {
                    let name_owned = name.to_string();
                    let item_type_clone = item_type.clone();
                    let state_update: Box<dyn FnOnce(&mut dyn super::virtual_server::VirtualSessionState) + Send> =
                        Box::new(move |s| {
                            if let Some(cm) = s.as_any_mut().downcast_mut::<ContextModeSessionState>() {
                                match item_type_clone {
                                    CatalogItemType::Tool => { cm.activated_tools.insert(name_owned); }
                                    CatalogItemType::Resource => { cm.activated_resources.insert(name_owned); }
                                    CatalogItemType::Prompt => { cm.activated_prompts.insert(name_owned); }
                                    CatalogItemType::ServerWelcome => {}
                                }
                            }
                        });
                    return VirtualToolCallResult::SuccessWithSideEffects {
                        response: result,
                        invalidate_cache: true,
                        send_list_changed: true,
                        state_update: Some(state_update),
                    };
                }
            }

            VirtualToolCallResult::Success(result)
        }
        Err(e) => VirtualToolCallResult::ToolError(format!("Read failed: {}", e)),
    }
}
```

### 3. Update the call site (line ~332)

Pass the additional parameters through to `handle_index_read_blocking`:

```rust
} else if tool == read_name {
    handle_index_read_blocking(
        &store,
        arguments,
        &catalog_sources,
        &activated_tools,
        &activated_resources,
        &activated_prompts,
    )
}
```

### 4. Add tests

Add test cases in the existing `handle_index_read_blocking` test section (~line 1284):
- Read a catalog-source label → verify `SuccessWithSideEffects` returned
- Read a catalog-source label that's already activated → verify plain `Success`
- Read a non-catalog label (e.g., response data) → verify plain `Success`

## Verification

1. `cargo test -p lr-mcp` — run existing + new tests
2. `cargo clippy -p lr-mcp` — no warnings
3. Manual: connect a client with context management, verify that calling `IndexRead` on a deferred tool's label activates it (tool appears in next `tools/list`)
