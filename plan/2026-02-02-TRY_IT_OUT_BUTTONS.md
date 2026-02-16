# Plan: Add "Try It Out" Buttons to Entity Pages

## Overview
Add a `FlaskConical` icon button to Skills, MCP Servers, LLM Providers, and Clients pages that navigates directly to Try It Out with the correct mode and target pre-selected.

## Navigation Encoding

Use the existing subTab path to encode initial state:
- **Providers**: `llm/init/direct/<providerInstanceName>`
- **MCP Servers**: `mcp/init/direct/server:<serverId>`
- **Skills**: `mcp/init/direct/skill:<skillName>`
- **Clients (LLM)**: `llm/init/client/<clientId>`
- **Clients (MCP)**: `mcp/init/client/<clientId>`

## Files to Modify

### 1. `src/views/try-it-out/index.tsx`
- Parse `init/...` from the innerPath and extract initial params
- Pass `initialMode` and `initialTarget` props to `LlmTab` and `McpTab`
- Clear the init params from the URL after passing them (call `onTabChange` with the clean tab name)

### 2. `src/views/try-it-out/llm-tab/index.tsx`
- Add optional props: `initialMode?: TestMode`, `initialProvider?: string`, `initialClientId?: string`
- On mount, if initial props are set, override default mode/selection

### 3. `src/views/try-it-out/mcp-tab/index.tsx`
- Add optional props: `initialMode?: McpTestMode`, `initialDirectTarget?: string`, `initialClientId?: string`
- On mount, if initial props are set, override default mode/selection

### 4. `src/views/resources/providers-panel.tsx`
- Add `onViewChange` prop
- Add FlaskConical button in the detail header (next to EntityActions)
- On click: `onViewChange('try-it-out', 'llm/init/direct/<instanceName>')`

### 5. `src/views/resources/index.tsx`
- Pass `onTabChange` through to `ProvidersPanel` as `onViewChange`

### 6. `src/views/resources/mcp-servers-panel.tsx`
- Add `onViewChange` prop
- Add FlaskConical button in the detail header
- On click: `onViewChange('try-it-out', 'mcp/init/direct/server:<id>')`

### 7. `src/views/mcp-servers/index.tsx`
- Pass `onTabChange` through to `McpServersPanel` as `onViewChange`

### 8. `src/views/skills/index.tsx`
- Add FlaskConical button in skill detail header (next to the enabled/disabled switch)
- On click: `onTabChange('try-it-out', 'mcp/init/direct/skill:<name>')`

### 9. `src/views/clients/client-detail.tsx`
- Add FlaskConical button in the client header (next to the dropdown menu)
- On click: `onViewChange('try-it-out', 'llm/init/client/<clientId>')`
- Only show when client is enabled

## Button Style
- Use `Button` with `variant="outline"` and `size="sm"`
- Icon: `FlaskConical` from lucide-react
- Text: "Try It Out"
- Tooltip or just the text label

## Verification
- Navigate to each page, select an entity, verify the button appears
- Click the button, verify Try It Out opens with the correct tab, mode, and target pre-selected
- Run `cargo tauri dev` to test in the app
