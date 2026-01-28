import { memo } from 'react'
import { Handle, Position, type NodeProps } from 'reactflow'
import { cn } from '@/lib/utils'
import type { ProviderNodeData, ItemHealthStatus } from '../types'
import ProviderIcon from '@/components/ProviderIcon'

// Get health dot color based on status
function getHealthDotClass(status: ItemHealthStatus): string {
  switch (status) {
    case 'healthy':
    case 'ready':
      return 'bg-green-500'
    case 'degraded':
      return 'bg-yellow-500'
    case 'unhealthy':
      return 'bg-red-500'
    case 'pending':
      return 'bg-slate-400 animate-pulse'
    case 'disabled':
      return 'bg-slate-300 dark:bg-slate-600'
    default:
      return 'bg-slate-400'
  }
}

function ProviderNodeComponent({ data }: NodeProps<ProviderNodeData>) {
  const { name, healthStatus, providerType } = data

  return (
    <div
      className={cn(
        'relative px-2.5 py-1.5 rounded-lg border shadow-sm min-w-[120px]',
        'bg-gradient-to-br from-violet-50 to-violet-100 dark:from-violet-950 dark:to-violet-900',
        'border-violet-300 dark:border-violet-700',
        'cursor-pointer transition-all hover:scale-[1.02] hover:shadow-md'
      )}
    >
      {/* Input handle */}
      <Handle
        type="target"
        position={Position.Left}
        className="!w-2 !h-2 !bg-violet-500 !border !border-white dark:!border-slate-900"
      />

      {/* Node content */}
      <div className="flex items-center gap-2">
        <div className="flex items-center justify-center w-6 h-6 rounded-full overflow-hidden">
          <ProviderIcon providerId={providerType} size={18} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-violet-900 dark:text-violet-100 truncate">
            {name}
          </div>
        </div>
        {/* Health indicator dot */}
        <div
          className={cn(
            'w-2.5 h-2.5 rounded-full flex-shrink-0',
            getHealthDotClass(healthStatus)
          )}
        />
      </div>
    </div>
  )
}

export const ProviderNode = memo(ProviderNodeComponent)
