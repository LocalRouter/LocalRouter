import { useEffect, useRef, useState } from 'react'
import type OpenAI from 'openai'
import { ListOrdered, Square } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { JsonTree } from '@/components/shared/JsonTree'

interface RerankResponse {
  results: {
    index: number
    relevance_score: number
    document?: { text: string }
  }[]
  meta?: unknown
  extensions?: Record<string, unknown>
}
export function RerankPanel({
  openaiClient,
  isReady,
}: {
  openaiClient: OpenAI | null
  isReady: boolean
}) {
  const [model, setModel] = useState('')
  const [query, setQuery] = useState('')
  const [documents, setDocuments] = useState(['', ''])
  const [running, setRunning] = useState(false)
  const [result, setResult] = useState<RerankResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const abort = useRef<AbortController | null>(null)
  useEffect(() => () => abort.current?.abort(), [])
  async function run() {
    if (
      !openaiClient ||
      !isReady ||
      !query.trim() ||
      !model.trim() ||
      documents.some((value) => !value.trim()) ||
      running
    )
      return
    setRunning(true)
    setError(null)
    setResult(null)
    const controller = new AbortController()
    abort.current = controller
    try {
      const response = await openaiClient.post<RerankResponse>('/rerank', {
        body: {
          model: model.trim(),
          query,
          documents,
          top_n: documents.length,
          return_documents: true,
        },
        signal: controller.signal,
        maxRetries: 0,
      })
      if (!controller.signal.aborted) setResult(response)
    } catch (error) {
      setError(
        controller.signal.aborted
          ? 'Rerank cancelled. The provider may already have processed and charged for the candidates.'
          : String(error),
      )
    } finally {
      setRunning(false)
      abort.current = null
    }
  }
  return (
    <div className="space-y-4 overflow-y-auto h-full pr-2">
      <p className="text-sm text-muted-foreground">
        Score supplied candidate documents against a query. Enable Allow rerank
        in client Settings first. No documents are fetched implicitly; provider
        fees may apply.
      </p>
      <div className="space-y-2">
        <Label htmlFor="rerank-model">Rerank model</Label>
        <Input
          id="rerank-model"
          value={model}
          disabled={running}
          onChange={(e) => {
            setModel(e.target.value)
            setResult(null)
          }}
          placeholder="cohere/rerank-v4.0-pro or gateway/qwen3-reranker-8b"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="rerank-query">Ranking query</Label>
        <Textarea
          id="rerank-query"
          value={query}
          disabled={running}
          maxLength={16384}
          onChange={(e) => {
            setQuery(e.target.value)
            setResult(null)
          }}
        />
      </div>
      {documents.map((document, index) => (
        <div key={index} className="space-y-2">
          <Label htmlFor={`rerank-candidate-${index}`}>
            Candidate {index} (original index)
          </Label>
          <Textarea
            id={`rerank-candidate-${index}`}
            value={document}
            disabled={running}
            maxLength={32768}
            onChange={(e) => {
              setDocuments((current) =>
                current.map((text, position) =>
                  position === index ? e.target.value : text,
                ),
              )
              setResult(null)
            }}
          />
        </div>
      ))}
      <div className="flex gap-2 flex-wrap">
        <Button
          variant="outline"
          disabled={running || documents.length >= 16}
          onClick={() => {
            setDocuments((current) => [...current, ''])
            setResult(null)
          }}
        >
          Add candidate
        </Button>
        <Button
          disabled={
            !isReady ||
            running ||
            !query.trim() ||
            !model.trim() ||
            documents.some((value) => !value.trim())
          }
          onClick={() => void run()}
        >
          <ListOrdered className="h-4 w-4 mr-2" />
          {running ? 'Ranking…' : 'Rerank candidates'}
        </Button>
        {running && (
          <Button variant="outline" onClick={() => abort.current?.abort()}>
            <Square className="h-4 w-4 mr-2" />
            Cancel
          </Button>
        )}
      </div>
      <p className="text-xs text-muted-foreground">
        Scores are model-specific, not probabilities calibrated across queries.
        Providers may truncate long candidates. Returned source text maps to the
        exact original index.
      </p>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {result && (
        <div className="space-y-3">
          {(result.extensions?.['localrouter.rerank'] as { demo?: boolean } | undefined)?.demo && <p className="text-sm text-muted-foreground">Demo ranking only. No model scored these documents.</p>}
          <p className="text-xs text-muted-foreground">Provider fee unknown</p>
          {result.results.map((entry, position) => (
            <article
              key={entry.index}
              className="border rounded-md p-3 space-y-2"
            >
              <p className="text-sm font-medium">
                Rank {position + 1} · Candidate {entry.index} · Score{' '}
                {entry.relevance_score.toPrecision(4)}
              </p>
              <p className="text-sm whitespace-pre-wrap">
                {entry.document?.text}
              </p>
            </article>
          ))}
          <details>
            <summary className="text-sm cursor-pointer">Response JSON</summary>
            <JsonTree data={result} />
          </details>
        </div>
      )}
    </div>
  )
}
