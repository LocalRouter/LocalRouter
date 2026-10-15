import { useEffect, useRef, useState } from 'react'
import type OpenAI from 'openai'
import { Download, RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Label } from '@/components/ui/label'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'

type Usage = {
  requests: number
  input_tokens: number
  output_tokens: number
  cost_usd: number | null
}
type Job = {
  id: string
  status: string
  output_file_id: string | null
  error_file_id: string | null
  request_counts: { total: number; completed: number; failed: number }
  localrouter: {
    phase: string
    provider_instance: string
    model: string
    worker_running: boolean
    validated_rows: number
    request_counts_known: boolean
    can_resume_validation: boolean
    can_submit_prepared: boolean
    cancellation_supported: boolean
    reserved_usage: Usage | null
    accounting_settled: boolean
    cleanup: Record<string, string>
    last_error: { message: string } | null
    accounting: {
      billing: {
        source: string
        actual_input_tokens: number | null
        actual_output_tokens: number | null
        reported_charge_usd: number | null
        upstream_inference_estimate_usd: number | null
      }
    } | null
  }
}
type Page = {
  data: Job[]
  has_more: boolean
  last_id: string | null
  localrouter_usage: {
    jobs: number
    jobs_with_unknown_cost: number
    known_reported_charge_usd: number | null
    known_inference_estimate_usd: number | null
  }
}
const money = (value: number | null | undefined) =>
  value == null ? 'unknown' : `$${value.toFixed(6)}`

export function BatchesPanel({
  openaiClient,
  isReady,
}: {
  openaiClient: OpenAI | null
  isReady: boolean
}) {
  const [input, setInput] = useState('')
  const [endpoint, setEndpoint] = useState('/v1/chat/completions')
  const [page, setPage] = useState<Page | null>(null)
  const [jobs, setJobs] = useState<Job[]>([])
  const [busy, setBusy] = useState(false)
  const [cleanup, setCleanup] = useState<{ job: Job; remote: boolean } | null>(
    null,
  )
  const [watch, setWatch] = useState<{ id: string; until: number } | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [submit, setSubmit] = useState<Job | null>(null)
  const [cancel, setCancel] = useState<Job | null>(null)
  const mounted = useRef(true)
  const controller = useRef<AbortController | null>(null)
  const createIntent = useRef<{ body: string; key: string } | null>(null)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      controller.current?.abort()
    }
  }, [])
  useEffect(() => {
    if (!watch || !openaiClient || !isReady) return
    const timer = setInterval(() => {
      if (Date.now() >= watch.until) {
        setWatch(null)
        return
      }
      if (busy) return
      const job = jobs.find((job) => job.id === watch.id)
      if (
        !job ||
        ['completed', 'failed', 'expired', 'cancelled'].includes(job.status)
      ) {
        setWatch(null)
        return
      }
      void act((signal) => control(job, 'refresh', signal))
    }, 5000)
    return () => clearInterval(timer)
  }, [watch, busy, jobs, openaiClient, isReady])
  async function act(action: (signal: AbortSignal) => Promise<void>) {
    if (!openaiClient || busy || !isReady) return
    const abort = new AbortController()
    controller.current = abort
    setBusy(true)
    setError(null)
    try {
      await action(abort.signal)
    } catch (e) {
      if (mounted.current)
        setError(e instanceof Error ? e.message : 'Batch operation failed.')
    } finally {
      if (mounted.current) setBusy(false)
    }
  }
  async function load(signal: AbortSignal, after?: string) {
    if (!openaiClient) return
    const result = await openaiClient.get<Page>('/batches', {
      query: { limit: 20, ...(after ? { after } : {}) },
      signal,
      maxRetries: 0,
    })
    if (mounted.current) {
      setPage(result)
      setJobs((previous) =>
        after ? [...previous, ...result.data] : result.data,
      )
    }
  }
  async function control(job: Job, action: string, signal: AbortSignal) {
    if (!openaiClient) return
    await openaiClient.post<Job>(
      `/batches/${encodeURIComponent(job.id)}/${action}`,
      { signal, maxRetries: 0 },
    )
    await load(signal)
  }
  async function download(id: string, signal: AbortSignal) {
    if (!openaiClient) return
    const response = await openaiClient.files.content(id, {
      signal,
      maxRetries: 0,
    })
    const blob = await response.blob()
    if (!mounted.current) return
    const url = URL.createObjectURL(blob)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = `${id}.jsonl`
    anchor.click()
    setTimeout(() => URL.revokeObjectURL(url), 1000)
  }
  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Upload JSONL in Files, then prepare it here. Every row needs one
        explicit provider-instance/model and an output token cap. Preparation
        checks permissions, approvals and safety before any provider upload.
        Direct OpenAI and OpenRouter native batches are supported for text Chat,
        Responses and embeddings.
      </p>
      <div className="flex flex-wrap items-end gap-2">
        <div className="min-w-64 flex-1 space-y-2">
          <Label htmlFor="batch-input-id">Local input file ID</Label>
          <Input
            id="batch-input-id"
            value={input}
            disabled={busy}
            onChange={(e) => setInput(e.target.value)}
            placeholder="file-lr-…"
          />
        </div>
        <div className="space-y-2">
          <Label htmlFor="batch-endpoint">Endpoint</Label>
          <select
            id="batch-endpoint"
            className="h-9 rounded-md border bg-background px-2 text-sm"
            value={endpoint}
            disabled={busy}
            onChange={(e) => setEndpoint(e.target.value)}
          >
            <option value="/v1/chat/completions">Chat Completions</option>
            <option value="/v1/responses">Responses</option>
            <option value="/v1/embeddings">Embeddings</option>
          </select>
        </div>
        <Button
          disabled={busy || !isReady || !input.trim()}
          onClick={() =>
            void act(async (signal) => {
              if (!openaiClient) return
              const body = {
                input_file_id: input.trim(),
                endpoint,
                completion_window: '24h',
                extensions: { 'localrouter.batch': { prepare_only: true } },
              }
              const serialized = JSON.stringify(body)
              if (createIntent.current?.body !== serialized)
                createIntent.current = {
                  body: serialized,
                  key: crypto.randomUUID(),
                }
              await openaiClient.post<Job>('/batches', {
                body,
                headers: { 'Idempotency-Key': createIntent.current.key },
                signal,
                maxRetries: 0,
              })
              // Keep the key for transport retries; changing the form creates a new intent.
              await load(signal)
            })
          }
        >
          Prepare batch
        </Button>
        <Button
          variant="outline"
          disabled={busy}
          onClick={() => {
            createIntent.current = null
            setInput('')
            setError(null)
          }}
        >
          New batch
        </Button>
        <Button
          variant="outline"
          disabled={busy || !isReady}
          onClick={() => void act((signal) => load(signal))}
        >
          <RefreshCw className="mr-2 h-4 w-4" />
          Read local status
        </Button>
      </div>
      <p className="text-xs text-muted-foreground">
        Local status never polls a provider. Use “Refresh from provider”
        explicitly to retrieve results. Preparation can wait for approval;
        stopping or restarting the app never automatically submits a batch.
      </p>
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {page && (
        <p className="text-xs text-muted-foreground">
          {page.localrouter_usage.jobs} jobs · reported charges{' '}
          {money(page.localrouter_usage.known_reported_charge_usd)} · inference
          estimates {money(page.localrouter_usage.known_inference_estimate_usd)}{' '}
          · {page.localrouter_usage.jobs_with_unknown_cost} jobs with unknown
          cost. Batch usage is asynchronous and separate from synchronous
          request latency.
        </p>
      )}
      {!jobs.length && (
        <p className="text-sm text-muted-foreground">
          Read local status to list this client’s batches.
        </p>
      )}
      {jobs.map((job) => {
        const local = job.localrouter
        const bill = local.accounting?.billing
        const terminal = [
          'completed',
          'failed',
          'expired',
          'cancelled',
        ].includes(job.status)
        return (
          <div key={job.id} className="space-y-3 rounded-md border p-3">
            <div>
              <p className="text-sm font-medium">
                {local.provider_instance}/{local.model} · {job.status}
              </p>
              <p className="break-all text-xs text-muted-foreground">
                {job.id} · {local.phase}
                {local.worker_running ? ' · worker running' : ''}
              </p>
            </div>
            <p className="text-xs text-muted-foreground">
              Validated {local.validated_rows}/{job.request_counts.total} ·
              completed{' '}
              {local.request_counts_known
                ? job.request_counts.completed
                : 'unknown'}{' '}
              · failed{' '}
              {local.request_counts_known
                ? job.request_counts.failed
                : 'unknown'}{' '}
              · reserved estimate {money(local.reserved_usage?.cost_usd)}
            </p>
            {bill && (
              <p className="text-xs text-muted-foreground">
                Input tokens {bill.actual_input_tokens ?? 'unknown'} · output
                tokens {bill.actual_output_tokens ?? 'unknown'} · reported
                charge {money(bill.reported_charge_usd)} · inference estimate{' '}
                {money(bill.upstream_inference_estimate_usd)} · {bill.source}
              </p>
            )}
            {Object.keys(local.cleanup).length > 0 && (
              <p className="text-xs text-muted-foreground">
                Cleanup:{' '}
                {Object.entries(local.cleanup)
                  .map(([kind, outcome]) => `${kind}: ${outcome}`)
                  .join(' · ')}
              </p>
            )}
            {local.last_error && (
              <p className="text-xs text-destructive">
                {local.last_error.message}
              </p>
            )}
            <div className="flex flex-wrap gap-2">
              {local.can_resume_validation && (
                <Button
                  size="sm"
                  variant="outline"
                  disabled={busy}
                  onClick={() =>
                    void act((signal) => control(job, 'resume', signal))
                  }
                >
                  Resume validation
                </Button>
              )}
              {local.can_submit_prepared && (
                <Button
                  size="sm"
                  disabled={busy}
                  onClick={() => setSubmit(job)}
                >
                  Submit to provider
                </Button>
              )}
              {['submitted', 'cancel_pending_outcome', 'terminal'].includes(
                local.phase,
              ) && (
                <Button
                  size="sm"
                  variant="outline"
                  disabled={busy || local.worker_running}
                  onClick={() =>
                    void act((signal) => control(job, 'refresh', signal))
                  }
                >
                  Refresh from provider
                </Button>
              )}
              {local.phase.includes('pending_outcome') &&
                !local.worker_running && (
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={busy}
                    onClick={() =>
                      void act(async (signal) => {
                        if (!openaiClient) return
                        await openaiClient.post<Job>(
                          `/batches/${encodeURIComponent(job.id)}/reconcile`,
                          { body: { after: null }, signal, maxRetries: 0 },
                        )
                        await load(signal)
                      })
                    }
                  >
                    Reconcile saved identity
                  </Button>
                )}
              {['submitted', 'cancel_pending_outcome'].includes(
                local.phase,
              ) && (
                <Button
                  size="sm"
                  variant="outline"
                  disabled={busy}
                  onClick={() =>
                    setWatch(
                      watch?.id === job.id
                        ? null
                        : { id: job.id, until: Date.now() + 300000 },
                    )
                  }
                >
                  {watch?.id === job.id
                    ? 'Stop watching'
                    : 'Watch for 5 minutes'}
                </Button>
              )}
              {terminal && local.cleanup.local_payloads !== 'deleted' && (
                <Button
                  size="sm"
                  variant="outline"
                  disabled={busy}
                  onClick={() => setCleanup({ job, remote: false })}
                >
                  Delete local payloads
                </Button>
              )}
              {terminal && local.accounting_settled && (
                <Button
                  size="sm"
                  variant="outline"
                  disabled={busy}
                  onClick={() => setCleanup({ job, remote: true })}
                >
                  Clean provider payloads
                </Button>
              )}
              {!terminal && (
                <Button
                  size="sm"
                  variant="outline"
                  disabled={busy}
                  onClick={() => setCancel(job)}
                >
                  {local.worker_running ? 'Stop worker' : 'Cancel batch'}
                </Button>
              )}
              {(
                [
                  ['Output', job.output_file_id],
                  ['Errors', job.error_file_id],
                ] as const
              ).map(
                ([label, id]) =>
                  id && (
                    <Button
                      key={label}
                      size="sm"
                      variant="outline"
                      disabled={busy}
                      onClick={() => void act((signal) => download(id, signal))}
                    >
                      <Download className="mr-2 h-4 w-4" />
                      {label}
                    </Button>
                  ),
              )}
            </div>
            {local.phase.includes('pending_outcome') && (
              <p className="text-xs text-muted-foreground">
                The last native operation has an uncertain outcome. Submission
                is locked to prevent duplicate paid work.
              </p>
            )}
          </div>
        )
      })}
      {page?.has_more && page.last_id && (
        <Button
          variant="outline"
          disabled={busy}
          onClick={() => void act((signal) => load(signal, page.last_id!))}
        >
          Load more
        </Button>
      )}
      <AlertDialog
        open={!!cleanup}
        onOpenChange={(open) => !open && setCleanup(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {cleanup?.remote
                ? 'Clean provider payloads?'
                : 'Delete local batch payloads?'}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {cleanup?.remote
                ? 'This explicitly deletes only files or the terminal batch linked to this job at its original provider account. Some providers retain upstream copies or do not support deletion; the outcome remains visible.'
                : 'Delete generated snapshots and output/error payloads locally. Original uploaded input files are managed separately in Files. Usage records, unknown costs and quota holds remain; payloads will not be downloaded again by refresh.'}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep payloads</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                const selected = cleanup
                setCleanup(null)
                if (selected)
                  void act(async (signal) => {
                    if (!openaiClient) return
                    await openaiClient.post<Job>(
                      `/batches/${encodeURIComponent(selected.job.id)}/cleanup`,
                      {
                        body: {
                          local_payloads: !selected.remote,
                          remote: selected.remote,
                        },
                        signal,
                        maxRetries: 0,
                      },
                    )
                    await load(signal)
                  })
              }}
            >
              Delete payloads
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog
        open={!!submit}
        onOpenChange={(open) => !open && setSubmit(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Submit this prepared batch?</AlertDialogTitle>
            <AlertDialogDescription>
              This uploads the sealed snapshot and starts{' '}
              {submit?.request_counts.total} native provider requests. Estimated
              reserved cost:{' '}
              {money(submit?.localrouter.reserved_usage?.cost_usd)}. Unknown
              prices remain unknown. Native jobs may run for up to 24 hours;
              cancellation depends on the provider and completed work may be
              charged.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep prepared</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                const job = submit
                setSubmit(null)
                if (job) void act((signal) => control(job, 'submit', signal))
              }}
            >
              Submit batch
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog
        open={!!cancel}
        onOpenChange={(open) => !open && setCancel(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Stop or cancel this batch?</AlertDialogTitle>
            <AlertDialogDescription>
              A running local worker stops first. Read local status before
              cancelling again. Prepared inference can be cancelled locally.
              OpenAI native cancellation can take several minutes and completed
              work may be charged. OpenRouter has no verified native
              cancellation operation.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep batch</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                const job = cancel
                setCancel(null)
                if (job) void act((signal) => control(job, 'cancel', signal))
              }}
            >
              Stop or cancel
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
