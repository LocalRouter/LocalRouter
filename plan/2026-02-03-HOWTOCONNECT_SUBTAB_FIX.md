# Fix: Persist Config/JSON sub-tab selection across auth methods

## Problem
In `HowToConnect`, the MCP tab has three auth methods (API Key, OAuth, STDIO), each with Config/JSON sub-tabs. Each sub-tab group is an independent `<Tabs defaultValue="config">`, so selecting "JSON" under one auth method resets to "Config" when switching to another.

## Fix
File: `src/components/client/HowToConnect.tsx`

1. Add a state variable to track the sub-tab selection:
   ```ts
   const [mcpSubTab, setMcpSubTab] = useState<string>("config")
   ```

2. Replace all three `<Tabs defaultValue="config">` (lines 379, 443, 529) with controlled tabs:
   ```tsx
   <Tabs value={mcpSubTab} onValueChange={setMcpSubTab}>
   ```

This ensures switching between API Key/OAuth/STDIO preserves the Config vs JSON selection.

## Verification
- Open a client's "How to Connect" card
- Go to MCP tab, select "JSON" sub-tab under API Key
- Switch to OAuth or STDIO — JSON should remain selected
- Switch back — JSON should still be selected
- Select "Config" — switching auth methods should preserve "Config"
