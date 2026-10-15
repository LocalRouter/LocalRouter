import { useEffect, useRef, useState } from 'react'
import type OpenAI from 'openai'
import { FileText, Square } from 'lucide-react'
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

interface DocumentResponse {
  choices?: { message?: { content?: string } }[]
}

export function DocumentsPanel({
  openaiClient,
  isReady,
}: {
  openaiClient: OpenAI | null
  isReady: boolean
}) {
  const [model, setModel] = useState('')
  const [prompt, setPrompt] = useState(
    'Summarize this document and identify the source of each claim.',
  )
  const [mode, setMode] = useState('text')
  const [file, setFile] = useState<File | null>(null)
  const [running, setRunning] = useState(false)
  const [result, setResult] = useState<DocumentResponse | null>(null)
  const [error, setError] = useState<string | null>(null)
  const abort = useRef<AbortController | null>(null)
  useEffect(() => () => abort.current?.abort(), [])
  async function run() {
    if (!openaiClient || !isReady || !file || !model.trim() || running) return
    setRunning(true)
    setResult(null)
    setError(null)
    const controller = new AbortController()
    abort.current = controller
    try {
      const pdf = file.name.toLowerCase().endsWith('.pdf')
      if (!file.size || file.size > (pdf ? 4 * 1024 * 1024 : 512 * 1024))
        throw new Error('Choose a PDF up to 4 MiB or UTF-8 text up to 512 KiB.')
      if (pdf && mode === 'text')
        throw new Error(
          'Extract the PDF with a configured local MCP tool first and select its UTF-8 text output, or choose native PDF processing.',
        )
      const data = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader()
        reader.onload = () => resolve(String(reader.result).split(',')[1])
        reader.onerror = () =>
          reject(new Error('Could not read the selected file.'))
        reader.onabort = () => reject(new Error('File read cancelled.'))
        reader.readAsDataURL(file)
      })
      if (controller.signal.aborted) return
      const response = await openaiClient.post<DocumentResponse>(
        '/chat/completions',
        {
          body: {
            model: model.trim(),
            stream: false,
            max_tokens: 1024,
            messages: [
              {
                role: 'user',
                content: [
                  { type: 'text', text: prompt },
                  {
                    type: 'file',
                    file: {
                      filename: file.name,
                      file_data: `data:${pdf ? 'application/pdf' : 'text/plain'};base64,${data}`,
                    },
                  },
                ],
              },
            ],
            extensions: { 'localrouter.documents': { mode } },
          },
          signal: controller.signal,
          maxRetries: 0,
        },
      )
      if (!controller.signal.aborted) setResult(response)
    } catch (error) {
      setError(
        controller.signal.aborted
          ? 'Cancelled. A dispatched request may still incur provider charges.'
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
        Attach locally extracted text or send a PDF to a verified native
        document route. Selecting a file keeps it local until you press Send
        document.
      </p>
      <div className="space-y-2">
        <Label htmlFor="document-model">Model</Label>
        <Input
          id="document-model"
          value={model}
          onChange={(e) => {
            setModel(e.target.value)
            setResult(null)
          }}
          disabled={running}
          placeholder="gateway/google/gemini-model"
        />
      </div>
      <div className="space-y-2">
        <Label>Processing</Label>
        <Select
          value={mode}
          onValueChange={(value) => {
            setMode(value)
            setResult(null)
          }}
          disabled={running}
        >
          <SelectTrigger aria-label="Document processing">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="text">Locally supplied UTF-8 text</SelectItem>
            <SelectItem value="native">Native PDF / document</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <p className="text-xs text-muted-foreground">
        Native PDF: OpenRouter with the native parser engine, or LLM Gateway
        with an explicit google/gemini-* or google-vertex/gemini-* model.
        Provider model support still applies. Opaque PDFs require extracted text
        when blocking document scans or cost ceilings are configured.
      </p>
      <div className="space-y-2">
        <Label htmlFor="document-file">Document</Label>
        <Input
          id="document-file"
          type="file"
          accept=".pdf,.txt,.md,.csv,.html,.xml,.css,.js"
          disabled={running}
          onChange={(e) => {
            setFile(e.target.files?.[0] ?? null)
            setResult(null)
            setError(null)
          }}
        />
      </div>
      <div className="space-y-2">
        <Label htmlFor="document-prompt">Instructions</Label>
        <Textarea
          id="document-prompt"
          value={prompt}
          disabled={running}
          onChange={(e) => {
            setPrompt(e.target.value)
            setResult(null)
          }}
        />
      </div>
      <div className="flex gap-2">
        <Button
          disabled={!isReady || !file || !model.trim() || running}
          onClick={() => void run()}
        >
          <FileText className="h-4 w-4 mr-2" />
          {running ? 'Processing…' : 'Send document'}
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
      {result?.choices?.[0]?.message?.content && (
        <p className="text-sm whitespace-pre-wrap">
          {result.choices[0].message.content}
        </p>
      )}
      {result != null && <JsonTree data={result} />}
    </div>
  )
}
