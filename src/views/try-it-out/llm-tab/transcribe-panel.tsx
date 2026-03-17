import { useState, useCallback, useRef } from "react"
import { Mic, RefreshCw, Copy, Check, Upload, X } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Label } from "@/components/ui/label"
import { Badge } from "@/components/ui/Badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import type OpenAI from "openai"

const MAX_FILE_SIZE = 25 * 1024 * 1024 // 25MB
const ACCEPTED_EXTENSIONS = ".wav,.mp3,.m4a,.ogg,.flac,.webm,.mp4,.mpeg,.mpga"

type TranscribeMode = "transcribe" | "translate"

const LANGUAGES: { value: string; label: string }[] = [
  { value: "auto", label: "Auto-detect" },
  { value: "en", label: "English" },
  { value: "es", label: "Spanish" },
  { value: "fr", label: "French" },
  { value: "de", label: "German" },
  { value: "it", label: "Italian" },
  { value: "pt", label: "Portuguese" },
  { value: "zh", label: "Chinese" },
  { value: "ja", label: "Japanese" },
  { value: "ko", label: "Korean" },
  { value: "ru", label: "Russian" },
  { value: "ar", label: "Arabic" },
  { value: "hi", label: "Hindi" },
  { value: "nl", label: "Dutch" },
  { value: "pl", label: "Polish" },
  { value: "sv", label: "Swedish" },
  { value: "tr", label: "Turkish" },
]

interface TranscriptionResult {
  id: string
  fileName: string
  fileSize: number
  mode: TranscribeMode
  model: string
  language?: string
  text: string
  duration?: number
  timestamp: Date
  latencyMs: number
}

interface TranscribePanelProps {
  openaiClient: OpenAI | null
  isReady: boolean
  selectedModel: string
}

function formatFileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

function formatDuration(seconds: number): string {
  const mins = Math.floor(seconds / 60)
  const secs = Math.round(seconds % 60)
  return mins > 0 ? `${mins}m ${secs}s` : `${secs}s`
}

export function TranscribePanel({ openaiClient, isReady, selectedModel }: TranscribePanelProps) {
  const [mode, setMode] = useState<TranscribeMode>("transcribe")
  const [file, setFile] = useState<File | null>(null)
  const [language, setLanguage] = useState("auto")
  const [isDragging, setIsDragging] = useState(false)
  const [isProcessing, setIsProcessing] = useState(false)
  const [results, setResults] = useState<TranscriptionResult[]>([])
  const [error, setError] = useState<string | null>(null)
  const [copiedId, setCopiedId] = useState<string | null>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)

  const validateFile = (f: File): string | null => {
    if (f.size > MAX_FILE_SIZE) {
      return `File too large (${formatFileSize(f.size)}). Maximum is 25 MB.`
    }
    return null
  }

  const handleFileSelect = (f: File) => {
    const validationError = validateFile(f)
    if (validationError) {
      setError(validationError)
      return
    }
    setError(null)
    setFile(f)
  }

  const handleInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const f = e.target.files?.[0]
    if (f) handleFileSelect(f)
    // Reset input so the same file can be re-selected
    e.target.value = ""
  }

  const handleDragOver = (e: React.DragEvent) => {
    e.preventDefault()
    setIsDragging(true)
  }

  const handleDragLeave = (e: React.DragEvent) => {
    e.preventDefault()
    setIsDragging(false)
  }

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault()
    setIsDragging(false)
    const f = e.dataTransfer.files[0]
    if (f) handleFileSelect(f)
  }

  const handleTranscribe = useCallback(async () => {
    if (!openaiClient || !file || !selectedModel) return

    setIsProcessing(true)
    setError(null)

    const startTime = performance.now()

    try {
      let text: string
      let detectedLanguage: string | undefined
      let duration: number | undefined

      if (mode === "transcribe") {
        const result = await openaiClient.audio.transcriptions.create({
          file,
          model: selectedModel,
          language: language !== "auto" ? language : undefined,
          response_format: "verbose_json",
        })
        // verbose_json response includes extra fields
        const verbose = result as unknown as {
          text: string
          language?: string
          duration?: number
        }
        text = verbose.text
        detectedLanguage = verbose.language
        duration = verbose.duration
      } else {
        const result = await openaiClient.audio.translations.create({
          file,
          model: selectedModel,
          response_format: "verbose_json",
        })
        const verbose = result as unknown as {
          text: string
          language?: string
          duration?: number
        }
        text = verbose.text
        detectedLanguage = verbose.language
        duration = verbose.duration
      }

      const latencyMs = Math.round(performance.now() - startTime)

      const transcriptionResult: TranscriptionResult = {
        id: crypto.randomUUID(),
        fileName: file.name,
        fileSize: file.size,
        mode,
        model: selectedModel,
        language: detectedLanguage,
        text,
        duration,
        timestamp: new Date(),
        latencyMs,
      }

      setResults((prev) => [transcriptionResult, ...prev])
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to transcribe audio")
    } finally {
      setIsProcessing(false)
    }
  }, [openaiClient, file, selectedModel, mode, language])

  const handleCopy = async (result: TranscriptionResult) => {
    await navigator.clipboard.writeText(result.text)
    setCopiedId(result.id)
    setTimeout(() => setCopiedId(null), 2000)
  }

  return (
    <div className="flex flex-col h-full gap-4">
      {/* Controls */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base flex items-center gap-2">
            <Mic className="h-4 w-4" />
            Audio Transcription
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* Mode Toggle */}
          <div className="space-y-2">
            <Label>Mode</Label>
            <RadioGroup
              value={mode}
              onValueChange={(v: string) => setMode(v as TranscribeMode)}
              className="flex gap-4"
            >
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="transcribe" id="mode-transcribe" />
                <Label htmlFor="mode-transcribe" className="cursor-pointer">
                  Transcribe
                </Label>
              </div>
              <div className="flex items-center space-x-2">
                <RadioGroupItem value="translate" id="mode-translate" />
                <Label htmlFor="mode-translate" className="cursor-pointer">
                  Translate to English
                </Label>
              </div>
            </RadioGroup>
          </div>

          {/* File Upload */}
          <div className="space-y-2">
            <Label>Audio File</Label>
            <input
              ref={fileInputRef}
              type="file"
              accept={ACCEPTED_EXTENSIONS}
              onChange={handleInputChange}
              className="hidden"
            />
            {file ? (
              <div className="flex items-center gap-2 p-3 border rounded-md bg-muted/30">
                <Mic className="h-4 w-4 text-muted-foreground shrink-0" />
                <span className="text-sm truncate flex-1">{file.name}</span>
                <Badge variant="secondary" className="text-xs shrink-0">
                  {formatFileSize(file.size)}
                </Badge>
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-6 w-6 p-0 shrink-0"
                  onClick={() => setFile(null)}
                >
                  <X className="h-3.5 w-3.5" />
                </Button>
              </div>
            ) : (
              <div
                onClick={() => fileInputRef.current?.click()}
                onDragOver={handleDragOver}
                onDragLeave={handleDragLeave}
                onDrop={handleDrop}
                className={`flex flex-col items-center justify-center gap-2 p-6 border-2 border-dashed rounded-md cursor-pointer transition-colors ${
                  isDragging
                    ? "border-primary bg-primary/5"
                    : "border-muted-foreground/25 hover:border-muted-foreground/50"
                }`}
              >
                <Upload className="h-6 w-6 text-muted-foreground" />
                <p className="text-sm text-muted-foreground">
                  Drag & drop an audio file or click to browse
                </p>
                <p className="text-xs text-muted-foreground/60">
                  WAV, MP3, M4A, OGG, FLAC, WebM (max 25 MB)
                </p>
              </div>
            )}
          </div>

          {/* Language (transcribe mode only) */}
          {mode === "transcribe" && (
            <div className="space-y-2">
              <Label>Language</Label>
              <Select value={language} onValueChange={setLanguage}>
                <SelectTrigger className="w-full max-w-[200px]">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {LANGUAGES.map((lang) => (
                    <SelectItem key={lang.value} value={lang.value}>
                      {lang.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          )}

          {error && (
            <div className="p-3 bg-destructive/10 text-destructive rounded-md text-sm">
              {error}
            </div>
          )}

          <Button
            onClick={handleTranscribe}
            disabled={!isReady || !file || !selectedModel || isProcessing}
            className="w-full"
          >
            {isProcessing ? (
              <>
                <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                {mode === "transcribe" ? "Transcribing..." : "Translating..."}
              </>
            ) : (
              <>
                <Mic className="h-4 w-4 mr-2" />
                {mode === "transcribe" ? "Transcribe" : "Translate"}
              </>
            )}
          </Button>
        </CardContent>
      </Card>

      {/* Results */}
      <Card className="flex-1 min-h-0">
        <CardHeader className="pb-3">
          <CardTitle className="text-base">Results ({results.length})</CardTitle>
        </CardHeader>
        <CardContent className="h-[calc(100%-4rem)]">
          <ScrollArea className="h-full">
            {results.length === 0 ? (
              <div className="flex items-center justify-center h-64 text-muted-foreground">
                <p className="text-sm">Transcription results will appear here</p>
              </div>
            ) : (
              <div className="space-y-3">
                {results.map((result) => (
                  <div key={result.id} className="border rounded-lg p-3 space-y-2">
                    <div className="flex items-start justify-between gap-2">
                      <p className="text-sm flex-1 whitespace-pre-wrap">
                        {result.text || <span className="text-muted-foreground italic">No speech detected</span>}
                      </p>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleCopy(result)}
                        disabled={!result.text}
                      >
                        {copiedId === result.id ? (
                          <Check className="h-4 w-4 text-green-500" />
                        ) : (
                          <Copy className="h-4 w-4" />
                        )}
                      </Button>
                    </div>
                    <div className="flex flex-wrap items-center gap-1">
                      <Badge variant="outline" className="text-xs">
                        {result.model}
                      </Badge>
                      <Badge variant="secondary" className="text-xs">
                        {result.mode === "transcribe" ? "Transcribe" : "Translate"}
                      </Badge>
                      {result.language && (
                        <Badge variant="secondary" className="text-xs">
                          {result.language}
                        </Badge>
                      )}
                      {result.duration != null && (
                        <Badge variant="secondary" className="text-xs">
                          {formatDuration(result.duration)}
                        </Badge>
                      )}
                      <Badge variant="secondary" className="text-xs">
                        {result.latencyMs}ms
                      </Badge>
                      <span className="text-xs text-muted-foreground ml-auto">
                        {result.fileName}
                      </span>
                    </div>
                  </div>
                ))}
              </div>
            )}
          </ScrollArea>
        </CardContent>
      </Card>
    </div>
  )
}
