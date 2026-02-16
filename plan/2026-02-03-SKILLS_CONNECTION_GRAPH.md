# Add Skills to Connection Graph

## Overview
Add skill nodes to the connection graph, following the same pattern as MCP servers. Skills will appear as a new node type with edges from clients based on `skills_access_mode` / `skills_names`.

## Files to Modify

### 1. `src/components/connection-graph/types.ts`
- Add `'skill'` to `GraphNodeType` union
- Add `SkillNodeData` interface (name, enabled)
- Add `SkillNodeData` to `GraphNodeData` union
- Add `SkillNode` typed node export
- Add `Skill` interface for backend data (`{ name, enabled }`)
- Add `skills: Skill[]` to `UseGraphDataResult`
- Add `skills_access_mode` and `skills_names` to `Client` interface

### 2. `src/components/connection-graph/nodes/SkillNode.tsx` (new file)
- Amber/orange themed node (matching skill badges elsewhere in the UI)
- Input handle on left for incoming edges
- Sparkles or Zap icon from lucide-react
- No health status dot (skills don't have health checks)
- Follow `McpServerNode` pattern closely

### 3. `src/components/connection-graph/utils/buildGraph.ts`
- Import `Skill`, `SkillNodeData` types
- Add `skills` parameter to `buildNodes` and `buildEdges`
- Create skill nodes from enabled skills
- Create client-to-skill edges (amber color `#f59e0b`) using same access mode pattern as MCP servers
- Update `buildGraph` export to accept `skills` param

### 4. `src/components/connection-graph/hooks/useGraphData.ts`
- Import `Skill` type
- Add `skills` state, fetch via `invoke<Skill[]>('list_skills')`
- Listen for `'skills-changed'` event to refetch
- Return `skills` in result

### 5. `src/components/connection-graph/ConnectionGraph.tsx`
- Import and register `SkillNode` in `nodeTypes`
- Pass `skills` to `buildGraph`
- Add `skill` case to `handleNodeClick` -> navigate to `'skills'` view with skill name
- Include skills in `isEmpty` check

## Design Decisions
- **Color**: Amber (`from-amber-50 to-amber-100`) - matches existing skill badge colors in the UI
- **Icon**: Lucide `Sparkles` icon - distinctive and skill-appropriate
- **Edge color**: `#f59e0b` (amber-500) when active, `#64748b` when inactive
- **No health dot**: Skills don't have health monitoring like providers/MCP servers

## Verification
- Run `cargo tauri dev` and check the dashboard connection graph
- Ensure skills appear when enabled and connected to clients
- Click a skill node and verify navigation to skills view
- Verify real-time updates when skills are added/removed/toggled
