import { memo } from 'react'
import { Handle, Position, type NodeProps } from 'reactflow'
import { cn } from '@/lib/utils'
import { Cpu, Server } from 'lucide-react'
import type { EndpointNodeData } from '../types'

const variantConfig = {
  llm: {
    label: 'LLM',
    icon: Cpu,
    bg: 'from-blue-50 to-blue-100 dark:from-blue-950 dark:to-blue-900',
    border: 'border-blue-300 dark:border-blue-700',
    text: 'text-blue-900 dark:text-blue-100',
    handle: '!bg-blue-500',
  },
  mcp: {
    label: 'MCP',
    icon: Server,
    bg: 'from-emerald-50 to-emerald-100 dark:from-emerald-950 dark:to-emerald-900',
    border: 'border-emerald-300 dark:border-emerald-700',
    text: 'text-emerald-900 dark:text-emerald-100',
    handle: '!bg-emerald-500',
  },
}

function EndpointNodeComponent({ data }: NodeProps<EndpointNodeData>) {
  const config = variantConfig[data.variant]
  const Icon = config.icon

  return (
    <div
      className={cn(
        'relative px-2.5 py-1.5 rounded-lg border shadow-sm',
        'bg-gradient-to-br',
        config.bg,
        config.border,
      )}
    >
      <Handle
        type="target"
        position={Position.Left}
        className={cn('!w-2 !h-2 !border !border-white dark:!border-slate-900', config.handle)}
      />

      <div className="flex items-center gap-1.5">
        <Icon className={cn('w-3.5 h-3.5', config.text)} />
        <span className={cn('text-xs font-medium whitespace-nowrap', config.text)}>{data.name}</span>
      </div>

      <Handle
        type="source"
        position={Position.Right}
        className={cn('!w-2 !h-2 !border !border-white dark:!border-slate-900', config.handle)}
      />
    </div>
  )
}

export const EndpointNode = memo(EndpointNodeComponent)
