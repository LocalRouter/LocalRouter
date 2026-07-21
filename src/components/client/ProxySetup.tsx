/**
 * ProxySetup - Connection instructions for a client in a proxy LLM mode.
 *
 * Shown instead of the native "How to Connect" panel when a client uses the
 * HTTPS Inspection Proxy. Surfaces the one-off Claude Code command, the proxy
 * URL, the root CA to trust, and the permanent settings.json fragment.
 */

import { useEffect, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import { listenSafe } from "@/hooks/useTauriListener"
import type { ProxySetupInfo, GetClientProxySetupParams } from "@/types/tauri-commands"

interface ProxySetupProps {
  clientId: string
}

function CopyRow({ label, value }: { label: string; value: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between">
        <span className="text-xs font-medium text-muted-foreground">{label}</span>
        <button
          type="button"
          className="text-xs text-primary hover:underline"
          onClick={() => {
            navigator.clipboard.writeText(value).then(() => {
              setCopied(true)
              setTimeout(() => setCopied(false), 1200)
            })
          }}
        >
          {copied ? "Copied" : "Copy"}
        </button>
      </div>
      <pre className="text-xs bg-muted rounded-md p-2 overflow-x-auto whitespace-pre-wrap break-all">
        {value}
      </pre>
    </div>
  )
}

export function ProxySetup({ clientId }: ProxySetupProps) {
  const [info, setInfo] = useState<ProxySetupInfo | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    const load = () => {
      invoke<ProxySetupInfo>("get_client_proxy_setup", { clientId } satisfies GetClientProxySetupParams)
        .then((data) => {
          if (!cancelled) {
            setInfo(data)
            setError(null)
          }
        })
        .catch((e) => {
          if (!cancelled) setError(String(e))
        })
    }
    load()
    // Refresh if the secret rotates or the proxy state changes.
    const l = listenSafe("clients-changed", load)
    return () => {
      cancelled = true
      l.cleanup()
    }
  }, [clientId])

  if (error) {
    return <p className="text-sm text-destructive">Failed to load proxy setup: {error}</p>
  }
  if (!info) {
    return <p className="text-sm text-muted-foreground">Loading proxy setup…</p>
  }

  return (
    <div className="space-y-5">
      <div className="rounded-lg border border-amber-500/40 bg-amber-500/5 p-3 text-sm">
        <p className="font-medium">HTTPS Inspection Proxy</p>
        <p className="text-muted-foreground mt-1">
          LocalRouter decrypts this client's LLM traffic to show it in the Monitor, then forwards it
          unchanged to the provider — your credentials pass straight through and are never stored.
          Trust the root CA below only on machines you control.
        </p>
      </div>

      {!info.running && (
        <p className="text-sm text-destructive">
          The proxy listener is not running. Check the server settings.
        </p>
      )}

      {info.oneoff_command && (
        <div>
          <p className="text-sm font-medium mb-2">Run Claude Code once through the proxy</p>
          <CopyRow label="Terminal command" value={info.oneoff_command} />
        </div>
      )}

      <div className="grid gap-3">
        {info.proxy_url && <CopyRow label="HTTPS_PROXY" value={info.proxy_url} />}
        <CopyRow label="NODE_EXTRA_CA_CERTS (root CA to trust)" value={info.ca_cert_path} />
      </div>

      {info.settings_json && (
        <div>
          <p className="text-sm font-medium mb-2">
            Permanent setup — add to <code>~/.claude/settings.json</code>
          </p>
          <CopyRow label="settings.json" value={info.settings_json} />
        </div>
      )}
    </div>
  )
}
