import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/Card'
import { Button } from '@/components/ui/Button'
import { Switch } from '@/components/ui/switch'
import type {
  ResponseCacheConfig,
  ResponseCacheStats,
  ResponseCacheClientParams,
} from '@/types/tauri-commands'
export const DEFAULT_RESPONSE_CACHE: ResponseCacheConfig = {
  enabled: false,
  ttl_secs: 300,
  max_entries: 128,
  max_bytes: 8 * 1024 * 1024,
}
export function ResponseCacheEditor({
  clientId,
  value,
  readOnly,
  onSave,
}: {
  clientId: string
  value?: ResponseCacheConfig
  readOnly?: boolean
  onSave: (value: ResponseCacheConfig) => Promise<void>
}) {
  const [draft, setDraft] = useState(value ?? DEFAULT_RESPONSE_CACHE)
  const [stats, setStats] = useState<ResponseCacheStats | null>(null)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  useEffect(() => setDraft(value ?? DEFAULT_RESPONSE_CACHE), [value])
  const refresh = async () =>
    setStats(
      await invoke<ResponseCacheStats>('get_response_cache_stats', {
        clientId,
      } satisfies ResponseCacheClientParams),
    )
  const action = async (fn: () => Promise<void>) => {
    setBusy(true)
    setError('')
    try {
      await fn()
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle>Exact response cache</CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">
          Reuse exact successful text responses for this client. Cached payloads
          stay in this process’s memory and expire automatically. Current
          permissions and checks still apply. Replay, streaming, tools, sessions
          and provider-bound state bypass the cache.
        </p>
        <label className="flex gap-2 items-center">
          <Switch
            checked={draft.enabled}
            disabled={readOnly || busy}
            onCheckedChange={(enabled) => setDraft((d) => ({ ...d, enabled }))}
          />
          Enable exact caching (off by default)
        </label>
        <div className="grid grid-cols-3 gap-3">
          {(
            [
              { key: 'ttl_secs', label: 'TTL (seconds)', min: 1, max: 3600 },
              { key: 'max_entries', label: 'Entry limit', min: 1, max: 1024 },
              {
                key: 'max_bytes',
                label: 'Payload byte limit',
                min: 1024,
                max: 32 * 1024 * 1024,
              },
            ] as const
          ).map((field) => (
            <label key={field.key} className="text-xs">
              {field.label}
              <input
                type="number"
                className="block border rounded p-2 bg-background w-full"
                value={draft[field.key]}
                min={field.min}
                max={field.max}
                disabled={readOnly || busy}
                onChange={(e) =>
                  setDraft((d) => ({
                    ...d,
                    [field.key]: Number(e.target.value),
                  }))
                }
              />
            </label>
          ))}
        </div>
        <div className="flex gap-2">
          <Button
            disabled={readOnly || busy}
            onClick={() =>
              void action(async () => {
                await onSave(draft)
                await refresh()
              })
            }
          >
            Save cache settings
          </Button>
          <Button
            variant="outline"
            disabled={busy}
            onClick={() => void action(refresh)}
          >
            Refresh cache statistics
          </Button>
          <Button
            variant="outline"
            disabled={busy}
            onClick={() =>
              void action(async () => {
                await invoke<void>('clear_response_cache', {
                  clientId,
                } satisfies ResponseCacheClientParams)
                await refresh()
              })
            }
          >
            Clear this client’s cached payloads
          </Button>
        </div>
        {error && (
          <p role="alert" className="text-destructive text-sm">
            {error}
          </p>
        )}
        {stats && (
          <div className="text-xs space-y-1">
            <p>
              {stats.entries} retained entries · {stats.bytes.toLocaleString()}{' '}
              serialized payload bytes · {stats.hits} hits · {stats.misses}{' '}
              misses · {stats.bypasses} bypasses · {stats.evictions} evictions
            </p>
            <p>
              {stats.upstream_requests_avoided} upstream inference requests
              avoided. Avoided cost estimate:{' '}
              {stats.avoided_cost_estimate_usd === null
                ? 'unknown'
                : `$${stats.avoided_cost_estimate_usd.toFixed(6)}`}{' '}
              (partial; based on previous model calls). Reusing a model response
              avoids a new call to that model. Current routing classifiers and
              safety checks still run and can have separate spend. Served tokens
              remain visible separately from billed tokens.
            </p>
          </div>
        )}
        <p className="text-xs text-muted-foreground">
          API controls: extensions.localrouter.response_cache.mode = use,
          bypass, or refresh. Requests cannot enable caching against disabled
          client settings. There is a shared 64 MiB process limit and a 1 MiB
          limit per cached response.
        </p>
      </CardContent>
    </Card>
  )
}
