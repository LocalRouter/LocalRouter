import { useState } from 'react'
import GraphSubtab from './preferences/GraphSubtab'
import ServerSubtab from './preferences/ServerSubtab'
import UpdatesSubtab from './preferences/UpdatesSubtab'
import SmartRoutingSubtab from './preferences/SmartRoutingSubtab'

type SubtabType = 'server' | 'updates' | 'graph' | 'smart-routing'

interface SettingsPageProps {
  initialSubtab?: SubtabType
}

export default function SettingsPage({ initialSubtab = 'server' }: SettingsPageProps) {
  const [activeSubtab, setActiveSubtab] = useState<SubtabType>(initialSubtab || 'server')

  const subtabs: { id: SubtabType; label: string; badge?: boolean }[] = [
    { id: 'server', label: 'Server' },
    { id: 'updates', label: 'Updates' },
    { id: 'graph', label: 'Graph' },
    { id: 'smart-routing', label: 'Smart routing' },
  ]

  return (
    <div className="h-full flex flex-col">
      {/* Page Header */}
      <div className="px-6 py-4 border-b border-gray-200 dark:border-gray-700">
        <h1 className="text-2xl font-bold text-gray-900 dark:text-gray-100">Settings</h1>
      </div>

      {/* Horizontal Subtab Navigation */}
      <div className="border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/50">
        <div className="flex px-6">
          {subtabs.map((subtab) => (
            <button
              key={subtab.id}
              onClick={() => setActiveSubtab(subtab.id)}
              className={`
                px-6 py-3 text-sm font-medium border-b-2 transition-colors relative
                ${
                  activeSubtab === subtab.id
                    ? 'text-blue-600 dark:text-blue-400 border-blue-600 dark:border-blue-400'
                    : 'text-gray-600 dark:text-gray-400 border-transparent hover:text-gray-900 dark:hover:text-gray-200'
                }
              `}
            >
              {subtab.label}
              {subtab.badge && (
                <span className="ml-2 inline-flex items-center justify-center w-2 h-2 bg-blue-500 dark:bg-blue-400 rounded-full" />
              )}
            </button>
          ))}
        </div>
      </div>

      {/* Subtab Content */}
      <div className="flex-1 overflow-y-auto">
        {activeSubtab === 'server' && <ServerSubtab />}
        {activeSubtab === 'updates' && <UpdatesSubtab />}
        {activeSubtab === 'graph' && <GraphSubtab />}
        {activeSubtab === 'smart-routing' && <SmartRoutingSubtab />}
      </div>
    </div>
  )
}
