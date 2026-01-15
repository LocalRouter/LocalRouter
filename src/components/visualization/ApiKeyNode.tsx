import { Handle, Position } from 'reactflow';

interface ApiKeyNodeData {
  nodeType: 'ApiKey';
  key_id: string;
  key_name: string;
  enabled: boolean;
  created_at: string;
  routing_strategy: string | null;
  onClick?: () => void;
}

export function ApiKeyNode({ data }: { data: ApiKeyNodeData }) {
  const getStrategyLabel = (strategy: string | null) => {
    if (!strategy) return 'No routing';
    switch (strategy) {
      case 'Available Models':
        return 'Available';
      case 'Force Model':
        return 'Force';
      case 'Prioritized List':
        return 'Priority';
      default:
        return strategy;
    }
  };

  return (
    <div
      className={`px-4 py-3 rounded-lg border-2 border-green-500 bg-green-50 shadow-md min-w-[180px] cursor-pointer hover:shadow-lg transition-shadow ${
        !data.enabled ? 'opacity-60' : ''
      }`}
      onClick={data.onClick}
    >
      <Handle type="target" position={Position.Left} className="w-3 h-3 !bg-green-500" />

      <div className="flex items-center justify-between gap-2 mb-1">
        <div className="text-xs font-semibold text-gray-500 uppercase">API Key</div>
        {!data.enabled && (
          <div className="w-2 h-2 rounded-full bg-red-500" title="Disabled" />
        )}
      </div>

      <div className="font-semibold text-sm text-gray-900">{data.key_name}</div>

      {data.routing_strategy && (
        <div className="text-xs text-green-700 font-medium mt-1">
          {getStrategyLabel(data.routing_strategy)}
        </div>
      )}

      {!data.enabled && (
        <div className="text-xs text-red-600 font-medium mt-1">Disabled</div>
      )}

      <Handle type="source" position={Position.Right} className="w-3 h-3 !bg-green-500" />
    </div>
  );
}
