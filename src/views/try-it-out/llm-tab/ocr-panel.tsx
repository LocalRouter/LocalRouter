import { useEffect, useRef, useState } from 'react'
import type OpenAI from 'openai'
import { FileText, Square } from 'lucide-react'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
import { Label } from '@/components/ui/label'
import { JsonTree } from '@/components/shared/JsonTree'

interface OcrResponse {
  model: string
  pages: { index: number; markdown: string }[]
  usage_info: { pages_processed: number }
  extensions?: Record<string, unknown>
}
export function OcrPanel({
  openaiClient,
  isReady,
}: {
  openaiClient: OpenAI | null
  isReady: boolean
}) {
  const [model, setModel] = useState('')
  const [pages, setPages] = useState('0')
  const [file, setFile] = useState<File | null>(null)
  const [running, setRunning] = useState(false)
  const [result, setResult] = useState<OcrResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const abort = useRef<AbortController | null>(null)
  useEffect(() => () => abort.current?.abort(), [])
  async function run() {
    if (!openaiClient || !isReady || !file || !model.trim() || running) return
    setRunning(true)
    setError(null)
    setResult(null)
    const controller = new AbortController()
    abort.current = controller
    try {
      if (!file.size || file.size > 4 * 1024 * 1024)
        throw new Error('Select a PDF, PNG or JPEG up to 4 MiB.')
      const extension = file.name.toLowerCase().split('.').pop()
      const mime =
        extension === 'pdf'
          ? 'application/pdf'
          : extension === 'png'
            ? 'image/png'
            : ['jpg', 'jpeg'].includes(extension ?? '')
              ? 'image/jpeg'
              : null
      if (!mime) throw new Error('Select a PDF, PNG or JPEG file.')
      const pageSelection = /^\d+-\d+$/.test(pages.trim())
        ? pages.trim()
        : pages
            .trim()
            .split(',')
            .map((value) => {
              if (!/^\d+$/.test(value.trim()))
                throw new Error(
                  'Enter zero-based page numbers, such as 0,2,4 or 0-4.',
                )
              return Number(value.trim())
            })
      const data = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader()
        reader.onload = () => resolve(String(reader.result).split(',')[1])
        reader.onerror = () =>
          reject(new Error('Could not read the selected file.'))
        reader.readAsDataURL(file)
      })
      if (controller.signal.aborted) return
      const inline = `data:${mime};base64,${data}`
      const response = await openaiClient.post<OcrResponse>('/ocr', {
        body: {
          model: model.trim(),
          filename: file.name,
          document:
            mime === 'application/pdf'
              ? { type: 'document_url', document_url: inline }
              : { type: 'image_url', image_url: inline },
          pages: pageSelection,
          include_image_base64: false,
          image_limit: 0,
        },
        signal: controller.signal,
        maxRetries: 0,
      })
      if (!controller.signal.aborted) setResult(response)
    } catch (error) {
      setError(
        controller.signal.aborted
          ? 'OCR cancelled. The provider may still have processed and charged for selected pages.'
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
        Extract selected pages into text. Enable Allow OCR in client Settings
        first. OCR sends the selected file to the provider when you press
        Extract pages; page fees may apply.
      </p>
      <div className="space-y-2">
        <Label htmlFor="ocr-model">OCR model</Label>
        <Input
          id="ocr-model"
          value={model}
          disabled={running}
          onChange={(e) => {
            setModel(e.target.value)
            setResult(null)
          }}
          placeholder="mistral/mistral-ocr-latest or gateway/mistral-ocr-latest"
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="ocr-pages">Pages (zero-based, at most 20)</Label>
        <Input
          id="ocr-pages"
          value={pages}
          disabled={running}
          maxLength={128}
          onChange={(e) => {
            setPages(e.target.value)
            setResult(null)
          }}
          placeholder="0,2,4 or 0-4"
        />
      </div>
      <p className="text-xs text-muted-foreground">
        Page 0 is the first page. Image OCR requires page 0. Full-document
        processing is never selected implicitly. Provider charges remain unknown
        when no fee is reported.
      </p>
      <div className="space-y-2">
        <Label htmlFor="ocr-file">OCR file</Label>
        <Input
          id="ocr-file"
          type="file"
          accept=".pdf,.png,.jpg,.jpeg"
          disabled={running}
          onChange={(e) => {
            setFile(e.target.files?.[0] ?? null)
            setResult(null)
            setError(null)
          }}
        />
      </div>
      <div className="flex gap-2">
        <Button
          disabled={
            !isReady || running || !file || !model.trim() || !pages.trim()
          }
          onClick={() => void run()}
        >
          <FileText className="h-4 w-4 mr-2" />
          {running ? 'Extracting…' : 'Extract pages'}
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
        <div className="space-y-4">
          <p className="text-xs text-muted-foreground">
            Reported processed pages: {result.usage_info.pages_processed} · Fee
            unknown
          </p>
          {result.pages.map((page) => (
            <article
              key={page.index}
              className="border rounded-md p-3 space-y-2"
            >
              <p className="text-sm font-medium">Page index {page.index}</p>
              <pre className="text-sm whitespace-pre-wrap break-words font-sans">
                {page.markdown}
              </pre>
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
