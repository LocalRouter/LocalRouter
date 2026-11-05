import { useEffect, useRef, useState } from "react"
import type { DecodedSecretJwt } from "@/types/tauri-commands"

/** Decode the display data independently of revealing the signed token. */
export function JwtContent({ load }: { load: () => Promise<DecodedSecretJwt | null> }) {
  const loader = useRef(load)
  loader.current = load
  const [decoded, setDecoded] = useState<DecodedSecretJwt | null>(null)
  const [loading, setLoading] = useState(true)
  useEffect(() => {
    let active = true
    loader.current().then(value => { if (active) setDecoded(value) }).catch(() => { if (active) setDecoded(null) }).finally(() => { if (active) setLoading(false) })
    return () => { active = false }
  }, [])
  if (loading) return <p role="status" className="text-xs text-muted-foreground">Loading JWT…</p>
  if (!decoded) return <p role="status" className="text-xs text-muted-foreground">Could not decode JWT header and payload.</p>
  return <section aria-label="Decoded JWT" className="space-y-3 text-xs">
    <p className="text-[10px] text-muted-foreground">Decoded locally · Signature not verified</p>
    {(["payload", "header"] as const).map(part => <section key={part} className="space-y-1.5">
      <h4 className="font-semibold capitalize">{part}</h4>
      <pre aria-label={`JWT ${part}`} className="whitespace-pre-wrap break-words rounded-md border bg-background p-3 font-mono text-xs leading-relaxed select-text">{JSON.stringify(decoded[part], null, 2)}</pre>
    </section>)}
  </section>
}
