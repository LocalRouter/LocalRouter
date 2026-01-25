import ReactFlow, {
  Node,
  Edge,
  Position,
  MarkerType,
} from 'reactflow'
import 'reactflow/dist/style.css'

// Hide connection handles
const hideHandlesStyle = `
  .react-flow__handle {
    opacity: 0;
    pointer-events: none;
  }
`

// Shared styles
const appNodeStyle = {
  background: 'linear-gradient(145deg, #ffffff 0%, #f8fafc 100%)',
  border: '2px solid #e2e8f0',
  borderRadius: '12px',
  padding: '10px 16px',
  fontSize: '14px',
  fontWeight: '500',
  boxShadow: '0 4px 12px rgba(0, 0, 0, 0.08)',
  minWidth: '160px',
}

const providerNodeStyle = {
  borderRadius: '12px',
  padding: '10px 16px',
  fontSize: '14px',
  fontWeight: '600',
  boxShadow: '0 4px 16px rgba(0, 0, 0, 0.1)',
  minWidth: '140px',
}

const mcpNodeStyle = {
  borderRadius: '12px',
  padding: '10px 16px',
  fontSize: '14px',
  fontWeight: '600',
  boxShadow: '0 4px 16px rgba(0, 0, 0, 0.1)',
  minWidth: '130px',
}

// Icon component for consistent sizing
const IconBox = ({ gradient, children }: { gradient: string; children: React.ReactNode }) => (
  <div style={{
    width: '28px',
    height: '28px',
    background: gradient,
    borderRadius: '6px',
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    color: 'white',
    fontWeight: 'bold',
    fontSize: '12px',
    flexShrink: 0,
  }}>
    {children}
  </div>
)

const ImgIcon = ({ src, alt }: { src: string; alt: string }) => (
  <img
    src={src}
    alt={alt}
    style={{
      width: '24px',
      height: '24px',
      objectFit: 'contain',
      flexShrink: 0,
    }}
  />
)

const NodeLabel = ({ icon, name, subtitle }: { icon: React.ReactNode; name: string; subtitle?: string }) => (
  <div style={{ display: 'flex', alignItems: 'center', gap: '10px' }}>
    {icon}
    <div style={{ lineHeight: '1.2' }}>
      <div style={{ fontWeight: '600', fontSize: '13px', color: '#1e293b' }}>{name}</div>
      {subtitle && <div style={{ fontSize: '10px', color: '#64748b' }}>{subtitle}</div>}
    </div>
  </div>
)

const initialNodes: Node[] = [
  // Column Labels
  {
    id: 'label-apps',
    type: 'default',
    data: { label: <span style={{ fontSize: '11px', fontWeight: '600', color: '#94a3b8', textTransform: 'uppercase', letterSpacing: '1px' }}>AI Clients</span> },
    position: { x: 45, y: 0 },
    style: { background: 'transparent', border: 'none', boxShadow: 'none' },
    draggable: false,
    selectable: false,
  },
  {
    id: 'label-providers',
    type: 'default',
    data: { label: <span style={{ fontSize: '11px', fontWeight: '600', color: '#94a3b8', textTransform: 'uppercase', letterSpacing: '1px' }}>LLM Providers</span> },
    position: { x: 510, y: 0 },
    style: { background: 'transparent', border: 'none', boxShadow: 'none' },
    draggable: false,
    selectable: false,
  },
  {
    id: 'label-mcp',
    type: 'default',
    data: { label: <span style={{ fontSize: '11px', fontWeight: '600', color: '#94a3b8', textTransform: 'uppercase', letterSpacing: '1px' }}>MCP Servers</span> },
    position: { x: 500, y: 195 },
    style: { background: 'transparent', border: 'none', boxShadow: 'none' },
    draggable: false,
    selectable: false,
  },

  // Apps (left column)
  {
    id: 'app-opencode',
    type: 'default',
    data: {
      label: <NodeLabel icon={<IconBox gradient="linear-gradient(135deg, #6366f1 0%, #4f46e5 100%)">{'</>'}</IconBox>} name="OpenCode" subtitle="AI coding assistant" />
    },
    position: { x: 0, y: 40 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: appNodeStyle,
  },
  {
    id: 'app-cursor',
    type: 'default',
    data: {
      label: <NodeLabel icon={<ImgIcon src="/icons/cursor.svg" alt="Cursor" />} name="Cursor" subtitle="IDE with AI" />
    },
    position: { x: 0, y: 115 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: appNodeStyle,
  },
  {
    id: 'app-openwebui',
    type: 'default',
    data: {
      label: <NodeLabel icon={<ImgIcon src="/icons/open-webui.png" alt="Open WebUI" />} name="Open WebUI" subtitle="Chat interface" />
    },
    position: { x: 0, y: 190 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: appNodeStyle,
  },
  {
    id: 'app-everythingllm',
    type: 'default',
    data: {
      label: <NodeLabel icon={<ImgIcon src="/icons/everythingllm.svg" alt="EverythingLLM" />} name="EverythingLLM" subtitle="RAG platform" />
    },
    position: { x: 0, y: 265 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: appNodeStyle,
  },

  // LocalRouter Box (center)
  {
    id: 'localrouter',
    type: 'group',
    data: { label: '' },
    position: { x: 240, y: 55 },
    style: {
      background: 'linear-gradient(160deg, #4f46e5 0%, #7c3aed 50%, #9333ea 100%)',
      border: '2px solid rgba(255, 255, 255, 0.2)',
      borderRadius: '20px',
      padding: '16px',
      width: 190,
      height: 245,
      boxShadow: '0 25px 50px -12px rgba(79, 70, 229, 0.5), inset 0 1px 0 rgba(255, 255, 255, 0.15)',
    },
  },

  // LocalRouter Title
  {
    id: 'localrouter-title',
    type: 'default',
    data: {
      label: (
        <div style={{ textAlign: 'center' }}>
          <div style={{ fontSize: '15px', fontWeight: '700', color: 'white' }}>LocalRouter</div>
        </div>
      )
    },
    position: { x: 30, y: 12 },
    parentNode: 'localrouter',
    extent: 'parent' as const,
    style: { background: 'transparent', border: 'none', boxShadow: 'none', width: 130 },
    draggable: false,
    selectable: false,
  },

  // API Keys
  {
    id: 'apikey-coding',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <span style={{ fontSize: '14px' }}>🔑</span>
          <span style={{ fontWeight: '600', fontSize: '12px', color: '#1e40af' }}>Coding</span>
        </div>
      )
    },
    position: { x: 20, y: 90 },
    parentNode: 'localrouter',
    extent: 'parent' as const,
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #ffffff 0%, #eff6ff 100%)',
      border: '2px solid #3b82f6',
      borderRadius: '10px',
      padding: '8px 12px',
      boxShadow: '0 4px 12px rgba(59, 130, 246, 0.3)',
      minWidth: '115px',
    },
  },
  {
    id: 'apikey-conversations',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <span style={{ fontSize: '14px' }}>🔑</span>
          <span style={{ fontWeight: '600', fontSize: '12px', color: '#b45309' }}>Conversations</span>
        </div>
      )
    },
    position: { x: 20, y: 155 },
    parentNode: 'localrouter',
    extent: 'parent' as const,
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      background: 'linear-gradient(145deg, #ffffff 0%, #fffbeb 100%)',
      border: '2px solid #f59e0b',
      borderRadius: '10px',
      padding: '8px 12px',
      boxShadow: '0 4px 12px rgba(245, 158, 11, 0.3)',
      minWidth: '115px',
    },
  },

  // LLM Providers
  {
    id: 'provider-openrouter',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <ImgIcon src="/icons/openrouter.svg" alt="OpenRouter" />
          <span style={{ color: '#1e40af' }}>OpenRouter</span>
        </div>
      )
    },
    position: { x: 500, y: 30 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      ...providerNodeStyle,
      background: 'linear-gradient(145deg, #eff6ff 0%, #dbeafe 100%)',
      border: '2px solid #3b82f6',
    },
  },
  {
    id: 'provider-chatgpt',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <ImgIcon src="/icons/chatgpt.svg" alt="ChatGPT" />
          <span style={{ color: '#065f46' }}>ChatGPT</span>
        </div>
      )
    },
    position: { x: 500, y: 100 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      ...providerNodeStyle,
      background: 'linear-gradient(145deg, #ecfdf5 0%, #d1fae5 100%)',
      border: '2px solid #10b981',
    },
  },
  {
    id: 'provider-ollama',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <ImgIcon src="/icons/ollama.svg" alt="Ollama" />
          <span style={{ color: '#0369a1' }}>Ollama</span>
        </div>
      )
    },
    position: { x: 500, y: 170 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      ...providerNodeStyle,
      background: 'linear-gradient(145deg, #f0f9ff 0%, #e0f2fe 100%)',
      border: '2px solid #0ea5e9',
    },
  },

  // MCP Servers
  {
    id: 'mcp-filesystem',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <ImgIcon src="/icons/filesystem.svg" alt="Filesystem" />
          <span style={{ color: '#92400e' }}>Filesystem</span>
        </div>
      )
    },
    position: { x: 500, y: 230 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      ...mcpNodeStyle,
      background: 'linear-gradient(145deg, #fffbeb 0%, #fef3c7 100%)',
      border: '2px solid #f59e0b',
    },
  },
  {
    id: 'mcp-jira',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <ImgIcon src="/icons/jira.svg" alt="Jira" />
          <span style={{ color: '#1e40af' }}>Jira</span>
        </div>
      )
    },
    position: { x: 500, y: 295 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      ...mcpNodeStyle,
      background: 'linear-gradient(145deg, #eff6ff 0%, #dbeafe 100%)',
      border: '2px solid #2563eb',
    },
  },
  {
    id: 'mcp-gmail',
    type: 'default',
    data: {
      label: (
        <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
          <ImgIcon src="/icons/gmail.svg" alt="Gmail" />
          <span style={{ color: '#b91c1c' }}>Gmail</span>
        </div>
      )
    },
    position: { x: 500, y: 360 },
    sourcePosition: Position.Right,
    targetPosition: Position.Left,
    style: {
      ...mcpNodeStyle,
      background: 'linear-gradient(145deg, #fef2f2 0%, #fee2e2 100%)',
      border: '2px solid #ef4444',
    },
  },
]

// Edge styling helper
const createEdge = (id: string, source: string, target: string, color: string, animated = true): Edge => ({
  id,
  source,
  target,
  type: 'default',
  animated,
  style: { stroke: color, strokeWidth: 2.5, opacity: 0.85 },
  markerEnd: { type: MarkerType.ArrowClosed, color, width: 16, height: 16 },
})

const blueColor = '#3b82f6'
const amberColor = '#f59e0b'

const initialEdges: Edge[] = [
  // Apps → API Keys
  createEdge('e-opencode-coding', 'app-opencode', 'apikey-coding', blueColor),
  createEdge('e-cursor-coding', 'app-cursor', 'apikey-coding', blueColor),
  createEdge('e-openwebui-conversations', 'app-openwebui', 'apikey-conversations', amberColor),
  createEdge('e-everythingllm-conversations', 'app-everythingllm', 'apikey-conversations', amberColor),

  // Coding → Providers
  createEdge('e-coding-openrouter', 'apikey-coding', 'provider-openrouter', blueColor),
  createEdge('e-coding-chatgpt', 'apikey-coding', 'provider-chatgpt', blueColor),
  createEdge('e-coding-ollama', 'apikey-coding', 'provider-ollama', blueColor),

  // Coding → MCP
  createEdge('e-coding-filesystem', 'apikey-coding', 'mcp-filesystem', blueColor),
  createEdge('e-coding-jira', 'apikey-coding', 'mcp-jira', blueColor),

  // Conversations → Providers
  createEdge('e-conversations-ollama', 'apikey-conversations', 'provider-ollama', amberColor),

  // Conversations → MCP
  createEdge('e-conversations-jira', 'apikey-conversations', 'mcp-jira', amberColor),
  createEdge('e-conversations-gmail', 'apikey-conversations', 'mcp-gmail', amberColor),
]

export default function ArchitectureDiagram() {
  return (
    <div className="w-full" style={{ height: '420px' }}>
      <style>{hideHandlesStyle}</style>
      <ReactFlow
        nodes={initialNodes}
        edges={initialEdges}
        fitView
        fitViewOptions={{ padding: 0.12 }}
        proOptions={{ hideAttribution: true }}
        nodesDraggable={false}
        nodesConnectable={false}
        elementsSelectable={false}
        panOnDrag={false}
        zoomOnScroll={false}
        zoomOnPinch={false}
        zoomOnDoubleClick={false}
        preventScrolling={true}
        minZoom={1}
        maxZoom={1}
      />
    </div>
  )
}
