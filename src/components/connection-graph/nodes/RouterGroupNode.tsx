import { memo } from 'react'
import type { NodeProps } from 'reactflow'
import type { RouterGroupNodeData } from '../types'

function RouterGroupNodeComponent(_props: NodeProps<RouterGroupNodeData>) {
  return (
    <div className="w-full h-full rounded-xl border-2 border-purple-300/60 dark:border-purple-600/40 bg-purple-100/30 dark:bg-purple-950/25 shadow-sm">
      <div className="text-[10px] font-semibold text-purple-600/80 dark:text-purple-400/70 uppercase tracking-wider pt-1.5 pl-2.5">
        LocalRouter
      </div>
    </div>
  )
}

export const RouterGroupNode = memo(RouterGroupNodeComponent)
