import { memo } from 'react'
import { Handle, Position, type NodeProps } from 'reactflow'
import { Server } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { McpServerNodeData, ItemHealthStatus } from '../types'

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
        'relative px-5 py-4 rounded-xl border-2 shadow-md min-w-[220px]',
        'bg-gradient-to-br from-emerald-50 to-emerald-100 dark:from-emerald-950 dark:to-emerald-900',
        'border-emerald-300 dark:border-emerald-700'
      )}
    >
      {/* Input handle */}
      <Handle
        type="target"
        position={Position.Left}
        className="!w-3 !h-3 !bg-emerald-500 !border-2 !border-white dark:!border-slate-900"
      />

      {/* Node content */}
      <div className="flex items-center gap-3">
        <div className="flex items-center justify-center w-10 h-10 rounded-full bg-emerald-200 text-emerald-600 dark:bg-emerald-800 dark:text-emerald-300">
          <Server className="w-5 h-5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-base font-semibold text-emerald-900 dark:text-emerald-100 truncate">
            {name}
          </div>
        </div>
        {/* Health indicator dot */}
        <div
          className={cn(
            'w-4 h-4 rounded-full flex-shrink-0',
            getHealthDotClass(healthStatus)
          )}
        />
      </div>
    </div>
  )
}

export const McpServerNode = memo(McpServerNodeComponent)
