import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { AlertTriangle, ArrowRight, CheckCircle2, Loader2 } from 'lucide-react'
import { Badge } from '@/components/ui/Badge'
import { Button } from '@/components/ui/Button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/Card'
import type { SecretScanResult, TestSecretScanParams } from '@/types/tauri-commands'

const SAMPLE = [
  'AWS_SECRET_ACCESS_KEY=6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr',
  "password = 'super_s3cret_P@ssw0rd!'",
  'DATABASE_URL=postgres://app:Hq3vT8mZ2xLp9wKr@db:5432/main',
  'Access key ID (not a secret): AKIAZ7Q3VJ2M4XK9PLR6',
].join('\n')
const field = 'w-full rounded-md border bg-background p-3 font-mono text-xs leading-5 resize-y focus:outline-none focus:ring-1 focus:ring-ring'
function Output({ value, label }: { value: string; label: string }) {
  return <pre aria-label={label} tabIndex={0} className="min-h-28 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md border bg-muted/30 p-3 font-mono text-xs leading-5 select-text">{value}</pre>
}

export function SecretScanPlayground() {
  const [input, setInput] = useState(SAMPLE)
  const [template, setTemplate] = useState('{{redacted}}')
  const [encoding, setEncoding] = useState<NonNullable<TestSecretScanParams['responseEncoding']>>('plain')
  const [result, setResult] = useState<SecretScanResult | null>(null)
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const revision = useRef(0)
  useEffect(() => {
    const current = ++revision.current
    setResult(null)
    setError(null)
    if (!input.trim()) { setLoading(false); return }
    setLoading(true)
    const timer = setTimeout(async () => {
      try {
        const preview = await invoke<SecretScanResult>('test_secret_scan', {
          input, responseTemplate: template, responseEncoding: encoding,
        } satisfies TestSecretScanParams)
        if (revision.current === current) setResult(preview)
      } catch (err) {
        if (revision.current === current) setError(String(err))
      } finally {
        if (revision.current === current) setLoading(false)
      }
    }, 350)
    return () => { clearTimeout(timer); revision.current++ }
  }, [input, template, encoding])

  return <div className="space-y-4">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div className="space-y-1">
        <h2 className="font-semibold">See the full round trip</h2>
        <p className="text-sm text-muted-foreground">Simulate Redact locally with the same scanner and restoration engine used by live requests.</p>
        <p className="text-xs text-muted-foreground">Temporary memory only. Nothing is saved to the secret store or sent to a provider. This preview does not change your live policy.</p>
      </div>
      <Button variant="outline" size="sm" onClick={() => { setInput(SAMPLE); setTemplate('{{redacted}}'); setEncoding('plain') }}>Reset example</Button>
    </div>
    <div className="flex items-center gap-2 text-xs text-muted-foreground" aria-hidden="true">
      <span>Your input</span><ArrowRight className="h-3 w-3" /><span>Secret Redacted</span><ArrowRight className="h-3 w-3" /><span>Secret Restored</span>
    </div>
    <div className="grid grid-cols-1 xl:grid-cols-2 gap-4">
      <Card>
        <CardHeader className="pb-3"><CardTitle className="text-sm">1. Your input</CardTitle><CardDescription>Enter synthetic credentials or text to scan.</CardDescription></CardHeader>
        <CardContent><textarea aria-label="Secret scan input" value={input} onChange={e => setInput(e.target.value)} rows={7} spellCheck={false} className={field} /></CardContent>
      </Card>
      <Card>
        <CardHeader className="pb-3"><CardTitle className="text-sm">2. Secret Redacted</CardTitle><CardDescription>This is the protected content a provider would receive.</CardDescription></CardHeader>
        <CardContent className="space-y-2">
          <Output label="Redacted outbound content" value={result?.redacted_request ?? (loading ? 'Scanning…' : 'Enter text to begin.')} />
          {result && <div className="flex flex-wrap items-center gap-2 text-xs"><Badge variant={result.findings.length ? 'outline' : 'secondary'}>{result.findings.length} secrets detected</Badge><span className="text-muted-foreground">{result.rules_evaluated} rules · {result.scan_duration_ms}ms</span></div>}
        </CardContent>
      </Card>
      <Card>
        <CardHeader className="pb-3"><CardTitle className="text-sm">3. Simulated model response</CardTitle><CardDescription>A model can return a reference in text, JSON, or escaped markup.</CardDescription></CardHeader>
        <CardContent className="space-y-3">
          <label className="block space-y-1 text-xs"><span>Reference encoding</span><select aria-label="Reference encoding" value={encoding} onChange={e => setEncoding(e.target.value as typeof encoding)} className="w-full rounded-md border bg-background px-2 py-2"><option value="plain">Plain text</option><option value="json">JSON string</option><option value="nested_json">JSON inside JSON</option><option value="html">HTML entities, including dashes</option></select></label>
          <label className="block space-y-1 text-xs"><span>Response template</span><textarea aria-label="Simulated response template" value={template} onChange={e => setTemplate(e.target.value)} rows={2} spellCheck={false} className={field} /></label>
          <p className="text-xs text-muted-foreground">{'{{redacted}}'} inserts the current protected content. Add an unknown reference to see the non-blocking diagnostic.</p>
          <Output label="Simulated model response" value={result?.model_response ?? 'Waiting for input…'} />
        </CardContent>
      </Card>
      <Card>
        <CardHeader className="pb-3"><CardTitle className="text-sm">4. Secret Restored</CardTitle><CardDescription>This is what the local client receives, with the matching escaping reapplied.</CardDescription></CardHeader>
        <CardContent className="space-y-3">
          <Output label="Restored local response" value={result?.restored_response ?? 'Waiting for the round trip…'} />
          {result && <div className="flex items-center gap-2 text-xs">{result.round_trip_matches && !result.warnings.length ? <CheckCircle2 className="h-4 w-4 text-emerald-600" /> : <AlertTriangle className="h-4 w-4 text-amber-600" />}<span>{result.warnings.length ? 'Response delivered with diagnostics' : result.round_trip_matches ? result.restored_references ? `${result.restored_references} references restored · round trip verified` : result.findings.length ? 'Response preserved; no matching references returned' : 'No secrets detected; content is unchanged' : 'Some content could not be restored; the response is preserved'}</span></div>}
          {result?.warnings.map(warning => <p key={warning} role="status" className="rounded-md border border-amber-500/30 bg-amber-500/5 p-2 text-xs"><strong>Secret Not Restored</strong><br />{warning}</p>)}
        </CardContent>
      </Card>
    </div>
    {loading && <p className="flex items-center gap-2 text-xs text-muted-foreground" role="status"><Loader2 className="h-3 w-3 animate-spin" />Updating local preview…</p>}
    {error && <p role="alert" className="rounded-md border border-destructive/30 p-3 text-sm text-destructive">Preview failed: {error}</p>}
    {result && result.findings.length > 0 && <details className="rounded-md border p-3 text-sm"><summary className="cursor-pointer font-medium">Detection details ({result.findings.length})</summary><div className="mt-3 space-y-2">{result.findings.map((finding, i) => <div key={`${finding.rule_id}:${i}`} className="rounded bg-muted/30 p-2 text-xs"><div className="flex justify-between gap-2"><strong>{finding.rule_description}</strong><span>{finding.category}</span></div><code className="mt-1 block break-all">{finding.matched_text}</code><span className="text-muted-foreground">Entropy {finding.entropy.toFixed(2)}</span></div>)}</div></details>}
  </div>
}
