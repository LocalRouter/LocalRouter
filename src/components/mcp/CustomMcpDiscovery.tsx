import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Loader2, Search } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import type { DiscoverMcpConnectionParams, McpConnectionDiscovery } from '@/types/tauri-commands'

interface Props {
  params: DiscoverMcpConnectionParams
  /** Includes manual fields so pending results cannot overwrite newer edits. */
  formRevision: string
  onTargetChange: (target: string) => void
  onDiscovered: (result: McpConnectionDiscovery) => void
}

export function CustomMcpDiscovery({ params, formRevision, onTargetChange, onDiscovered }: Props) {
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<McpConnectionDiscovery | null>(null)
  const [error, setError] = useState<string | null>(null)
  const generation = useRef(0)
  const currentRevision = useRef(formRevision)
  currentRevision.current = formRevision
  const connectionKey = JSON.stringify(params)

  useEffect(() => {
    generation.current++
    setBusy(false)
    setResult(null)
    setError(null)
    return () => { generation.current++ }
  }, [connectionKey])

  const discover = async () => {
    const requestGeneration = ++generation.current
    const revision = formRevision
    setBusy(true)
    setError(null)
    setResult(null)
    try {
      const discovered = await invoke<McpConnectionDiscovery>('discover_mcp_connection', { ...params } satisfies DiscoverMcpConnectionParams)
      if (requestGeneration !== generation.current) return
      setResult(discovered)
      if (revision === currentRevision.current) onDiscovered(discovered)
      else setError('Settings changed during discovery. Review the result below; your edits were kept.')
    } catch (e) {
      if (requestGeneration === generation.current) setError(typeof e === 'string' ? e : 'Discovery failed. Configure the server manually or try again.')
    } finally {
      if (requestGeneration === generation.current) setBusy(false)
    }
  }

  const authentication = result?.auth_method === 'oauth_browser' ? 'OAuth browser login'
    : result?.auth_method === 'bearer' ? 'Bearer token'
    : result?.auth_method === 'manual' ? 'Credentials required; configure below' : 'No authentication advertised'

  return (
    <div className="space-y-2 rounded-md border p-3">
      <label htmlFor="custom-mcp-target" className="block text-sm font-medium">URL or command</label>
      <div className="flex gap-2">
        <Input id="custom-mcp-target" value={params.target} onChange={e => onTargetChange(e.target.value)}
          placeholder="https://example.com/mcp or npx -y my-mcp-server" spellCheck={false} />
        <Button type="button" variant="outline" onClick={discover} disabled={busy || !params.target.trim()}>
          {busy ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : <Search className="mr-2 h-4 w-4" />}
          {busy ? 'Discovering…' : 'Discover'}
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">Detect server details and login requirements. Commands run when you click Discover. You can edit all settings below.</p>
      {error && <p role="alert" className="text-xs text-destructive">{error}</p>}
      {result && (
        <div role="status" className="space-y-1 text-xs">
          <p className="font-medium">{result.server_name || 'Connection detected'}{result.server_version ? ` (${result.server_version})` : ''}</p>
          <p>{result.transport === 'stdio' ? 'STDIO subprocess' : 'HTTP (Streamable HTTP / SSE)'} · {authentication}</p>
          {result.protocol_versions.length > 0 && <p>MCP versions: {result.protocol_versions.join(', ')}</p>}
          {result.capabilities && <p>Capabilities: {Object.keys(result.capabilities).join(', ') || 'None advertised'}</p>}
          {result.oauth && <p>Authorization server: {result.oauth.issuer || result.oauth.authorization_endpoint}</p>}
          {result.suggested_headers.length > 0 && <p>Credential header: {result.suggested_headers.join(', ')} (supply its value below)</p>}
          {result.warnings.map(warning => <p key={warning} className="text-muted-foreground">{warning}</p>)}
        </div>
      )}
    </div>
  )
}
