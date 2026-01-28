import { memo } from 'react'
import { Handle, Position, type NodeProps } from 'reactflow'
import { cn } from '@/lib/utils'
import type { McpServerNodeData, ItemHealthStatus } from '../types'
import McpServerIcon from '@/components/McpServerIcon'

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

function McpServerNodeComponent({ data }: NodeProps<McpServerNodeData>) {
  const { name, healthStatus } = data

  return (
    <div
      className={cn(
        'relative px-2.5 py-1.5 rounded-lg border shadow-sm min-w-[120px]',
        'bg-gradient-to-br from-emerald-50 to-emerald-100 dark:from-emerald-950 dark:to-emerald-900',
        'border-emerald-300 dark:border-emerald-700',
        'cursor-pointer transition-all hover:scale-[1.02] hover:shadow-md'
      )}
    >
      {/* Input handle */}
      <Handle
        type="target"
        position={Position.Left}
        className="!w-2 !h-2 !bg-emerald-500 !border !border-white dark:!border-slate-900"
      />

      {/* Node content */}
      <div className="flex items-center gap-2">
        <div className="flex items-center justify-center w-6 h-6 rounded-full overflow-hidden">
          <McpServerIcon serverName={name} size={18} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-emerald-900 dark:text-emerald-100 truncate">
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

export const McpServerNode = memo(McpServerNodeComponent)
