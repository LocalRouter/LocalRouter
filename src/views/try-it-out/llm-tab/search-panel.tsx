import { useEffect, useRef, useState } from 'react'
import type OpenAI from 'openai'
import { Search, Square } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/Select'
import { Textarea } from '@/components/ui/textarea'
import { JsonTree } from '@/components/shared/JsonTree'

interface SearchResponse {
  id: string
  model?: string
  results: {
    title: string
    url: string
    snippet?: string
    date?: string
    last_updated?: string
  }[]
  extensions?: Record<string, unknown>
}
export function SearchPanel({
  openaiClient,
  isReady,
}: {
  openaiClient: OpenAI | null
  isReady: boolean
}) {
  const [model, setModel] = useState('')
  const [query, setQuery] = useState('')
  const [searchType, setSearchType] = useState('default')
  const [running, setRunning] = useState(false)
  const [result, setResult] = useState<SearchResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const abort = useRef<AbortController | null>(null)
  useEffect(() => () => abort.current?.abort(), [])
  async function run() {
    if (!openaiClient || !isReady || running || !query.trim()) return
    setRunning(true)
    setError(null)
    setResult(null)
    const controller = new AbortController()
    abort.current = controller
    try {
      const response = await openaiClient.post<SearchResponse>('/search', {
        body: {
          ...(model.trim() ? { model: model.trim() } : {}),
          query: query.trim(),
          ...(searchType === 'default' ? {} : { search_type: searchType }),
          max_results: 5,
          max_tokens_per_page: 1000,
        },
        signal: controller.signal,
        maxRetries: 0,
      })
      if (!controller.signal.aborted) setResult(response)
    } catch (error) {
      setError(
        controller.signal.aborted
          ? 'Search cancelled. A dispatched request may still have incurred a provider fee.'
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
        Ranked page results, separate from model-written answers. Use client
        mode with web search enabled. Provider request fees are reported as
        unknown when absent.
      </p>
      <div className="space-y-2">
        <Label htmlFor="ranked-search-model">
          Search model (optional with a client default)
        </Label>
        <Input
          id="ranked-search-model"
          value={model}
          onChange={(event) => {
            setModel(event.target.value)
            setResult(null)
          }}
          placeholder="perplexity/search or gateway/perplexity/perplexity-search"
          disabled={running}
        />
      </div>
      <div className="space-y-2">
        <Label>Search type</Label>
        <Select
          value={searchType}
          onValueChange={(value) => {
            setSearchType(value)
            setResult(null)
          }}
          disabled={running}
        >
          <SelectTrigger aria-label="Search type">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="default">Model default</SelectItem>
            <SelectItem value="web">Web</SelectItem>
            <SelectItem value="fast">Fast</SelectItem>
            <SelectItem value="people">
              People (direct Perplexity only)
            </SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div className="space-y-2">
        <Label htmlFor="ranked-search-query">Query</Label>
        <Textarea
          id="ranked-search-query"
          value={query}
          onChange={(event) => {
            setQuery(event.target.value)
            setResult(null)
          }}
          maxLength={4096}
          disabled={running}
          placeholder="Find current primary sources…"
        />
      </div>
      <div className="flex gap-2">
        <Button
          disabled={!isReady || running || !query.trim()}
          onClick={() => void run()}
        >
          <Search className="h-4 w-4 mr-2" />
          {running ? 'Searching…' : 'Search'}
        </Button>
        {running && (
          <Button variant="outline" onClick={() => abort.current?.abort()}>
            <Square className="h-4 w-4 mr-2" />
            Cancel
          </Button>
        )}
      </div>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {result && (
        <div className="space-y-3">
          <p className="text-xs text-muted-foreground">
            {result.results.length} ranked results · Request fee unknown
          </p>
          {result.results.map((item, index) => (
            <article
              key={`${index}-${item.url}`}
              className="border rounded-md p-3 space-y-1"
            >
              <a
                href={item.url}
                target="_blank"
                rel="noreferrer noopener"
                className="font-medium text-sm underline break-words"
              >
                {index + 1}. {item.title}
              </a>
              <p className="text-xs text-muted-foreground break-all">
                {item.url}
              </p>
              {item.snippet && (
                <p className="text-sm whitespace-pre-wrap">{item.snippet}</p>
              )}
              {(item.date || item.last_updated) && (
                <p className="text-xs text-muted-foreground">
                  {item.date ? `Published ${item.date}` : ''}
                  {item.last_updated ? ` · Updated ${item.last_updated}` : ''}
                </p>
              )}
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
