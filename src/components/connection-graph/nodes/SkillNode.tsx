import { memo } from 'react'
import { Handle, Position, type NodeProps } from 'reactflow'
import { cn } from '@/lib/utils'
import { SkillsIcon } from '@/components/icons/category-icons'
import type { SkillNodeData } from '../types'

function SkillNodeComponent({ data }: NodeProps<SkillNodeData>) {
  const { name } = data

  return (
    <div
      className={cn(
        'relative px-2.5 py-1.5 rounded-lg border shadow-sm min-w-[120px]',
        'bg-gradient-to-br from-amber-50 to-amber-100 dark:from-amber-950 dark:to-amber-900',
        'border-amber-300 dark:border-amber-700',
        'cursor-pointer transition-all hover:scale-[1.02] hover:shadow-md'
      )}
    >
      {/* Input handle */}
      <Handle
        type="target"
        position={Position.Left}
        className="!w-2 !h-2 !bg-amber-500 !border !border-white dark:!border-slate-900"
      />

      {/* Node content */}
      <div className="flex items-center gap-2">
        <div className="flex items-center justify-center w-6 h-6 rounded-full bg-amber-200 dark:bg-amber-800">
          <SkillsIcon className="w-3.5 h-3.5 text-amber-700 dark:text-amber-300" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-amber-900 dark:text-amber-100 truncate">
            {name}
          </div>
        </div>
      </div>
    </div>
  )
}

export const SkillNode = memo(SkillNodeComponent)
