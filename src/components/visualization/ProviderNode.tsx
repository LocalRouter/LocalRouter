import { Handle, Position } from 'reactflow';

interface ProviderHealth {
  status: 'Healthy' | 'Degraded' | 'Unhealthy';
  latency_ms: number | null;
  last_checked: string;
  error_message: string | null;
}

interface ProviderNodeData {
  nodeType: 'Provider';
  instance_name: string;
  provider_type: string;
  health: ProviderHealth;
  enabled: boolean;
  onClick?: () => void;
}

export function ProviderNode({ data }: { data: ProviderNodeData }) {
  const getHealthColor = () => {
    switch (data.health.status) {
      case 'Healthy':
        return 'border-green-500 bg-green-50';
      case 'Degraded':
        return 'border-yellow-500 bg-yellow-50';
      case 'Unhealthy':
        return 'border-red-500 bg-red-50';
      default:
        return 'border-gray-400 bg-gray-50';
    }
  };

  const getHealthBadgeColor = () => {
    switch (data.health.status) {
      case 'Healthy':
        return 'bg-green-500';
      case 'Degraded':
        return 'bg-yellow-500';
      case 'Unhealthy':
        return 'bg-red-500';
      default:
        return 'bg-gray-400';
    }
  };

  return (
    <div
      className={`px-4 py-3 rounded-lg border-2 shadow-md min-w-[180px] cursor-pointer hover:shadow-lg transition-shadow ${getHealthColor()} ${
        !data.enabled ? 'opacity-60' : ''
      }`}
      onClick={data.onClick}
    >
      <Handle type="target" position={Position.Left} className="w-3 h-3 !bg-blue-500" />

      <div className="flex items-center justify-between gap-2 mb-1">
        <div className="text-xs font-semibold text-gray-500 uppercase">Provider</div>
        <div
          className={`w-2 h-2 rounded-full ${getHealthBadgeColor()}`}
          title={`${data.health.status}${
            data.health.latency_ms ? ` (${data.health.latency_ms}ms)` : ''
          }`}
        />
      </div>

      <div className="font-semibold text-sm text-gray-900">{data.instance_name}</div>
      <div className="text-xs text-gray-600 mt-0.5">{data.provider_type}</div>

      {!data.enabled && (
        <div className="text-xs text-red-600 font-medium mt-1">Disabled</div>
      )}

      <Handle type="source" position={Position.Right} className="w-3 h-3 !bg-blue-500" />
    </div>
  );
}
