import { memo } from 'react'
import { Handle, Position, type NodeProps } from 'reactflow'
import { Store } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { MarketplaceNodeData } from '../types'

function MarketplaceNodeComponent({ data }: NodeProps<MarketplaceNodeData>) {
  const { name } = data

  return (
    <div
      className={cn(
        'relative px-2.5 py-1.5 rounded-lg border shadow-sm min-w-[120px]',
        'bg-gradient-to-br from-pink-50 to-pink-100 dark:from-pink-950 dark:to-pink-900',
        'border-pink-300 dark:border-pink-700',
        'cursor-pointer transition-all hover:scale-[1.02] hover:shadow-md'
      )}
    >
      {/* Input handle */}
      <Handle
        type="target"
        position={Position.Left}
        className="!w-2 !h-2 !bg-pink-500 !border !border-white dark:!border-slate-900"
      />

      {/* Node content */}
      <div className="flex items-center gap-2">
        <div className="flex items-center justify-center w-6 h-6 rounded-full bg-pink-200 dark:bg-pink-800">
          <Store className="w-3.5 h-3.5 text-pink-700 dark:text-pink-300" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-pink-900 dark:text-pink-100 truncate">
            {name}
          </div>
        </div>
      </div>
    </div>
  )
}

export const MarketplaceNode = memo(MarketplaceNodeComponent)
