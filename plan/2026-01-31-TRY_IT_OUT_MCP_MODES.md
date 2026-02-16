# Plan: MCP & Skill Selection Radio Buttons in Try It Out

## Goal
Change the MCP tab's target selection from a single dropdown to radio buttons matching the LLM tab pattern, with three modes: "Against Client", "All MCPs & Skills", "Direct MCP/Skill". Rename "MCP" to "MCP & Skill" where appropriate.

## File to Modify
- `src/views/try-it-out/mcp-tab/index.tsx`

## Changes

### 1. Add imports
- Import `RadioGroup`, `RadioGroupItem` from `@/components/ui/radio-group`
- Import `Users`, `Globe`, `Zap` icons from lucide-react (matching LLM tab pattern)

### 2. Add mode state and client data
- Add `type McpTestMode = "client" | "all" | "direct"`
- Add state: `mode`, `clients`, `selectedClientId`, `clientApiKey`
- Fetch clients list on init (reuse `list_clients` Tauri command, same as LLM tab)
- Fetch client API key when client changes (via `get_client_value` command, same pattern as LLM tab)

### 3. Replace Select dropdown with RadioGroup
Replace the current `<Select>` target selector with:
```
RadioGroup with 3 options:
  - "client" → "Against Client" (Users icon)
  - "all" → "All MCPs & Skills" (Globe icon)
  - "direct" → "Direct MCP/Skill" (Zap icon)
```

### 4. Add mode-specific selectors below radio buttons
- **"client" mode**: Show client Select dropdown (same pattern as LLM tab)
- **"all" mode**: No additional selector needed (connects to unified gateway)
- **"direct" mode**: Show MCP server Select dropdown (current individual server list)

### 5. Update connection logic
- **"client" mode**: Use client's API key (`get_client_value`) as `clientToken`, connect to gateway (the gateway already enforces per-client MCP access)
- **"all" mode**: Use `get_internal_test_token` as today (connects to gateway with full access)
- **"direct" mode**: Use `get_internal_test_token`, connect to specific server by ID

### 6. Deferred loading checkbox
- Show for "all" mode (gateway) - same as today
- Show for "client" mode since it also connects to gateway (client may have `mcp_deferred_loading` preference)
- Hide for "direct" mode

### 7. Rename labels
- Card title: "MCP Connection" → "MCP & Skill Connection"
- Card description: "Test MCP servers through the unified gateway or individually" → "Test MCP servers and skills through a client, the unified gateway, or individually"

## Verification
1. Run `cargo tauri dev` and navigate to Try It Out → MCP & Skill tab
2. Verify radio buttons render with correct labels and icons
3. Test "Against Client" mode: select a client, verify connect works with client's token
4. Test "All MCPs & Skills" mode: verify connects to gateway with internal token
5. Test "Direct MCP/Skill" mode: verify individual server selection and connection
6. Verify deferred loading checkbox visibility per mode
7. Verify disabled state while connected/connecting
