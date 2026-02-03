import { useMemo, useEffect, useCallback } from 'react'
import ReactFlow, {
  Background,
  Controls,
  useNodesState,
  useEdgesState,
  type NodeTypes,
  type Node,
  type Edge,
  type NodeMouseHandler,
} from 'reactflow'
import 'reactflow/dist/style.css'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/Card'
import { Skeleton } from '@/components/ui/skeleton'
import { AlertCircle, Network } from 'lucide-react'
import { useGraphData } from './hooks/useGraphData'
import { buildGraph } from './utils/buildGraph'
import { AccessKeyNode } from './nodes/AccessKeyNode'
import { ProviderNode } from './nodes/ProviderNode'
import { McpServerNode } from './nodes/McpServerNode'
import { SkillNode } from './nodes/SkillNode'
import { MarketplaceNode } from './nodes/MarketplaceNode'
import type { GraphNodeData } from './types'

// Register custom node types
const nodeTypes: NodeTypes = {
  accessKey: AccessKeyNode,
  provider: ProviderNode,
  mcpServer: McpServerNode,
  skill: SkillNode,
  marketplace: MarketplaceNode,
}

// Props for the ConnectionGraph component
interface ConnectionGraphProps {
  className?: string
  onViewChange?: (view: string, subTab?: string | null) => void
}

export function ConnectionGraph({ className, onViewChange }: ConnectionGraphProps) {
  const { clients, providers, mcpServers, skills, healthState, activeConnections, loading, error } = useGraphData()

  // Build the graph from data
  const { graphNodes, graphEdges, graphBounds } = useMemo(() => {
    const { nodes, edges, bounds } = buildGraph(
      clients,
      providers,
      mcpServers,
      skills,
      healthState,
      activeConnections
    )
    return { graphNodes: nodes as Node<GraphNodeData>[], graphEdges: edges as Edge[], graphBounds: bounds }
  }, [clients, providers, mcpServers, skills, healthState, activeConnections])

  // React Flow state
  const [nodes, setNodes, onNodesChange] = useNodesState<GraphNodeData>([])
  const [edges, setEdges, onEdgesChange] = useEdgesState([])

  // Update nodes and edges when data changes
  useEffect(() => {
    setNodes(graphNodes)
    setEdges(graphEdges)
  }, [graphNodes, graphEdges, setNodes, setEdges])

  // Calculate if graph is empty
  const isEmpty = clients.filter(c => c.enabled).length === 0 &&
    providers.filter(p => p.enabled).length === 0 &&
    mcpServers.filter(s => s.enabled).length === 0 &&
    skills.length === 0

  // Count connected apps
  const connectedCount = activeConnections.length

  // Handle node click for navigation
  const handleNodeClick: NodeMouseHandler = useCallback((_event, node) => {
    if (!onViewChange) return

    const { type, id } = node.data as GraphNodeData

    switch (type) {
      case 'accessKey':
        onViewChange('clients', `${id}|config`)
        break
      case 'provider':
        onViewChange('resources', `providers/${id}`)
        break
      case 'mcpServer':
        onViewChange('mcp-servers', id)
        break
      case 'skill':
        onViewChange('skills', id)
        break
      case 'marketplace':
        onViewChange('marketplace')
        break
    }
  }, [onViewChange])

  // Loading state
  if (loading) {
    return (
      <Card className={className}>
        <CardHeader className="pb-2">
          <CardTitle className="text-lg flex items-center gap-2">
            <Network className="w-5 h-5" />
            Connection Graph
          </CardTitle>
        </CardHeader>
        <CardContent>
          <Skeleton className="w-full h-[150px] rounded-lg" />
        </CardContent>
      </Card>
    )
  }

  // Error state
  if (error) {
    return (
      <Card className={className}>
        <CardHeader className="pb-2">
          <CardTitle className="text-lg flex items-center gap-2">
            <Network className="w-5 h-5" />
            Connection Graph
          </CardTitle>
        </CardHeader>
        <CardContent>
          <div className="flex items-center justify-center h-[150px] text-muted-foreground">
            <AlertCircle className="w-5 h-5 mr-2" />
            <span>Failed to load connection graph</span>
          </div>
        </CardContent>
      </Card>
    )
  }

  // Empty state - completely hide the graph
  if (isEmpty) {
    return null
  }

  // Calculate container height based on graph bounds (minimum 150px)
  const containerHeight = Math.max(150, graphBounds.height)

  return (
    <Card className={className}>
      <CardHeader className="pb-2">
        <div className="flex items-center justify-between">
          <CardTitle className="text-lg flex items-center gap-2">
            <Network className="w-5 h-5" />
            Connection Graph
          </CardTitle>
          {connectedCount > 0 && (
            <span className="inline-flex items-center px-2 py-1 rounded-full text-xs font-medium bg-green-100 text-green-700 dark:bg-green-900 dark:text-green-300">
              {connectedCount} app{connectedCount !== 1 ? 's' : ''} connected
            </span>
          )}
        </div>
      </CardHeader>
      <CardContent className="p-0">
        <div style={{ height: containerHeight }} className="w-full rounded-b-lg overflow-hidden">
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onNodeClick={handleNodeClick}
            nodeTypes={nodeTypes}
            defaultViewport={{ x: 0, y: 0, zoom: 1 }}
            minZoom={0.5}
            maxZoom={1.5}
            nodesDraggable={false}
            nodesConnectable={false}
            elementsSelectable={false}
            panOnDrag={true}
            zoomOnScroll={true}
            preventScrolling={false}
            proOptions={{ hideAttribution: true }}
          >
            <Background color="#94a3b8" gap={16} size={1} />
            <Controls
              showZoom={true}
              showFitView={true}
              showInteractive={false}
              position="bottom-right"
            />
          </ReactFlow>
        </div>
      </CardContent>
    </Card>
  )
}
