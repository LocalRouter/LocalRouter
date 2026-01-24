import { useEffect, useRef } from 'react'
import cytoscape from 'cytoscape'

export default function ArchitectureGraph() {
  const containerRef = useRef<HTMLDivElement>(null)
  const cyRef = useRef<cytoscape.Core | null>(null)

  useEffect(() => {
    if (!containerRef.current) return

    const cy = cytoscape({
      container: containerRef.current,
      elements: [
        // Apps (left side)
        { data: { id: 'opencode', label: 'OpenCode', type: 'app' } },
        { data: { id: 'cursor', label: 'Cursor', type: 'app' } },
        { data: { id: 'everythingllm', label: 'EverythingLLM', type: 'app' } },

        // API Keys (center)
        { data: { id: 'coding-key', label: 'Coding', type: 'apikey' } },
        { data: { id: 'conversations-key', label: 'Conversations', type: 'apikey' } },

        // LocalRouter (center hub)
        { data: { id: 'localrouter', label: 'LocalRouter\nlocalhost:3625', type: 'router' } },

        // LLM Providers (right side)
        { data: { id: 'openrouter', label: 'OpenRouter', type: 'llm' } },
        { data: { id: 'chatgpt', label: 'ChatGPT', type: 'llm' } },
        { data: { id: 'ollama', label: 'Ollama', type: 'llm-local' } },

        // MCP Servers (right side)
        { data: { id: 'filesystem', label: 'Filesystem', type: 'mcp' } },
        { data: { id: 'jira', label: 'Jira', type: 'mcp' } },
        { data: { id: 'gmail', label: 'Gmail', type: 'mcp' } },

        // Edges: Apps to API Keys
        { data: { id: 'e1', source: 'opencode', target: 'coding-key', type: 'app-key' } },
        { data: { id: 'e2', source: 'cursor', target: 'coding-key', type: 'app-key' } },
        { data: { id: 'e3', source: 'everythingllm', target: 'conversations-key', type: 'app-key' } },

        // Edges: API Keys to LocalRouter
        { data: { id: 'e4', source: 'coding-key', target: 'localrouter', type: 'key-router' } },
        { data: { id: 'e5', source: 'conversations-key', target: 'localrouter', type: 'key-router' } },

        // Edges: LocalRouter to LLM Providers (Coding key routes)
        { data: { id: 'e6', source: 'localrouter', target: 'chatgpt', keyType: 'coding' } },
        { data: { id: 'e7', source: 'localrouter', target: 'ollama', keyType: 'coding' } },
        { data: { id: 'e8', source: 'localrouter', target: 'openrouter', keyType: 'coding' } },

        // Edges: LocalRouter to LLM Providers (Conversations key routes)
        { data: { id: 'e9', source: 'localrouter', target: 'ollama', keyType: 'conversations' } },
        { data: { id: 'e10', source: 'localrouter', target: 'openrouter', keyType: 'conversations' } },

        // Edges: LocalRouter to MCP Servers (Coding key routes)
        { data: { id: 'e11', source: 'localrouter', target: 'filesystem', keyType: 'coding' } },
        { data: { id: 'e12', source: 'localrouter', target: 'jira', keyType: 'coding' } },

        // Edges: LocalRouter to MCP Servers (Conversations key routes)
        { data: { id: 'e13', source: 'localrouter', target: 'jira', keyType: 'conversations' } },
        { data: { id: 'e14', source: 'localrouter', target: 'gmail', keyType: 'conversations' } },
      ],
      style: [
        // Base node styles
        {
          selector: 'node',
          style: {
            'label': 'data(label)',
            'text-valign': 'center',
            'text-halign': 'center',
            'font-size': '11px',
            'font-weight': 500,
            'text-wrap': 'wrap',
            'text-max-width': '80px',
            'color': '#fff',
            'text-outline-color': '#000',
            'text-outline-width': 1,
            'width': 80,
            'height': 36,
            'shape': 'round-rectangle',
            'border-width': 2,
            'border-color': '#374151',
          },
        },
        // App nodes (blue)
        {
          selector: 'node[type="app"]',
          style: {
            'background-color': '#3b82f6',
            'border-color': '#1d4ed8',
          },
        },
        // API Key nodes (amber)
        {
          selector: 'node[type="apikey"]',
          style: {
            'background-color': '#f59e0b',
            'border-color': '#d97706',
            'width': 90,
            'height': 40,
          },
        },
        // LocalRouter (primary purple/violet)
        {
          selector: 'node[type="router"]',
          style: {
            'background-color': '#8b5cf6',
            'border-color': '#6d28d9',
            'width': 110,
            'height': 50,
            'font-size': '12px',
            'font-weight': 600,
          },
        },
        // LLM Provider nodes (green)
        {
          selector: 'node[type="llm"]',
          style: {
            'background-color': '#10b981',
            'border-color': '#059669',
          },
        },
        // Local LLM (green with different border)
        {
          selector: 'node[type="llm-local"]',
          style: {
            'background-color': '#10b981',
            'border-color': '#fbbf24',
            'border-width': 3,
          },
        },
        // MCP Server nodes (pink/rose)
        {
          selector: 'node[type="mcp"]',
          style: {
            'background-color': '#ec4899',
            'border-color': '#db2777',
          },
        },
        // Base edge styles
        {
          selector: 'edge',
          style: {
            'width': 2,
            'line-color': '#6b7280',
            'target-arrow-color': '#6b7280',
            'target-arrow-shape': 'triangle',
            'curve-style': 'bezier',
            'arrow-scale': 0.8,
          },
        },
        // App to Key edges
        {
          selector: 'edge[type="app-key"]',
          style: {
            'line-color': '#60a5fa',
            'target-arrow-color': '#60a5fa',
          },
        },
        // Key to Router edges
        {
          selector: 'edge[type="key-router"]',
          style: {
            'line-color': '#fbbf24',
            'target-arrow-color': '#fbbf24',
            'width': 3,
          },
        },
        // Coding key routes (blue)
        {
          selector: 'edge[keyType="coding"]',
          style: {
            'line-color': '#3b82f6',
            'target-arrow-color': '#3b82f6',
          },
        },
        // Conversations key routes (amber)
        {
          selector: 'edge[keyType="conversations"]',
          style: {
            'line-color': '#f59e0b',
            'target-arrow-color': '#f59e0b',
          },
        },
      ],
      layout: {
        name: 'preset',
        positions: {
          // Apps (left)
          'opencode': { x: 50, y: 60 },
          'cursor': { x: 50, y: 120 },
          'everythingllm': { x: 50, y: 200 },

          // API Keys (center-left)
          'coding-key': { x: 180, y: 90 },
          'conversations-key': { x: 180, y: 200 },

          // LocalRouter (center)
          'localrouter': { x: 340, y: 145 },

          // LLM Providers (right-top)
          'openrouter': { x: 500, y: 50 },
          'chatgpt': { x: 500, y: 110 },
          'ollama': { x: 500, y: 170 },

          // MCP Servers (right-bottom)
          'filesystem': { x: 500, y: 230 },
          'jira': { x: 580, y: 170 },
          'gmail': { x: 580, y: 230 },
        },
      },
      userZoomingEnabled: false,
      userPanningEnabled: false,
      boxSelectionEnabled: false,
      autoungrabify: true,
    })

    cyRef.current = cy

    // Fit to container with padding
    cy.fit(undefined, 30)

    return () => {
      cy.destroy()
    }
  }, [])

  return (
    <div className="rounded-xl border bg-card shadow-sm overflow-hidden">
      <div
        ref={containerRef}
        className="w-full h-[320px] bg-gradient-to-br from-zinc-900 to-zinc-800"
      />
      {/* Legend */}
      <div className="px-4 py-3 border-t bg-card/50 flex flex-wrap justify-center gap-4 text-xs text-muted-foreground">
        <div className="flex items-center gap-1.5">
          <span className="w-3 h-3 rounded bg-blue-500"></span>
          Apps
        </div>
        <div className="flex items-center gap-1.5">
          <span className="w-3 h-3 rounded bg-amber-500"></span>
          API Keys
        </div>
        <div className="flex items-center gap-1.5">
          <span className="w-3 h-3 rounded bg-violet-500"></span>
          LocalRouter
        </div>
        <div className="flex items-center gap-1.5">
          <span className="w-3 h-3 rounded bg-emerald-500"></span>
          LLM Providers
        </div>
        <div className="flex items-center gap-1.5">
          <span className="w-3 h-3 rounded bg-pink-500"></span>
          MCP Servers
        </div>
        <div className="flex items-center gap-1.5">
          <span className="w-3 h-3 rounded border-2 border-amber-400 bg-emerald-500"></span>
          Local
        </div>
      </div>
    </div>
  )
}
