import { useEffect, useState } from 'react'
import type OpenAI from 'openai'
import { Play, Loader2 } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Textarea } from '@/components/ui/textarea'
import { Label } from '@/components/ui/label'
import { JsonTree } from '@/components/shared/JsonTree'
import type { DecisionRequest, DecisionResponse } from '@/types/decisions'

export function DecisionsPanel({ openaiClient, isReady, selectedModel }: {
  openaiClient: OpenAI | null
  isReady: boolean
  selectedModel: string
}) {
  const [body, setBody] = useState(() => JSON.stringify({
    model: selectedModel,
    input: 'I was charged twice and need a refund.',
    questions: [
      { type: 'choice', name: 'team', instructions: 'Which team should handle this?', choices: [{ value: 'billing', description: 'Payments and refunds' }, { value: 'technical', description: 'Product problems' }] },
      { type: 'predicate', name: 'urgent', instructions: 'Is this urgent?' },
    ],
  } satisfies DecisionRequest, null, 2))
  const [running, setRunning] = useState(false)
  const [result, setResult] = useState<DecisionResponse | null>(null)
  const [backend, setBackend] = useState<string | null>(null)
  const [duration, setDuration] = useState<number | null>(null)
  const [error, setError] = useState<string | null>(null)

  useEffect(() => {
    setBody(previous => {
      try { return JSON.stringify({ ...JSON.parse(previous), model: selectedModel }, null, 2) }
      catch { return previous }
    })
  }, [selectedModel])

  const run = async () => {
    if (!openaiClient || running) return
    setError(null)
    setResult(null)
    let request: DecisionRequest
    try {
      request = JSON.parse(body)
      if (!request || typeof request !== 'object' || typeof request.model !== 'string' || !Array.isArray(request.questions)) throw new Error('Provide a model, input, and questions array.')
    } catch (e) { setError(e instanceof Error ? e.message : 'Invalid request JSON.'); return }
    setRunning(true)
    const started = performance.now()
    try {
      const { data, response } = await openaiClient.post<DecisionResponse>('/decisions', { body: request, maxRetries: 0 }).withResponse()
      setResult(data)
      setBackend(response.headers.get('x-localrouter-decisions-backend'))
      setDuration(Math.round(performance.now() - started))
    } catch (e) { setError(e instanceof Error ? e.message : 'Decision request failed.') }
    finally { setRunning(false) }
  }

  return <div className="space-y-4">
    <p className="text-sm text-muted-foreground">Ask predicate, choice, or score questions. Choice values can be strings or booleans. Image input requires a compatible native Decisions model.</p>
    <div className="space-y-2">
      <Label htmlFor="decisions-request">Request</Label>
      <Textarea id="decisions-request" value={body} onChange={e => setBody(e.target.value)} className="min-h-72 font-mono text-xs" spellCheck={false} disabled={running} />
    </div>
    <Button onClick={run} disabled={!isReady || running}>{running ? <Loader2 className="mr-2 h-4 w-4 animate-spin" /> : <Play className="mr-2 h-4 w-4" />}Run decision</Button>
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    {result && <div className="space-y-2">
      <p className="text-xs text-muted-foreground">{duration} ms{backend && ` · ${backend === 'native' ? 'Native decision API' : `Translated (${backend}); probabilities are not asserted to be calibrated`}`}</p>
      <JsonTree data={result} />
    </div>}
  </div>
}
