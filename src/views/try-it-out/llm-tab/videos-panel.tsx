import { useEffect, useRef, useState } from 'react'
import type OpenAI from 'openai'
import { Clapperboard, Download, RefreshCw, Square } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/Select'
import { JsonTree } from '@/components/shared/JsonTree'
interface VideoJob {
  id: string
  model: string
  status: string
  seconds: string
  size: string
  extensions: {
    'localrouter.video': {
      phase?: string
      cost_usd: number | null
      cost_source: string
      demo?: boolean
      content_supported?: boolean
    }
  }
}
interface VideoRequest {
  model: string
  prompt: string
  seconds: number
  size: string
}
export function VideosPanel({
  openaiClient,
  isReady,
}: {
  openaiClient: OpenAI | null
  isReady: boolean
}) {
  const [model, setModel] = useState('')
  const [prompt, setPrompt] = useState('')
  const [seconds, setSeconds] = useState(4)
  const [size, setSize] = useState('1280x720')
  const [jobs, setJobs] = useState<VideoJob[]>([])
  const [pending, setPending] = useState<{
    key: string
    body: VideoRequest
  } | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const abort = useRef<AbortController | null>(null)
  useEffect(() => () => abort.current?.abort(), [])
  function upsert(job: VideoJob) {
    setJobs((current) => [job, ...current.filter((row) => row.id !== job.id)])
  }
  async function action(
    run: (client: OpenAI, signal: AbortSignal) => Promise<void>,
  ) {
    if (!openaiClient || !isReady || busy) return
    setBusy(true)
    setError(null)
    const controller = new AbortController()
    abort.current = controller
    try {
      await run(openaiClient, controller.signal)
    } catch (error) {
      setError(
        controller.signal.aborted
          ? 'Stopped waiting locally. Rendering and billing may continue. Retry the same request key or load recorded jobs.'
          : String(error),
      )
    } finally {
      setBusy(false)
      abort.current = null
    }
  }
  function create(retry: boolean) {
    const request = retry
      ? pending
      : {
          key: crypto.randomUUID(),
          body: { model: model.trim(), prompt, seconds, size },
        }
    if (!request) return
    setPending(request)
    void action(async (client, signal) => {
      const job = await client.post<VideoJob>('/videos', {
        body: request.body,
        headers: { 'Idempotency-Key': request.key },
        signal,
        maxRetries: 0,
      })
      if (!signal.aborted) {
        upsert(job)
        setPending(null)
      }
    })
  }
  return (
    <div className="space-y-4 overflow-y-auto h-full pr-2">
      <p className="text-sm text-muted-foreground">
        Create an asynchronous video with an explicit provider/model. Enable
        Allow video generation in client Settings first. Provider fees may
        apply; native cancellation is unavailable for these mappings.
      </p>
      <div className="space-y-2">
        <Label htmlFor="video-model">Video model</Label>
        <Input
          id="video-model"
          value={model}
          disabled={busy}
          onChange={(e) => setModel(e.target.value)}
          placeholder="openai/sora-2 or gateway/google-vertex/veo-3.1-generate-preview"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="video-prompt">Video prompt</Label>
        <Textarea
          id="video-prompt"
          value={prompt}
          disabled={busy}
          maxLength={16384}
          onChange={(e) => setPrompt(e.target.value)}
        />
      </div>
      <div className="grid grid-cols-2 gap-3">
        <div className="space-y-2">
          <Label htmlFor="video-seconds">Seconds</Label>
          <Input
            id="video-seconds"
            type="number"
            min={4}
            max={30}
            value={seconds}
            disabled={busy}
            onChange={(e) => setSeconds(Number(e.target.value))}
          />
        </div>
        <div className="space-y-2">
          <Label>Size</Label>
          <Select value={size} onValueChange={setSize} disabled={busy}>
            <SelectTrigger aria-label="Video size">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="1280x720">1280 × 720</SelectItem>
              <SelectItem value="720x1280">720 × 1280</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>
      <p className="text-xs text-muted-foreground">
        Direct Sora accepts 4, 8 or 12 seconds here. Gateway durations depend on
        the selected model. Only text prompts are supported; no remote
        references or callbacks are fetched.
      </p>
      <div className="flex gap-2 flex-wrap">
        <Button
          disabled={
            !isReady ||
            busy ||
            !!pending ||
            !model.trim() ||
            !prompt.trim() ||
            !Number.isInteger(seconds) ||
            seconds < 4 ||
            seconds > 30
          }
          onClick={() => create(false)}
        >
          <Clapperboard className="h-4 w-4 mr-2" />
          Generate video
        </Button>
        <Button
          variant="outline"
          disabled={!isReady || busy}
          onClick={() =>
            void action(async (client, signal) => {
              const result = await client.get<{ data: VideoJob[] }>(
                '/videos?limit=20',
                { signal, maxRetries: 0 },
              )
              if (!signal.aborted) setJobs(result.data)
            })
          }
        >
          Load recorded jobs
        </Button>
        {busy && (
          <Button variant="outline" onClick={() => abort.current?.abort()}>
            <Square className="h-4 w-4 mr-2" />
            Stop waiting
          </Button>
        )}
      </div>
      {pending && !busy && (
        <div className="space-y-2 text-sm">
          <p>
            The previous request may have been dispatched. Reusing its key never
            repeats a recorded upstream create.
          </p>
          <div className="flex gap-2">
            <Button variant="outline" onClick={() => create(true)}>
              Retry same request
            </Button>
            <Button variant="outline" onClick={() => setPending(null)}>
              Start a different request
            </Button>
          </div>
        </div>
      )}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {jobs.map((job) => (
        <article key={job.id} className="border rounded-md p-3 space-y-2">
          <p className="font-medium text-sm break-all">{job.id}</p>
          <p className="text-sm">
            {job.status} · {job.seconds}s · {job.size}
          </p>
          <p className="text-xs text-muted-foreground">
            {job.extensions['localrouter.video'].cost_usd === null
              ? 'Job fee unknown'
              : job.extensions['localrouter.video'].cost_source ===
                  'not_dispatched'
                ? 'Not dispatched; no video generation charge'
                : `Reported job fee: $${job.extensions['localrouter.video'].cost_usd}`}
          </p>
          {job.extensions['localrouter.video'].demo && (
            <p className="text-sm text-muted-foreground">
              Demo job metadata. No video is rendered or billed.
            </p>
          )}
          <div className="flex gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={busy}
              onClick={() =>
                void action(async (client, signal) => {
                  const result = await client.get<VideoJob>(
                    `/videos/${encodeURIComponent(job.id)}`,
                    { signal, maxRetries: 0 },
                  )
                  if (!signal.aborted) upsert(result)
                })
              }
            >
              <RefreshCw className="h-3 w-3 mr-1" />
              Refresh status
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={
                busy || !job.extensions['localrouter.video'].content_supported
              }
              onClick={() =>
                void action(async (client, signal) => {
                  const response = await client
                    .get(`/videos/${encodeURIComponent(job.id)}/content`, {
                      signal,
                      maxRetries: 0,
                    })
                    .asResponse()
                  const blob = await response.blob()
                  if (signal.aborted) return
                  const url = URL.createObjectURL(blob)
                  try {
                    const link = document.createElement('a')
                    link.href = url
                    link.download = `${job.id}.mp4`
                    link.click()
                  } finally {
                    setTimeout(() => URL.revokeObjectURL(url), 1000)
                  }
                })
              }
            >
              <Download className="h-3 w-3 mr-1" />
              Download MP4
            </Button>
          </div>
          <details>
            <summary className="text-sm cursor-pointer">Job JSON</summary>
            <JsonTree data={job} />
          </details>
        </article>
      ))}
    </div>
  )
}
