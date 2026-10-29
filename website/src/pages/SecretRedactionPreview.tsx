import { useState } from 'react'
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { emit } from '@tauri-apps/api/event'
import { FirewallApproval } from '@app/views/firewall-approval'
import { FirewallApprovalCard, type ApprovalAction } from '@app/components/shared/FirewallApprovalCard'
import type { SecretFindingSummary } from '@app/types/tauri-commands'

const SAMPLES = ['6nmCEa00cbNmH0B4zKPSYOPP5z1oYFR26doJldQr', 'q8Zr2LmX7vKp4NwT9sYb3HdJ6fGc1eRa0uVi5zQ2']
const MARKER = '[REDACTED: secret removed by LocalRouter]'
const INTRO = ['Beginning of the deployment request', ...Array.from({ length: 60 }, (_, i) => `Earlier deployment detail ${i + 1}`)].join('\n')
const OUTRO = [...Array.from({ length: 60 }, (_, i) => `Later deployment detail ${i + 1}`), 'End of the deployment request'].join('\n')
const FINDINGS: SecretFindingSummary[] = SAMPLES.map((_, i) => ({
  rule_id: 'aws-secret-access-key', rule_description: 'AWS Secret Access Key', category: 'cloud_provider',
  request_match: null,
  matched_text: i === 0 ? '6nmCEa********************ldQr' : 'q8Zr2L********************5zQ2', entropy: 4.12,
  context: { source: 'Message 2 · user message', line: 63,
    before: `${INTRO}\nHelp me configure the deployment.\naws_secret_access_key_${i + 1}=`,
    after: `\nPlease check why authentication fails.\n${OUTRO}`,  },
}))
type Choice = 'redact' | 'allow'
type SavedPolicy = { choice: Choice; hash: string }

/** Development-only review using the real approval component and simulated requests.
 * No requests are sent; saved hashes remain in memory. */
export default function SecretRedactionPreview() {
  return new URLSearchParams(window.location.search).has('native')
    ? <NativeApprovalPreview /> : <SharedApprovalPreview />
}

/** Real desktop view with mocked IPC, for lifecycle and race regressions. */
function NativeApprovalPreview() {
  const [closed, setClosed] = useState(false)
  const [requestId, setRequestId] = useState('preview')
  const [, refresh] = useState(0)
  const query = new URLSearchParams(window.location.search)
  const [backend] = useState(() => ({
    pending: !query.has('missing'), nextPending: false,
    submits: 0, attempts: 0, dismisses: 0, closes: 0, forwarded: false,
  }))
  mockWindows(`firewall-approval-${requestId}`)
  mockIPC(async (command, args) => {
    const id = (args as { requestId?: string } | undefined)?.requestId
    if (command === 'get_firewall_approval_details') {
      if (id === 'preview-next' ? !backend.nextPending : !backend.pending) return null
      if (query.has('endDuringLoad')) {
        backend.pending = false
        await emit('firewall-approval-ended')
      }
      return {
        request_id: id, client_id: 'client', client_name: 'Cursor',
        tool_name: 'gpt-4o', server_name: 'Secret Scan', arguments_preview: '',
        timeout_seconds: 86400, created_at_secs_ago: 0,
        is_secret_scan_request: true, is_notify_only: false,
        secret_scan_details: { findings: [FINDINGS[id === 'preview-next' ? 1 : 0]], scan_duration_ms: 1, request_json: null, llm_source: 'OpenAI' },
      }
    }
    if (command === 'plugin:window|is_focused') return true
    if (command === 'submit_firewall_approval') {
      backend.attempts++
      if (query.has('lostOnSubmit')) backend.pending = false
      const pending = id === 'preview-next' ? backend.nextPending : backend.pending
      if (!pending) {
        refresh(value => value + 1)
        const action = (args as { action?: string }).action
        if (action === 'redact_once') throw 'Redaction is only available for a pending blocking secret scan'
        throw `Invalid parameters: Firewall approval request ${id} not found or expired`
      }
      backend.submits++
      if (id === 'preview-next') backend.nextPending = false
      else {
        backend.pending = false
        backend.nextPending = query.has('multiple')
      }
      backend.forwarded = !backend.pending && !backend.nextPending
      refresh(value => value + 1)
      await emit('firewall-approval-ended')
    }
    if (command === 'dismiss_firewall_notification') backend.dismisses++
    if (command === 'plugin:window|close') {
      backend.closes++
      const failures = query.has('closeFailureTwice') ? 2 : query.has('closeFailure') ? 1 : 0
      if (backend.closes <= failures) throw 'Window close failed'
      setClosed(true)
    }
    return null
  })
  return <main className="dark bg-background text-foreground">
    <button onClick={async () => { backend.pending = false; await emit('firewall-approval-ended') }}>End backend approval</button>
    <button onClick={() => { setRequestId('preview-next'); setClosed(false) }}>Review next approval</button>
    <output data-testid="backend-state">Submissions {backend.submits} · attempts {backend.attempts} · second pending {String(backend.nextPending)} · forwarded {String(backend.forwarded)}</output>
    <div data-testid="native-approval" style={{ width: 440, height: 620 }}>
      {closed ? <p>Window closed · submissions {backend.submits} · dismissals {backend.dismisses}</p> : <FirewallApproval key={requestId} />}
    </div>
  </main>
}

function SharedApprovalPreview() {
  const count = new URLSearchParams(window.location.search).has('multiple') ? 2 : 1
  const source = `${INTRO}\n${SAMPLES.slice(0, count).map((secret, i) => `aws_secret_access_key_${i + 1}=${secret}`).join('\n')}\n${OUTRO}`
  const requestJson = JSON.stringify({ model: 'gpt-4o', messages: [{ role: 'assistant', content: 'Previous conversation message' }, { role: 'user', content: SAMPLES.reduce((text, secret) => text.split(secret).join(MARKER), source) }], temperature: 0.7 }, null, 2)
  const offsets = Array.from(requestJson.matchAll(/\[REDACTED: secret removed by LocalRouter\]/g), match => match.index!)
  const [current, setCurrent] = useState<number | null>(0)
  const [decisions, setDecisions] = useState<Record<number, Choice>>({})
  const [policies, setPolicies] = useState<Record<number, SavedPolicy>>({})
  const [payload, setPayload] = useState<string | null>(null)
  const [status, setStatus] = useState('Request paused for your decision')
  const [saving, setSaving] = useState(false)
  const [inactive, setInactive] = useState(false)

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
          {current !== null ? <div className="rounded-lg border shadow-xl w-[440px] max-w-full" style={{ height: new URLSearchParams(window.location.search).has('notify') ? 415 : 480 }} data-testid="approval-popup"><FirewallApprovalCard key={current} className="flex h-full flex-col p-4 overflow-hidden" clientName="Cursor" toolName="gpt-4o" serverName="openai" isSecretScanRequest isInactive={inactive} secretScanLlmSource={new URLSearchParams(window.location.search).get('llmSource') ?? 'OpenAI'} secretScanRequestJson={requestJson} isNotifyOnly={new URLSearchParams(window.location.search).has('notify')} onDismiss={() => setCurrent(null)} secretScanFindings={[{ ...FINDINGS[current], request_match: { start: offsets[current], end: offsets[current] + MARKER.length } }]} onAction={action} onReveal={async () => SAMPLES[current]} onAllowSecretPermanently={() => remember('allow')} onRedactPermanently={() => remember('redact')} submitting={saving} /></div> : <div className="rounded-lg border p-6 text-sm" role="status">{status}</div>}
          <div className="flex gap-3 text-sm"><button className="rounded border px-3 py-2 hover:bg-muted" onClick={() => { setInactive(false); nextRequest() }}>Send the same request again</button><button className="rounded border px-3 py-2 hover:bg-muted" onClick={() => { setInactive(false); setPolicies({}); setDecisions({}); setCurrent(0); setPayload(null); setStatus('Request paused for your decision') }}>Reset preview</button><button className="rounded border px-3 py-2 hover:bg-muted" onClick={() => setInactive(true)}>Simulate ended approval</button></div>
        </div>
        <div className="space-y-5 text-sm">
          <section className="rounded-lg border p-4 space-y-2"><h2 className="font-medium">Outgoing request</h2><pre className="whitespace-pre-wrap break-words text-xs font-mono bg-muted rounded p-3" data-testid="outgoing-payload">{payload ?? 'Not forwarded'}</pre><p className="text-xs text-muted-foreground">{status}</p></section>
          <section className="rounded-lg border p-4 space-y-2"><h2 className="font-medium">Remembered for Cursor</h2>{Object.keys(policies).length === 0 && <p className="text-xs text-muted-foreground">No policy saved</p>}{Object.entries(policies).map(([index, policy]) => <div key={index}><p className="text-xs">{FINDINGS[Number(index)].matched_text} · Always {policy.choice}</p><code className="block break-all text-xs" data-testid="stored-hash">{policy.hash}</code></div>)}<p className="text-xs text-muted-foreground">PBKDF2-HMAC-SHA256 · 600,000 iterations. The application saves a random per-client salt and hash in local settings. Choices can be removed in the client's Secret Scanning settings.</p></section>
        </div>
      </div>
    </div>
  </main>
}
