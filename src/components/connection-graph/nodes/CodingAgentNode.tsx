import { memo } from 'react'
import { Handle, Position, type NodeProps } from 'reactflow'
import { cn } from '@/lib/utils'
import { CodingAgentsIcon } from '@/components/icons/category-icons'
import type { CodingAgentNodeData } from '../types'

function CodingAgentNodeComponent({ data }: NodeProps<CodingAgentNodeData>) {
  const { name } = data

  return (
    <div
      className={cn(
        'relative px-2.5 py-1.5 rounded-lg border shadow-sm min-w-[120px]',
        'bg-gradient-to-br from-orange-50 to-orange-100 dark:from-orange-950 dark:to-orange-900',
        'border-orange-300 dark:border-orange-700',
        'cursor-pointer transition-all hover:scale-[1.02] hover:shadow-md'
      )}
    >
      {/* Input handle */}
      <Handle
        type="target"
        position={Position.Left}
        className="!w-2 !h-2 !bg-orange-500 !border !border-white dark:!border-slate-900"
      />

      {/* Node content */}
      <div className="flex items-center gap-2">
        <div className="flex items-center justify-center w-6 h-6 rounded-full bg-orange-200 dark:bg-orange-800">
          <CodingAgentsIcon className="w-3.5 h-3.5 text-orange-700 dark:text-orange-300" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-orange-900 dark:text-orange-100 truncate">
            {name}
          </div>
        </div>
      </div>
    </div>
  )
}

export const CodingAgentNode = memo(CodingAgentNodeComponent)
