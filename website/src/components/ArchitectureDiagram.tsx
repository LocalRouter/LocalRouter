import ReactFlow, {
  Node,
  Edge,
  Background,
  useNodesState,
  useEdgesState,
  Position,
  MarkerType,
} from 'reactflow'
import 'reactflow/dist/style.css'

const initialNodes: Node[] = [
  // Local Apps (left column) - 6 apps total
  {
    id: 'app-1',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
          <div style={{
            width: '32px',
            height: '32px',
            background: 'linear-gradient(135deg, #6366f1 0%, #4f46e5 100%)',
            borderRadius: '6px',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: 'white',
            fontWeight: 'bold',
            fontSize: '16px',
            flexShrink: 0
          }}>
            {'<>'}
          </div>
          <div style={{ lineHeight: '1.3' }}>
            <div style={{ fontWeight: '600', fontSize: '13px' }}>OpenCode</div>
          </div>
        </div>
      )
    },
    position: { x: 0, y: 0 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #ffffff 0%, #f8fafc 100%)',
      border: '2px solid #e2e8f0',
      borderRadius: '10px',
      padding: '12px 14px',
      fontSize: '14px',
      fontWeight: '500',
      boxShadow: '0 2px 8px rgba(0, 0, 0, 0.06)',
      zIndex: 10,
      minWidth: '200px',
    },
  },
  {
    id: 'app-2',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
          <img
            src="/icons/cursor.svg"
            alt="Cursor"
            style={{
              width: '32px',
              height: '32px',
              objectFit: 'contain',
              flexShrink: 0
            }}
          />
          <div style={{ lineHeight: '1.3' }}>
            <div style={{ fontWeight: '600', fontSize: '13px' }}>Cursor</div>
          </div>
        </div>
      )
    },
    position: { x: 0, y: 70 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #ffffff 0%, #f8fafc 100%)',
      border: '2px solid #e2e8f0',
      borderRadius: '10px',
      padding: '12px 14px',
      fontSize: '14px',
      fontWeight: '500',
      boxShadow: '0 2px 8px rgba(0, 0, 0, 0.06)',
      zIndex: 10,
      minWidth: '200px',
    },
  },
  {
    id: 'app-3',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
          <img
            src="/icons/open-webui.png"
            alt="Open WebUI"
            style={{
              width: '32px',
              height: '32px',
              objectFit: 'contain',
              flexShrink: 0
            }}
          />
          <div style={{ lineHeight: '1.3' }}>
            <div style={{ fontWeight: '600', fontSize: '13px' }}>Open WebUI</div>
          </div>
        </div>
      )
    },
    position: { x: 0, y: 140 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #ffffff 0%, #f8fafc 100%)',
      border: '2px solid #e2e8f0',
      borderRadius: '10px',
      padding: '12px 14px',
      fontSize: '14px',
      fontWeight: '500',
      boxShadow: '0 2px 8px rgba(0, 0, 0, 0.06)',
      zIndex: 10,
      minWidth: '200px',
    },
  },
  {
    id: 'app-4',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
          <img
            src="/icons/obsidian.svg"
            alt="Obsidian"
            style={{
              width: '32px',
              height: '32px',
              objectFit: 'contain',
              flexShrink: 0
            }}
          />
          <div style={{ lineHeight: '1.3' }}>
            <div style={{ fontWeight: '600', fontSize: '13px' }}>Obsidian</div>
            <div style={{ fontSize: '11px', color: '#64748b' }}>(Copilot plugin)</div>
          </div>
        </div>
      )
    },
    position: { x: 0, y: 210 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #ffffff 0%, #f8fafc 100%)',
      border: '2px solid #e2e8f0',
      borderRadius: '10px',
      padding: '12px 14px',
      fontSize: '14px',
      fontWeight: '500',
      boxShadow: '0 2px 8px rgba(0, 0, 0, 0.06)',
      zIndex: 10,
      minWidth: '200px',
    },
  },
  {
    id: 'app-5',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
          <img
            src="/icons/thunderbird.svg"
            alt="Thunderbird"
            style={{
              width: '32px',
              height: '32px',
              objectFit: 'contain',
              flexShrink: 0
            }}
          />
          <div style={{ lineHeight: '1.3' }}>
            <div style={{ fontWeight: '600', fontSize: '13px' }}>Thunderbird</div>
            <div style={{ fontSize: '11px', color: '#64748b' }}>(ThunderAI Add-on)</div>
          </div>
        </div>
      )
    },
    position: { x: 0, y: 280 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #ffffff 0%, #f8fafc 100%)',
      border: '2px solid #e2e8f0',
      borderRadius: '10px',
      padding: '12px 14px',
      fontSize: '14px',
      fontWeight: '500',
      boxShadow: '0 2px 8px rgba(0, 0, 0, 0.06)',
      zIndex: 10,
      minWidth: '200px',
    },
  },
  {
    id: 'app-6',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
          <img
            src="/icons/libreoffice.svg"
            alt="LibreOffice"
            style={{
              width: '32px',
              height: '32px',
              objectFit: 'contain',
              flexShrink: 0
            }}
          />
          <div style={{ lineHeight: '1.3' }}>
            <div style={{ fontWeight: '600', fontSize: '13px' }}>LibreOffice</div>
            <div style={{ fontSize: '11px', color: '#64748b' }}>(AI extension)</div>
          </div>
        </div>
      )
    },
    position: { x: 0, y: 350 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #ffffff 0%, #f8fafc 100%)',
      border: '2px solid #e2e8f0',
      borderRadius: '10px',
      padding: '12px 14px',
      fontSize: '14px',
      fontWeight: '500',
      boxShadow: '0 2px 8px rgba(0, 0, 0, 0.06)',
      zIndex: 10,
      minWidth: '200px',
    },
  },

  // LocalRouter Box (center, parent for API keys)
  {
    id: 'localrouter',
    type: 'group',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <span style={{ fontSize: '28px' }}>🔀</span>
          <span>LocalRouter</span>
        </div>
      )
    },
    position: { x: 280, y: 0 },
    style: {
      background: 'linear-gradient(145deg, #4f46e5 0%, #7c3aed 50%, #a855f7 100%)',
      border: '2px solid rgba(255, 255, 255, 0.3)',
      borderRadius: '16px',
      padding: '20px',
      width: 200,
      height: 270,
      color: 'white',
      fontWeight: '700',
      fontSize: '16px',
      boxShadow: '0 20px 40px rgba(79, 70, 229, 0.4), inset 0 1px 0 rgba(255, 255, 255, 0.2)',
      zIndex: 1,
    },
  },

  // API Keys Section Label
  {
    id: 'apikeys-label',
    type: 'default',
    data: {
      label: (
        <div style={{
          textAlign: 'center',
          fontSize: '11px',
          fontWeight: '600',
          color: 'rgba(255, 255, 255, 0.8)',
          textTransform: 'uppercase',
          letterSpacing: '1px'
        }}>
          API Keys
        </div>
      )
    },
    position: { x: 50, y: 55 },
    parentNode: 'localrouter',
    extent: 'parent' as const,
    style: {
      background: 'transparent',
      border: 'none',
      pointerEvents: 'none',
      zIndex: 5,
    },
    draggable: false,
    selectable: false,
  },

  // API Keys (inside LocalRouter)
  {
    id: 'apikey-coding',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
          <span style={{ fontSize: '16px' }}>🔑</span>
          <span>Coding</span>
        </div>
      )
    },
    position: { x: 20, y: 80 },
    parentNode: 'localrouter',
    extent: 'parent' as const,
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'rgba(255, 255, 255, 0.95)',
      border: '2px solid #c084fc',
      borderRadius: '8px',
      padding: '6px 10px',
      fontSize: '12px',
      fontWeight: '600',
      minWidth: '120px',
      boxShadow: '0 4px 12px rgba(0, 0, 0, 0.15)',
      zIndex: 20,
    },
  },
  {
    id: 'apikey-online',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
          <span style={{ fontSize: '16px' }}>🔑</span>
          <span>Online-first</span>
        </div>
      )
    },
    position: { x: 20, y: 130 },
    parentNode: 'localrouter',
    extent: 'parent' as const,
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'rgba(255, 255, 255, 0.95)',
      border: '2px solid #c084fc',
      borderRadius: '8px',
      padding: '6px 10px',
      fontSize: '12px',
      fontWeight: '600',
      minWidth: '120px',
      boxShadow: '0 4px 12px rgba(0, 0, 0, 0.15)',
      zIndex: 20,
    },
  },
  {
    id: 'apikey-privacy',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '6px' }}>
          <span style={{ fontSize: '16px' }}>🔑</span>
          <span>Privacy-first</span>
        </div>
      )
    },
    position: { x: 20, y: 180 },
    parentNode: 'localrouter',
    extent: 'parent' as const,
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'rgba(255, 255, 255, 0.95)',
      border: '2px solid #c084fc',
      borderRadius: '8px',
      padding: '6px 10px',
      fontSize: '12px',
      fontWeight: '600',
      minWidth: '120px',
      boxShadow: '0 4px 12px rgba(0, 0, 0, 0.15)',
      zIndex: 20,
    },
  },


  // Providers (right column) - All standalone, no grouping
  {
    id: 'provider-claude',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <img
            src="/icons/anthropic.svg"
            alt="Claude"
            style={{
              width: '24px',
              height: '24px',
              objectFit: 'contain'
            }}
          />
          <span>Claude</span>
        </div>
      )
    },
    position: { x: 600, y: 0 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #fef3e2 0%, #fde8c7 100%)',
      border: '2px solid #CA5A2C',
      borderRadius: '10px',
      padding: '12px 16px',
      fontSize: '14px',
      fontWeight: '600',
      boxShadow: '0 4px 12px rgba(202, 90, 44, 0.2)',
      zIndex: 10,
    },
  },
  {
    id: 'provider-openai',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <span style={{ fontSize: '24px' }}>🤖</span>
          <span>OpenAI</span>
        </div>
      )
    },
    position: { x: 600, y: 80 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #dbeafe 0%, #bfdbfe 100%)',
      border: '2px solid #3b82f6',
      borderRadius: '10px',
      padding: '12px 16px',
      fontSize: '14px',
      fontWeight: '600',
      boxShadow: '0 4px 12px rgba(59, 130, 246, 0.25)',
      zIndex: 10,
    },
  },
  {
    id: 'provider-openrouter',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <img
            src="/icons/openrouter.png"
            alt="OpenRouter"
            style={{
              width: '24px',
              height: '24px',
              objectFit: 'contain'
            }}
          />
          <span>OpenRouter</span>
        </div>
      )
    },
    position: { x: 600, y: 300 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #dbeafe 0%, #bfdbfe 100%)',
      border: '2px solid #3b82f6',
      borderRadius: '10px',
      padding: '12px 16px',
      fontSize: '14px',
      fontWeight: '600',
      boxShadow: '0 4px 12px rgba(59, 130, 246, 0.25)',
      zIndex: 10,
    },
  },
  {
    id: 'provider-lmstudio',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <img
            src="/icons/lmstudio.ico"
            alt="LM Studio"
            style={{
              width: '24px',
              height: '24px',
              objectFit: 'contain'
            }}
          />
          <span>LM Studio</span>
        </div>
      )
    },
    position: { x: 600, y: 380 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #e0f2fe 0%, #bae6fd 100%)',
      border: '2px solid #0ea5e9',
      borderRadius: '10px',
      padding: '12px 16px',
      fontSize: '14px',
      fontWeight: '600',
      boxShadow: '0 4px 12px rgba(14, 165, 233, 0.2)',
      zIndex: 10,
    },
  },
  {
    id: 'provider-ollama',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <img
            src="/icons/ollama.svg"
            alt="Ollama"
            style={{
              width: '24px',
              height: '24px',
              objectFit: 'contain'
            }}
          />
          <span>Ollama</span>
        </div>
      )
    },
    position: { x: 600, y: 460 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #e0f2fe 0%, #bae6fd 100%)',
      border: '2px solid #0ea5e9',
      borderRadius: '10px',
      padding: '12px 16px',
      fontSize: '14px',
      fontWeight: '600',
      boxShadow: '0 4px 12px rgba(14, 165, 233, 0.2)',
      zIndex: 10,
    },
  },

  // MCP Servers (right column, between OpenAI and OpenRouter, slightly offset to the right)
  {
    id: 'service-github',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <img
            src="/icons/github.svg"
            alt="GitHub MCP"
            style={{
              width: '24px',
              height: '24px',
              objectFit: 'contain'
            }}
          />
          <span>Github MCP</span>
        </div>
      )
    },
    position: { x: 630, y: 160 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #f3f4f6 0%, #e5e7eb 100%)',
      border: '2px solid #1f2937',
      borderRadius: '10px',
      padding: '12px 16px',
      fontSize: '14px',
      fontWeight: '600',
      boxShadow: '0 4px 12px rgba(31, 41, 55, 0.15)',
      zIndex: 10,
    },
  },
  {
    id: 'service-jira',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <img
            src="/icons/jira.svg"
            alt="Jira MCP"
            style={{
              width: '24px',
              height: '24px',
              objectFit: 'contain'
            }}
          />
          <span>Jira MCP</span>
        </div>
      )
    },
    position: { x: 630, y: 230 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #deebff 0%, #b3d4ff 100%)',
      border: '2px solid #0052CC',
      borderRadius: '10px',
      padding: '12px 16px',
      fontSize: '14px',
      fontWeight: '600',
      boxShadow: '0 4px 12px rgba(0, 82, 204, 0.2)',
      zIndex: 10,
    },
  },
]

const initialEdges: Edge[] = [
  // Apps to API Keys
  // OpenCode and Cursor → Coding
  {
    id: 'e-app-1-coding',
    source: 'app-1',
    target: 'apikey-coding',
    animated: true,
    style: { stroke: '#64748b', strokeWidth: 3 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#64748b', width: 10, height: 10 },
    zIndex: 100,
  },
  {
    id: 'e-app-2-coding',
    source: 'app-2',
    target: 'apikey-coding',
    animated: true,
    style: { stroke: '#64748b', strokeWidth: 3 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#64748b', width: 10, height: 10 },
    zIndex: 100,
  },

  // Open WebUI → Online-first
  {
    id: 'e-app-3-online',
    source: 'app-3',
    target: 'apikey-online',
    animated: true,
    style: { stroke: '#64748b', strokeWidth: 3 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#64748b', width: 10, height: 10 },
    zIndex: 100,
  },

  // Obsidian, Thunderbird, LibreOffice → Privacy-first
  {
    id: 'e-app-4-privacy',
    source: 'app-4',
    target: 'apikey-privacy',
    animated: true,
    style: { stroke: '#64748b', strokeWidth: 3 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#64748b', width: 10, height: 10 },
    zIndex: 100,
  },
  {
    id: 'e-app-5-privacy',
    source: 'app-5',
    target: 'apikey-privacy',
    animated: true,
    style: { stroke: '#64748b', strokeWidth: 3 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#64748b', width: 10, height: 10 },
    zIndex: 100,
  },
  {
    id: 'e-app-6-privacy',
    source: 'app-6',
    target: 'apikey-privacy',
    animated: true,
    style: { stroke: '#64748b', strokeWidth: 3 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#64748b', width: 10, height: 10 },
    zIndex: 100,
  },

  // API Keys to Providers
  // Coding → Claude AND OpenAI
  {
    id: 'e-coding-claude',
    source: 'apikey-coding',
    target: 'provider-claude',
    animated: true,
    style: { stroke: '#CA5A2C', strokeWidth: 3 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#CA5A2C', width: 10, height: 10 },
    zIndex: 100,
  },
  {
    id: 'e-coding-openai',
    source: 'apikey-coding',
    target: 'provider-openai',
    animated: true,
    style: { stroke: '#3b82f6', strokeWidth: 3 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#3b82f6', width: 10, height: 10 },
    zIndex: 100,
  },

  // Online-first → OpenRouter and LM Studio
  {
    id: 'e-online-openrouter',
    source: 'apikey-online',
    target: 'provider-openrouter',
    animated: true,
    style: { stroke: '#3b82f6', strokeWidth: 3 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#3b82f6', width: 10, height: 10 },
    zIndex: 100,
  },
  {
    id: 'e-online-lmstudio',
    source: 'apikey-online',
    target: 'provider-lmstudio',
    animated: true,
    style: { stroke: '#0ea5e9', strokeWidth: 3 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#0ea5e9', width: 10, height: 10 },
    zIndex: 100,
  },

  // Privacy-first → Ollama
  {
    id: 'e-privacy-ollama',
    source: 'apikey-privacy',
    target: 'provider-ollama',
    animated: true,
    style: { stroke: '#0ea5e9', strokeWidth: 3 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#0ea5e9', width: 10, height: 10 },
    zIndex: 100,
  },

  // Coding API Key to MCP Servers
  {
    id: 'e-coding-github-mcp',
    source: 'apikey-coding',
    target: 'service-github',
    animated: true,
    style: { stroke: '#1f2937', strokeWidth: 3 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#1f2937', width: 10, height: 10 },
    zIndex: 100,
  },
  {
    id: 'e-coding-jira-mcp',
    source: 'apikey-coding',
    target: 'service-jira',
    animated: true,
    style: { stroke: '#0052CC', strokeWidth: 3 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#0052CC', width: 10, height: 10 },
    zIndex: 100,
  },
]

export default function ArchitectureDiagram() {
  const [nodes, , onNodesChange] = useNodesState(initialNodes)
  const [edges, , onEdgesChange] = useEdgesState(initialEdges)

  return (
    <div className="w-full" style={{ height: '600px' }}>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        fitView
        fitViewOptions={{ padding: 0.2 }}
        attributionPosition="bottom-left"
        proOptions={{ hideAttribution: true }}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable={false}
        panOnDrag={false}
        zoomOnScroll={false}
        zoomOnPinch={false}
        preventScrolling={true}
        elevateEdgesOnSelect={false}
      >
        <Background color="#e5e7eb" gap={20} size={1} />
      </ReactFlow>
    </div>
  )
}
