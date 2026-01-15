import { useEffect, useState, useCallback, useMemo } from 'react';
import { invoke } from '@tauri-apps/api/core';
import ReactFlow, {
  Node,
  Edge,
  Controls,
  Background,
  useNodesState,
  useEdgesState,
  BackgroundVariant,
  MiniMap,
  NodeTypes,
  Panel,
} from 'reactflow';
import 'reactflow/dist/style.css';
import { ProviderNode } from '../visualization/ProviderNode';
import { ModelNode } from '../visualization/ModelNode';
import { ApiKeyNode } from '../visualization/ApiKeyNode';
import { AddProviderNode } from '../visualization/AddProviderNode';
import { AddApiKeyNode } from '../visualization/AddApiKeyNode';
import { ProviderSettingsModal } from '../visualization/modals/ProviderSettingsModal';
import { ModelChatModal } from '../visualization/modals/ModelChatModal';
import { ApiKeyChatModal } from '../visualization/modals/ApiKeyChatModal';
import { ProviderSelectionModal } from '../visualization/modals/ProviderSelectionModal';

interface VisualizationGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

interface GraphNode {
  id: string;
  type: string;
  label: string;
  data: NodeData;
  position?: { x: number; y: number };
}

type NodeData =
  | {
      nodeType: 'Provider';
      instance_name: string;
      provider_type: string;
      health: ProviderHealth;
      enabled: boolean;
    }
  | {
      nodeType: 'Model';
      model_id: string;
      provider_instance: string;
      capabilities: string[];
      context_window: number;
      supports_streaming: boolean;
    }
  | {
      nodeType: 'ApiKey';
      key_id: string;
      key_name: string;
      enabled: boolean;
      created_at: string;
      routing_strategy: string | null;
    }
  | { nodeType: 'AddNode' };

interface ProviderHealth {
  status: 'Healthy' | 'Degraded' | 'Unhealthy';
  latency_ms: number | null;
  last_checked: string;
  error_message: string | null;
}

interface GraphEdge {
  id: string;
  source: string;
  target: string;
  type: string;
}

// Layout configuration
const getLayoutedElements = (nodes: Node[], edges: Edge[]) => {
  // Group nodes by type
  const providers = nodes.filter((n) => n.type === 'provider');
  const models = nodes.filter((n) => n.type === 'model');
  const apiKeys = nodes.filter((n) => n.type === 'apikey');
  const addNodes = nodes.filter((n) => n.type === 'addprovider' || n.type === 'addapikey');

  // Column positions
  const columnWidth = 250;
  const verticalSpacing = 100;
  const startY = 50;

  // Position providers in left column
  providers.forEach((node, index) => {
    node.position = {
      x: 50,
      y: startY + index * verticalSpacing,
    };
  });

  // Position models in middle column
  models.forEach((node, index) => {
    node.position = {
      x: 50 + columnWidth,
      y: startY + index * verticalSpacing,
    };
  });

  // Position API keys in right column
  apiKeys.forEach((node, index) => {
    node.position = {
      x: 50 + columnWidth * 2,
      y: startY + index * verticalSpacing,
    };
  });

  // Position add nodes at the bottom of their respective columns
  const addProviderNode = addNodes.find((n) => n.type === 'addprovider');
  const addApiKeyNode = addNodes.find((n) => n.type === 'addapikey');

  if (addProviderNode) {
    addProviderNode.position = {
      x: 50,
      y: startY + providers.length * verticalSpacing + 50,
    };
  }

  if (addApiKeyNode) {
    addApiKeyNode.position = {
      x: 50 + columnWidth * 2,
      y: startY + apiKeys.length * verticalSpacing + 50,
    };
  }

  return { nodes, edges };
};

export function VisualizationTab() {
  const [nodes, setNodes, onNodesChange] = useNodesState([]);
  const [edges, setEdges, onEdgesChange] = useEdgesState([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Modal states
  const [providerModal, setProviderModal] = useState<{
    isOpen: boolean;
    data: any;
  }>({ isOpen: false, data: null });

  const [modelModal, setModelModal] = useState<{
    isOpen: boolean;
    data: any;
  }>({ isOpen: false, data: null });

  const [apiKeyModal, setApiKeyModal] = useState<{
    isOpen: boolean;
    data: any;
  }>({ isOpen: false, data: null });

  const [addProviderModal, setAddProviderModal] = useState(false);

  // Define custom node types
  const nodeTypes: NodeTypes = useMemo(
    () => ({
      provider: ProviderNode,
      model: ModelNode,
      apikey: ApiKeyNode,
      addprovider: AddProviderNode,
      addapikey: AddApiKeyNode,
    }),
    []
  );

  // Click handlers
  const handleProviderClick = useCallback((providerData: any) => {
    setProviderModal({ isOpen: true, data: providerData });
  }, []);

  const handleModelClick = useCallback((modelData: any) => {
    setModelModal({ isOpen: true, data: modelData });
  }, []);

  const handleApiKeyClick = useCallback((apiKeyData: any) => {
    setApiKeyModal({ isOpen: true, data: apiKeyData });
  }, []);

  const handleAddProviderClick = useCallback(() => {
    setAddProviderModal(true);
  }, []);

  // Modal close handlers
  const closeProviderModal = useCallback(() => {
    setProviderModal({ isOpen: false, data: null });
  }, []);

  const closeModelModal = useCallback(() => {
    setModelModal({ isOpen: false, data: null });
  }, []);

  const closeApiKeyModal = useCallback(() => {
    setApiKeyModal({ isOpen: false, data: null });
  }, []);

  const closeAddProviderModal = useCallback(() => {
    setAddProviderModal(false);
  }, []);

  const handleAddApiKeyClick = useCallback(async () => {
    // Create new API key immediately with default settings
    try {
      await invoke('create_api_key', {
        name: null,
        modelSelection: null,
      });
      // Graph will auto-refresh within 5 seconds
    } catch (err) {
      console.error('Failed to create API key:', err);
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const loadGraph = useCallback(async () => {
    try {
      setIsLoading(true);
      setError(null);

      const graph = await invoke<VisualizationGraph>('get_visualization_graph');

      // Convert graph nodes to React Flow nodes with click handlers
      const flowNodes: Node[] = graph.nodes.map((node) => {
        let onClick;
        switch (node.type) {
          case 'provider':
            onClick = () => handleProviderClick(node.data);
            break;
          case 'model':
            onClick = () => handleModelClick(node.data);
            break;
          case 'apikey':
            onClick = () => handleApiKeyClick(node.data);
            break;
          case 'addprovider':
            onClick = handleAddProviderClick;
            break;
          case 'addapikey':
            onClick = handleAddApiKeyClick;
            break;
        }

        return {
          id: node.id,
          type: node.type,
          position: node.position || { x: 0, y: 0 },
          data: { label: node.label, ...node.data, onClick },
        };
      });

      // Convert graph edges to React Flow edges
      const flowEdges: Edge[] = graph.edges.map((edge) => ({
        id: edge.id,
        source: edge.source,
        target: edge.target,
        type: 'default',
        style: {
          stroke: edge.type === 'providertomodel' ? '#8b5cf6' : '#10b981',
          strokeWidth: 2,
          strokeDasharray: edge.type === 'apikeytomodel' ? '5,5' : undefined,
        },
        animated: false,
      }));

      // Apply column-based layout
      const { nodes: layoutedNodes, edges: layoutedEdges } = getLayoutedElements(
        flowNodes,
        flowEdges
      );

      setNodes(layoutedNodes);
      setEdges(layoutedEdges);
      setIsLoading(false);
    } catch (err) {
      console.error('Failed to load visualization graph:', err);
      setError(err instanceof Error ? err.message : String(err));
      setIsLoading(false);
    }
  }, [
    setNodes,
    setEdges,
    handleProviderClick,
    handleModelClick,
    handleApiKeyClick,
    handleAddProviderClick,
    handleAddApiKeyClick,
  ]);

  useEffect(() => {
    loadGraph();

    // Auto-refresh every 5 seconds
    const interval = setInterval(loadGraph, 5000);
    return () => clearInterval(interval);
  }, [loadGraph]);

  if (isLoading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="text-gray-600">Loading visualization...</div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="p-4">
        <div className="bg-red-50 border border-red-200 rounded-lg p-4">
          <h3 className="text-red-800 font-semibold mb-2">Error Loading Graph</h3>
          <p className="text-red-600">{error}</p>
          <button
            onClick={loadGraph}
            className="mt-4 px-4 py-2 bg-red-600 text-white rounded hover:bg-red-700"
          >
            Retry
          </button>
        </div>
      </div>
    );
  }

  return (
    <>
      <div style={{ width: '100%', height: '100%' }}>
        <ReactFlow
          nodes={nodes}
          edges={edges}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          nodeTypes={nodeTypes}
          fitView
          minZoom={0.1}
          maxZoom={2}
        >
          <Controls />
          <MiniMap />
          <Background variant={BackgroundVariant.Dots} gap={12} size={1} />

          {/* Column Headers */}
          <Panel position="top-left" style={{ marginTop: 10, marginLeft: 10 }}>
            <div className="flex gap-[200px] text-sm font-semibold text-gray-700">
              <div className="w-[200px] text-center">Providers</div>
              <div className="w-[200px] text-center">Models</div>
              <div className="w-[200px] text-center">API Keys</div>
            </div>
          </Panel>
        </ReactFlow>
      </div>

      {/* Modals */}
      {providerModal.isOpen && providerModal.data && (
        <ProviderSettingsModal
          isOpen={providerModal.isOpen}
          onClose={closeProviderModal}
          providerData={providerModal.data}
          onUpdate={loadGraph}
        />
      )}

      {modelModal.isOpen && modelModal.data && (
        <ModelChatModal
          isOpen={modelModal.isOpen}
          onClose={closeModelModal}
          modelData={modelModal.data}
        />
      )}

      {apiKeyModal.isOpen && apiKeyModal.data && (
        <ApiKeyChatModal
          isOpen={apiKeyModal.isOpen}
          onClose={closeApiKeyModal}
          apiKeyData={apiKeyModal.data}
          onUpdate={loadGraph}
        />
      )}

      {addProviderModal && (
        <ProviderSelectionModal
          isOpen={addProviderModal}
          onClose={closeAddProviderModal}
          onCreate={loadGraph}
        />
      )}
    </>
  );
}
