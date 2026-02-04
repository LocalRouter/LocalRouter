import { Monitor } from 'lucide-react'

interface MacOSMenuBarProps {
  onTrayClick: () => void
  trayOpen: boolean
}

export function MacOSMenuBar({ onTrayClick, trayOpen }: MacOSMenuBarProps) {
  const now = new Date()
  const timeString = now.toLocaleTimeString('en-US', {
    hour: 'numeric',
    minute: '2-digit',
    hour12: true,
  })

  return (
    <div className="h-6 bg-gradient-to-b from-[#f6f6f6] to-[#e8e8e8] border-b border-gray-300 flex items-center justify-between px-4 text-[13px] font-medium text-gray-800">
      <div className="flex items-center gap-5">
        <span className="font-bold"></span>
        <span>LocalRouter</span>
        <span className="text-gray-500">File</span>
        <span className="text-gray-500">Edit</span>
        <span className="text-gray-500">View</span>
        <span className="text-gray-500">Window</span>
        <span className="text-gray-500">Help</span>
      </div>
      <div className="flex items-center gap-3">
        <span className="text-gray-500 text-sm"></span>
        <span className="text-gray-500 text-sm"></span>
        <button
          onClick={onTrayClick}
          className={`p-1 rounded transition-colors ${
            trayOpen ? 'bg-blue-500 text-white' : 'hover:bg-gray-300/50'
          }`}
          title="LocalRouter Tray Menu"
        >
          <Monitor className="w-4 h-4" />
        </button>
        <span className="text-gray-500 text-sm">{timeString}</span>
      </div>
    </div>
  )
}
