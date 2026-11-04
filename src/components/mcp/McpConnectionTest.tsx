import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Loader2 } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import type { DiscoverMcpConnectionParams, McpConnectionDiscovery } from '@/types/tauri-commands'

interface Props {
  params: DiscoverMcpConnectionParams
  revision: string
  oauth: boolean
  disabled: boolean
}

/** Remount on edits so a pending response cannot certify different settings. */
export function McpConnectionTest(props: Props) {
  return <ConnectionTest key={JSON.stringify([props.params, props.revision, props.oauth])} {...props} />
}

function ConnectionTest({ params, oauth, disabled }: Props) {
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<McpConnectionDiscovery | null>(null)
  const [error, setError] = useState<string | null>(null)
  const active = useRef(true)
  const pending = useRef(false)
  useEffect(() => {
    active.current = true
    return () => { active.current = false }
  }, [])

  const test = async (event: React.MouseEvent<HTMLButtonElement>) => {
    const form = event.currentTarget.closest('form')
    if (form && !form.reportValidity()) return
    if (pending.current) return
    pending.current = true
    setBusy(true)
    setResult(null)
    setError(null)
    try {
      const discovered = await invoke<McpConnectionDiscovery>('discover_mcp_connection', { ...params } satisfies DiscoverMcpConnectionParams)
      if (active.current) setResult(discovered)
    } catch (e) {
      if (active.current) setError(typeof e === 'string' ? e : 'Connection test failed. Check the settings and try again.')
    } finally {
      pending.current = false
      if (active.current) setBusy(false)
    }
  }

  return (
    <div className="space-y-2 rounded-md border p-3">
      <Button type="button" variant="outline" onClick={test} disabled={busy || disabled || !params.target.trim()}>
        {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
        {busy ? 'Testing connection…' : 'Test connection'}
      </Button>
      <p className="text-xs text-muted-foreground">
        Check the MCP connection before creating this server. {params.transportOverride === 'stdio' ? 'This runs the configured command.' : 'This contacts the configured URL.'}
        {oauth && ' OAuth login and credentials can only be verified after creating the server.'}
      </p>
      {error && <p role="alert" className="text-xs text-destructive">Connection failed: {error}</p>}
      {result && <div role="status" className="space-y-1 text-xs">
        <p className="font-medium">{result.auth_required
          ? oauth ? 'Server reached; OAuth login required after creation.' : 'Authentication required. Check your credentials and test again.'
          : `MCP connection successful${result.server_name ? `: ${result.server_name}` : ''}.`}</p>
        {result.warnings.map(warning => <p key={warning} className="text-muted-foreground">{warning}</p>)}
      </div>}
    </div>
  )
}
