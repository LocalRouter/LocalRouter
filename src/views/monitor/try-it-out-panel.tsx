import { useState, useEffect } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Button } from '@/components/ui/Button'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/Select'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { X } from 'lucide-react'
import { LlmTab } from '@/views/try-it-out/llm-tab'
import { McpTab } from '@/views/try-it-out/mcp-tab'

interface Client {
  id: string
  name: string
  client_id: string
  enabled: boolean
  client_mode: string
}

interface TryItOutPanelProps {
  onClose: () => void
}

export function TryItOutPanel({ onClose }: TryItOutPanelProps) {
  const [clients, setClients] = useState<Client[]>([])
  const [selectedClientId, setSelectedClientId] = useState<string | null>(null)
  const [activeTab, setActiveTab] = useState<string>('llm')

  useEffect(() => {
    invoke<Client[]>('list_clients').then(setClients).catch(() => {})
  }, [])

  const selectedClient = clients.find(c => c.client_id === selectedClientId)
  const showLlm = selectedClient ? selectedClient.client_mode !== 'mcp_only' : false
  const showMcp = selectedClient ? (selectedClient.client_mode === 'both' || selectedClient.client_mode === 'mcp_only') : false

  // Auto-switch tab when selected client doesn't support the current tab
  useEffect(() => {
    if (showLlm && !showMcp && activeTab === 'mcp') setActiveTab('llm')
    if (showMcp && !showLlm && activeTab === 'llm') setActiveTab('mcp')
  }, [selectedClientId, showLlm, showMcp])

  return (
    <div className="flex flex-col h-full bg-background">
      {/* Header */}
      <div className="flex items-center justify-between p-2 border-b">
        <span className="text-xs font-medium">Try It Out</span>
        <Button variant="ghost" size="sm" className="h-6 w-6 p-0" onClick={onClose}>
          <X className="h-3 w-3" />
        </Button>
      </div>

      {/* Client selector */}
      <div className="p-2 border-b">
        <Select
          value={selectedClientId ?? ''}
          onValueChange={(v) => setSelectedClientId(v || null)}
        >
          <SelectTrigger className="h-7 text-xs">
            <SelectValue placeholder="Select a client..." />
          </SelectTrigger>
          <SelectContent>
            {clients.filter(c => c.enabled).map(c => (
              <SelectItem key={c.client_id} value={c.client_id} className="text-xs">
                {c.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {/* Content */}
      {selectedClientId ? (
        <div className="flex-1 overflow-hidden">
          {showLlm && showMcp ? (
            <Tabs value={activeTab} onValueChange={setActiveTab} className="h-full flex flex-col">
              <TabsList className="mx-2 mt-2">
                <TabsTrigger value="llm" className="text-xs">LLM</TabsTrigger>
                <TabsTrigger value="mcp" className="text-xs">MCP</TabsTrigger>
              </TabsList>
              <TabsContent value="llm" className="flex-1 overflow-auto mt-0">
                <LlmTab
                  initialMode="client"
                  initialClientId={selectedClientId}
                  hideModeSwitcher
                />
              </TabsContent>
              <TabsContent value="mcp" className="flex-1 overflow-auto mt-0">
                <McpTab
                  initialMode="client"
                  initialClientId={selectedClientId}
                  hideModeSwitcher
                  innerPath={null}
                  onPathChange={() => {}}
                />
              </TabsContent>
            </Tabs>
          ) : showLlm ? (
            <div className="h-full overflow-auto">
              <LlmTab
                initialMode="client"
                initialClientId={selectedClientId}
                hideModeSwitcher
              />
            </div>
          ) : (
            <div className="h-full overflow-auto">
              <McpTab
                initialMode="client"
                initialClientId={selectedClientId}
                hideModeSwitcher
                innerPath={null}
                onPathChange={() => {}}
              />
            </div>
          )}
        </div>
      ) : (
        <div className="flex-1 flex items-center justify-center text-muted-foreground text-xs">
          Select a client to get started
        </div>
      )}
    </div>
  )
}
