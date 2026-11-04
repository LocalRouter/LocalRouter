import { decodeJwt } from "@/utils/jwt"

/** Render only revealed plaintext; decoded claims are as sensitive as the token. */
export function JwtContent({ token }: { token: string }) {
  const decoded = decodeJwt(token)
  if (!decoded) return <p className="text-xs text-muted-foreground" role="status">Could not decode JWT header and payload.</p>

  return (
    <section aria-label="Decoded JWT" className="min-w-0 space-y-2">
      <p className="text-[10px] text-muted-foreground">Decoded JWT · Signature not verified</p>
      {(["header", "payload"] as const).map(part => (
        <div key={part} className="min-w-0 space-y-1">
          <p className="font-semibold capitalize">{part}</p>
          <pre aria-label={`JWT ${part}`} tabIndex={0} className="max-h-40 overflow-auto whitespace-pre-wrap break-words rounded bg-background/70 p-2 text-xs font-mono select-text">
            {JSON.stringify(decoded[part], null, 2)}
          </pre>
        </div>
      ))}
    </section>
  )
}
