import { useState } from 'react'
import { FirewallApprovalCard, type ApprovalAction } from '@app/components/shared/FirewallApprovalCard'
import type { SecretFindingSummary } from '@app/types/tauri-commands'

const SAMPLES = ['AKIAZ7Q3VJ2M4XK9PLR6', 'AKIAX9Q7W5E3R1T8Y6U4']
const MARKER = '[REDACTED: secret removed by LocalRouter]'
const INTRO = ['Beginning of the deployment request', ...Array.from({ length: 60 }, (_, i) => `Earlier deployment detail ${i + 1}`)].join('\n')
const OUTRO = [...Array.from({ length: 60 }, (_, i) => `Later deployment detail ${i + 1}`), 'End of the deployment request'].join('\n')
const FINDINGS: SecretFindingSummary[] = SAMPLES.map((_, i) => ({
  rule_id: 'aws-access-token', rule_description: 'AWS Access Key ID', category: 'cloud_provider',
  request_match: null,
  matched_text: i === 0 ? 'AKIA************PLR6' : 'AKIA************Y6U4', entropy: 4.12,
  context: { source: 'Message 2 · user message', line: 63,
    before: `${INTRO}\nHelp me configure the deployment.\naws_access_key_id_${i + 1}=`,
    after: `\nPlease check why authentication fails.\n${OUTRO}`,  },
}))
type Choice = 'redact' | 'allow'
type SavedPolicy = { choice: Choice; hash: string }

/** Development-only review using the real approval component and simulated requests.
 * No requests are sent; saved hashes remain in memory. */
export default function SecretRedactionPreview() {
  const count = new URLSearchParams(window.location.search).has('multiple') ? 2 : 1
  const source = `${INTRO}\n${SAMPLES.slice(0, count).map((secret, i) => `aws_access_key_id_${i + 1}=${secret}`).join('\n')}\n${OUTRO}`
  const requestJson = JSON.stringify({ model: 'gpt-4o', messages: [{ role: 'assistant', content: 'Previous conversation message' }, { role: 'user', content: SAMPLES.reduce((text, secret) => text.split(secret).join(MARKER), source) }], temperature: 0.7 }, null, 2)
  const offsets = Array.from(requestJson.matchAll(/\[REDACTED: secret removed by LocalRouter\]/g), match => match.index!)
  const [current, setCurrent] = useState<number | null>(0)
  const [decisions, setDecisions] = useState<Record<number, Choice>>({})
  const [policies, setPolicies] = useState<Record<number, SavedPolicy>>({})
  const [payload, setPayload] = useState<string | null>(null)
  const [status, setStatus] = useState('Request paused for your decision')
  const [saving, setSaving] = useState(false)

  const finish = (choices: Record<number, Choice>) => {
    setCurrent(null)
    setPayload(SAMPLES.slice(0, count).reduce((text, secret, i) => choices[i] === 'redact' ? text.split(secret).join(MARKER) : text, source))
    setStatus('All secrets reviewed — request continued')
  }
  const advance = (choices: Record<number, Choice>, start: number) => {
    const next = Array.from({ length: count }, (_, i) => i).find(i => i >= start && choices[i] === undefined)
    setDecisions(choices)
    if (next === undefined) finish(choices)
    else { setCurrent(next); setPayload(null); setStatus(`Request paused for secret ${next + 1} of ${count}`) }
  }
  const action = (value: ApprovalAction) => {
    if (current === null) return
    if (value === 'redact_once' || value.startsWith('allow')) {
      advance({ ...decisions, [current]: value === 'redact_once' ? 'redact' : 'allow' }, current + 1)
    } else {
      setCurrent(null)
      setPayload(null)
      setStatus('Request blocked — remaining secrets were not shown')
    }
  }
  const remember = async (choice: Choice) => {
    if (current === null) throw new Error('No pending secret')
    if (new URLSearchParams(window.location.search).has('saveError')) throw new Error('Could not save this secret’s policy')
    setSaving(true)
    try {
      const salt = new TextEncoder().encode('local-preview-client-salt')
      const key = await crypto.subtle.importKey('raw', new TextEncoder().encode(SAMPLES[current]), 'PBKDF2', false, ['deriveBits'])
      const digest = await crypto.subtle.deriveBits({ name: 'PBKDF2', salt, iterations: 600000, hash: 'SHA-256' }, key, 256)
      const hash = Array.from(new Uint8Array(digest), b => b.toString(16).padStart(2, '0')).join('')
      setPolicies(previous => ({ ...previous, [current]: { choice, hash } }))
    } finally { setSaving(false) }
  }
  const nextRequest = () => {
    const choices: Record<number, Choice> = {}
    for (const [index, policy] of Object.entries(policies)) choices[Number(index)] = policy.choice
    advance(choices, 0)
  }

  return <main className="dark min-h-screen bg-background text-foreground p-8">
    <div className="mx-auto max-w-5xl space-y-6">
      <div><p className="text-xs uppercase tracking-widest text-blue-400">Local preview · no requests sent</p><h1 className="text-2xl font-semibold mt-2">Secret redaction</h1><p className="text-sm text-muted-foreground mt-2">Actual approval UI with simulated requests and in-memory hashes.</p><a className="text-sm underline text-muted-foreground" href={count === 1 ? '?multiple' : '?'}>{count === 1 ? 'Try a request with two secrets' : 'Try a request with one secret'}</a></div>
      <div className="grid gap-8 lg:grid-cols-2 items-start">
        <div className="space-y-4">
          {current !== null ? <div className="rounded-lg border shadow-xl w-[440px] max-w-full" style={{ height: new URLSearchParams(window.location.search).has('notify') ? 415 : 480 }} data-testid="approval-popup"><FirewallApprovalCard key={current} className="flex h-full flex-col p-4 overflow-hidden" clientName="Cursor" toolName="gpt-4o" serverName="openai" isSecretScanRequest secretScanLlmSource={new URLSearchParams(window.location.search).get('llmSource') ?? 'OpenAI'} secretScanRequestJson={requestJson} isNotifyOnly={new URLSearchParams(window.location.search).has('notify')} onDismiss={() => setCurrent(null)} secretScanFindings={[{ ...FINDINGS[current], request_match: { start: offsets[current], end: offsets[current] + MARKER.length } }]} onAction={action} onReveal={async () => SAMPLES[current]} onAllowSecretPermanently={() => remember('allow')} onRedactPermanently={() => remember('redact')} submitting={saving} /></div> : <div className="rounded-lg border p-6 text-sm" role="status">{status}</div>}
          <div className="flex gap-3 text-sm"><button className="rounded border px-3 py-2 hover:bg-muted" onClick={nextRequest}>Send the same request again</button><button className="rounded border px-3 py-2 hover:bg-muted" onClick={() => { setPolicies({}); setDecisions({}); setCurrent(0); setPayload(null); setStatus('Request paused for your decision') }}>Reset preview</button></div>
        </div>
        <div className="space-y-5 text-sm">
          <section className="rounded-lg border p-4 space-y-2"><h2 className="font-medium">Outgoing request</h2><pre className="whitespace-pre-wrap break-words text-xs font-mono bg-muted rounded p-3" data-testid="outgoing-payload">{payload ?? 'Not forwarded'}</pre><p className="text-xs text-muted-foreground">{status}</p></section>
          <section className="rounded-lg border p-4 space-y-2"><h2 className="font-medium">Remembered for Cursor</h2>{Object.keys(policies).length === 0 && <p className="text-xs text-muted-foreground">No policy saved</p>}{Object.entries(policies).map(([index, policy]) => <div key={index}><p className="text-xs">{FINDINGS[Number(index)].matched_text} · Always {policy.choice}</p><code className="block break-all text-xs" data-testid="stored-hash">{policy.hash}</code></div>)}<p className="text-xs text-muted-foreground">PBKDF2-HMAC-SHA256 · 600,000 iterations. The application saves a random per-client salt and hash in local settings. Choices can be removed in the client's Secret Scanning settings.</p></section>
        </div>
      </div>
    </div>
  </main>
}
