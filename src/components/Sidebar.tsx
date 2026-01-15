type Tab = 'home' | 'api-keys' | 'providers'

interface SidebarProps {
  activeTab: Tab
  onTabChange: (tab: Tab) => void
}

export default function Sidebar({ activeTab, onTabChange }: SidebarProps) {
  const tabs = [
    { id: 'home' as Tab, label: 'Home' },
    { id: 'api-keys' as Tab, label: 'API Keys' },
    { id: 'providers' as Tab, label: 'Providers' },
  ]

  return (
    <nav className="w-[200px] bg-white border-r border-gray-200 shadow-sm py-4">
      {tabs.map((tab) => (
        <div
          key={tab.id}
          onClick={() => onTabChange(tab.id)}
          className={`
            px-6 py-3 cursor-pointer transition-all font-medium border-l-4
            ${
              activeTab === tab.id
                ? 'bg-blue-50 text-blue-600 border-blue-600'
                : 'text-gray-600 border-transparent hover:bg-gray-50 hover:text-gray-900'
            }
          `}
        >
          {tab.label}
        </div>
      ))}
    </nav>
  )
}
