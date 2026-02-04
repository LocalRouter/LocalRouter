import { useState } from 'react'
import { MacOSMenuBar } from './MacOSMenuBar'
import { MacOSTrayMenu } from './MacOSTrayMenu'
import { MacOSWindow } from './MacOSWindow'
import { DemoBanner } from './DemoBanner'

export function LocalRouterDemo() {
  const [trayOpen, setTrayOpen] = useState(false)

  return (
    <div className="relative mx-auto max-w-5xl">
      {/* macOS Menu Bar */}
      <div className="rounded-t-lg overflow-hidden border border-b-0 border-gray-300/50">
        <MacOSMenuBar
          onTrayClick={() => setTrayOpen(!trayOpen)}
          trayOpen={trayOpen}
        />
      </div>

      {/* Tray Menu Dropdown */}
      {trayOpen && <MacOSTrayMenu onClose={() => setTrayOpen(false)} />}

      {/* App Window - using iframe to isolate overlays/modals */}
      <MacOSWindow title="LocalRouter" height={600}>
        <iframe
          src="/demo"
          className="w-full h-full border-0"
          title="LocalRouter Demo"
        />
      </MacOSWindow>

      {/* Demo mode indicator */}
      <DemoBanner />
    </div>
  )
}
