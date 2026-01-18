import { ReactNode } from 'react'
import Card from '../ui/Card'
import Badge from '../ui/Badge'
import Button from '../ui/Button'

export interface TabConfig {
  id: string
  label: string
  count?: number
  content: ReactNode
}

export interface DetailPageLayoutProps {
  icon?: ReactNode
  title: string
  subtitle?: string
  badges?: Array<{ label: string; variant: 'success' | 'warning' | 'error' | 'default' }>
  actions?: ReactNode
  tabs: TabConfig[]
  activeTab: string
  onTabChange: (tabId: string) => void
  loading?: boolean
}

export default function DetailPageLayout({
  icon,
  title,
  subtitle,
  badges = [],
  actions,
  tabs,
  activeTab,
  onTabChange,
  loading = false,
}: DetailPageLayoutProps) {
  if (loading) {
    return (
      <div className="bg-white dark:bg-gray-800 rounded-lg shadow-sm p-6">
        <div className="text-center py-8 text-gray-500 dark:text-gray-400">Loading...</div>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      {/* Header Card */}
      <Card>
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-4">
            {icon && <div>{icon}</div>}
            <div>
              <h2 className="text-2xl font-bold text-gray-900 dark:text-gray-100">{title}</h2>
              {subtitle && <p className="text-sm text-gray-500 dark:text-gray-400">{subtitle}</p>}
            </div>
          </div>
          <div className="flex items-center gap-3">
            {badges.map((badge, index) => (
              <Badge key={index} variant={badge.variant}>
                {badge.label}
              </Badge>
            ))}
            {actions && <div className="flex items-center gap-2">{actions}</div>}
          </div>
        </div>
      </Card>

      {/* Tab Navigation */}
      <div className="flex border-b border-gray-200 dark:border-gray-700">
        {tabs.map((tab) => (
          <button
            key={tab.id}
            onClick={() => onTabChange(tab.id)}
            className={`px-6 py-3 font-medium transition-colors ${
              activeTab === tab.id
                ? 'border-b-2 border-blue-500 dark:border-blue-400 text-blue-600 dark:text-blue-400'
                : 'text-gray-600 dark:text-gray-400 hover:text-gray-900 dark:hover:text-gray-100'
            }`}
          >
            {tab.label}
            {tab.count !== undefined && ` (${tab.count})`}
          </button>
        ))}
      </div>

      {/* Tab Content */}
      {tabs.map((tab) => (
        <div key={tab.id} className={activeTab === tab.id ? 'block' : 'hidden'}>
          {tab.content}
        </div>
      ))}
    </div>
  )
}
