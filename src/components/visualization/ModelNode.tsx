import { Handle, Position } from 'reactflow';

interface ModelNodeData {
  nodeType: 'Model';
  model_id: string;
  provider_instance: string;
  capabilities: string[];
  context_window: number;
  supports_streaming: boolean;
  label?: string;
  onClick?: () => void;
}

export function ModelNode({ data }: { data: ModelNodeData }) {
  const formatContextWindow = (tokens: number) => {
    if (tokens >= 1000000) {
      return `${(tokens / 1000000).toFixed(1)}M`;
    } else if (tokens >= 1000) {
      return `${(tokens / 1000).toFixed(0)}K`;
    }
    return tokens.toString();
  };

  const getCapabilityIcon = (capability: string) => {
    switch (capability) {
      case 'Chat':
        return '💬';
      case 'Vision':
        return '👁️';
      case 'FunctionCalling':
        return '🔧';
      case 'Embedding':
        return '📊';
      default:
        return '';
    }
  };

  return (
    <div
      className="px-4 py-3 rounded-lg border-2 border-purple-500 bg-purple-50 shadow-md min-w-[180px] cursor-pointer hover:shadow-lg transition-shadow"
      onClick={data.onClick}
    >
      <Handle type="target" position={Position.Left} className="w-3 h-3 !bg-purple-500" />

      <div className="text-xs font-semibold text-gray-500 uppercase mb-1">Model</div>

      <div className="font-semibold text-sm text-gray-900">{data.label || data.model_id}</div>

      <div className="flex items-center gap-2 mt-2">
        <div className="text-xs text-gray-600">
          {formatContextWindow(data.context_window)} ctx
        </div>
        {data.supports_streaming && (
          <div className="text-xs text-purple-600 font-medium">Stream</div>
        )}
      </div>

      {data.capabilities.length > 0 && (
        <div className="flex gap-1 mt-1">
          {data.capabilities.slice(0, 3).map((cap) => (
            <span key={cap} className="text-xs" title={cap}>
              {getCapabilityIcon(cap)}
            </span>
          ))}
        </div>
      )}

      <Handle type="source" position={Position.Right} className="w-3 h-3 !bg-purple-500" />
    </div>
  );
}
