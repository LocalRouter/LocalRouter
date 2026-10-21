import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listenSafe } from '@/hooks/useTauriListener'
import { Button } from '@/components/ui/Button'
import { toast } from 'sonner'
import { gatewayBaseUrl, gatewayPortExplanation } from '@/utils/gateway-connection'
import type { ServerConfigInfo, Strategy, GetStrategyParams } from '@/types/tauri-commands'

export function GatewayConnectionSummary({ strategyId }: { strategyId: string }) {
  const [state, setState] = useState<{ config: ServerConfigInfo; status: string; strategy: Strategy } | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let live = true
    let generation = 0
    setState(null)
    const load = async () => {
      const request = ++generation
      try {
        const [config, status, strategy] = await Promise.all([invoke<ServerConfigInfo>('get_server_config'), invoke<string>('get_server_status'), invoke<Strategy>('get_strategy', { strategyId } satisfies GetStrategyParams)])
        if (live && request === generation) { setState({ config, status, strategy }); setError(null) }
      } catch (e) { if (live && request === generation) { setState(null); setError(String(e)) } }
    }
    const listeners = ['server-status-changed', 'config-changed', 'strategies-changed'].map(event => listenSafe(event, load))
    void Promise.all(listeners.map(l => l.promise)).then(() => { if (live) void load() })
    return () => { live = false; listeners.forEach(l => l.cleanup()) }
  }, [strategyId])
  const copy = (value: string) => { void navigator.clipboard.writeText(value).then(() => toast.success('Copied to clipboard')).catch(e => toast.error(String(e))) }
  if (error) return <p role="alert" className="text-sm text-destructive">Connection details unavailable: {error}</p>
  if (!state) return <p className="text-sm text-muted-foreground">Loading connection details…</p>
  const auto = state.strategy.auto_config
  const alias = auto?.model_name || 'auto'
  const running = state.status === 'running' && state.config.actual_port != null
  const explanation = gatewayPortExplanation(state.config)
  return <div className="rounded-lg border p-4 space-y-3 text-sm" aria-label="Gateway connection summary">
    <div className="font-medium">Auto routing connection <span className="text-muted-foreground font-normal">· {running ? 'Running' : 'Server stopped'}</span></div>
    <div className="flex items-center justify-between gap-2"><span>OpenAI base URL: <code>{gatewayBaseUrl(state.config)}/v1</code></span><Button variant="outline" size="sm" disabled={!running} onClick={() => copy(`${gatewayBaseUrl(state.config)}/v1`)}>Copy URL</Button></div>
    {explanation && <p className="text-muted-foreground">{explanation}</p>}
    {auto && auto.permission !== 'off' ? <>
      <div className="flex items-center justify-between gap-2"><span>Model: <code>{alias}</code></span><Button variant="outline" size="sm" onClick={() => copy(alias)}>Copy model</Button></div>
      <p className="text-muted-foreground">Use this model name to run your routing policy. A concrete model name selects that model directly. Use the client API key shown below for authentication.</p>
      <Button variant="outline" size="sm" disabled={!running} onClick={() => copy(JSON.stringify({ model: alias, messages: [{ role: 'user', content: 'Hello' }] }, null, 2))}>Copy request body for POST /v1/chat/completions</Button>
    </> : <p className="text-muted-foreground">Auto routing is disabled. Enable it in the LLM tab or select a permitted concrete model.</p>}
  </div>
}
