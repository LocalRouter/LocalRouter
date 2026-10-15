import { useEffect, useRef, useState } from 'react'
import type OpenAI from 'openai'
import { Download, Loader2, RefreshCw, Trash2, Upload } from 'lucide-react'
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

type LocalFile = OpenAI.Files.FileObject & { expires_at?: number }

export function FilesPanel({
  openaiClient,
  isReady,
}: {
  openaiClient: OpenAI | null
  isReady: boolean
}) {
  const [files, setFiles] = useState<LocalFile[]>([])
  const [selected, setSelected] = useState<File | null>(null)
  const [pendingDelete, setPendingDelete] = useState<LocalFile | null>(null)
  const [cursor, setCursor] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const controller = useRef<AbortController | null>(null)
  const mounted = useRef(true)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      controller.current?.abort()
    }
  }, [])

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
        setError(e instanceof Error ? e.message : 'File operation failed.')
    } finally {
      if (mounted.current) setBusy(false)
    }
  }
  async function load(signal: AbortSignal, after?: string) {
    if (!openaiClient) return
    const page = await openaiClient.files.list(
      {
        purpose: 'batch',
        limit: 20,
        order: 'desc',
        ...(after ? { after } : {}),
      },
      { signal, maxRetries: 0 },
    )
    if (!mounted.current) return
    setFiles((previous) => (after ? [...previous, ...page.data] : page.data))
    setCursor(page.has_more ? (page.data.at(-1)?.id ?? null) : null)
  }

  return (
    <div className="space-y-4">
      <p className="text-sm text-muted-foreground">
        Store batch JSONL locally for this client. Uploading creates no provider
        request. Inputs are limited to 16 MiB, 50,000 rows and 1 MiB per row;
        default retention is 30 days. Submission policy is checked separately.
      </p>
      <div className="flex items-end gap-2">
        <div className="flex-1 space-y-2">
          <Label htmlFor="batch-local-file">JSONL input</Label>
          <Input
            id="batch-local-file"
            type="file"
            accept=".jsonl,application/jsonl"
            disabled={busy || !isReady}
            onChange={(event) => setSelected(event.target.files?.[0] ?? null)}
          />
        </div>
        <Button
          disabled={busy || !isReady || !selected}
          onClick={() =>
            void act(async (signal) => {
              if (!selected || !openaiClient) return
              if (selected.size > 16 * 1024 * 1024)
                throw new Error('Local files are limited to 16 MiB.')
              await openaiClient.files.create(
                { file: selected, purpose: 'batch' },
                { signal, maxRetries: 0 },
              )
              await load(signal)
            })
          }
        >
          <Upload className="mr-2 h-4 w-4" />
          Upload locally
        </Button>
        <Button
          variant="outline"
          disabled={busy || !isReady}
          onClick={() => void act((signal) => load(signal))}
        >
          <RefreshCw className="mr-2 h-4 w-4" />
          Refresh
        </Button>
      </div>
      {busy && <Loader2 className="h-4 w-4 animate-spin" />}
      {error && (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      )}
      {!files.length && (
        <p className="text-sm text-muted-foreground">
          Refresh to list this client's local files.
        </p>
      )}
      <div className="space-y-2">
        {files.map((file) => (
          <div
            key={file.id}
            className="flex items-center gap-3 rounded-md border p-3"
          >
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm font-medium">{file.filename}</p>
              <p className="break-all text-xs text-muted-foreground">
                {file.id} · {file.bytes.toLocaleString()} bytes
                {file.expires_at
                  ? ` · expires ${new Date(file.expires_at * 1000).toLocaleString()}`
                  : ''}
              </p>
            </div>
            <Button
              variant="outline"
              size="sm"
              disabled={busy || !isReady}
              aria-label={`Download ${file.filename}`}
              onClick={() =>
                void act(async (signal) => {
                  if (!openaiClient) return
                  const response = await openaiClient.files.content(file.id, {
                    signal,
                    maxRetries: 0,
                  })
                  const blob = await response.blob()
                  if (!mounted.current) return
                  const url = URL.createObjectURL(blob)
                  const link = document.createElement('a')
                  link.href = url
                  link.download = file.filename.replace(/[\\/\x00-\x1f]/g, '_')
                  link.click()
                  setTimeout(() => URL.revokeObjectURL(url), 1000)
                })
              }
            >
              <Download className="h-4 w-4" />
            </Button>
            <Button
              variant="outline"
              size="sm"
              disabled={busy || !isReady}
              aria-label={`Delete ${file.filename}`}
              onClick={() => setPendingDelete(file)}
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
        ))}
      </div>
      {cursor && (
        <Button
          variant="outline"
          disabled={busy || !isReady}
          onClick={() => void act((signal) => load(signal, cursor))}
        >
          Load more
        </Button>
      )}
      <AlertDialog
        open={!!pendingDelete}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete local file?</AlertDialogTitle>
            <AlertDialogDescription>
              Delete {pendingDelete?.filename} from this client's local storage.
              Files required by active jobs cannot be deleted.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep file</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                const file = pendingDelete
                setPendingDelete(null)
                if (file)
                  void act(async (signal) => {
                    await openaiClient?.files.delete(file.id, {
                      signal,
                      maxRetries: 0,
                    })
                    await load(signal)
                  })
              }}
            >
              Delete file
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
