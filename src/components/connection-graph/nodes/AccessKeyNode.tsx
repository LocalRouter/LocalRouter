import { memo } from 'react'
import { Handle, Position, type NodeProps } from 'reactflow'
import { Wifi, Key } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { AccessKeyNodeData } from '../types'

function AccessKeyNodeComponent({ data }: NodeProps<AccessKeyNodeData>) {
  const { name, isConnected } = data

  return (
    <div
      className={cn(
        'relative px-5 py-4 rounded-xl border-2 shadow-md min-w-[220px]',
        'bg-gradient-to-br from-blue-50 to-blue-100 dark:from-blue-950 dark:to-blue-900',
        isConnected
          ? 'border-blue-500 shadow-blue-200 dark:shadow-blue-900'
          : 'border-blue-300 dark:border-blue-700'
      )}
    >
      {/* Node content */}
      <div className="flex items-center gap-3">
        <div className={cn(
          'flex items-center justify-center w-10 h-10 rounded-full',
          isConnected
            ? 'bg-blue-500 text-white'
            : 'bg-blue-200 text-blue-600 dark:bg-blue-800 dark:text-blue-300'
        )}>
          <Key className="w-5 h-5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-base font-semibold text-blue-900 dark:text-blue-100 truncate">
            {name}
          </div>
          {isConnected && (
            <div className="flex items-center gap-1.5 text-sm text-green-600 dark:text-green-400 font-medium">
              <Wifi className="w-4 h-4" />
              <span>Connected</span>
            </div>
          )}
        </div>
      </div>

      {/* Output handle */}
      <Handle
        type="source"
        position={Position.Right}
        className="!w-3 !h-3 !bg-blue-500 !border-2 !border-white dark:!border-slate-900"
      />
    </div>
  )
}

export const AccessKeyNode = memo(AccessKeyNodeComponent)
