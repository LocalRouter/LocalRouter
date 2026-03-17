import { useState, useCallback, useEffect, useRef } from "react"
import { Volume2, Download, RefreshCw, Play } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { Badge } from "@/components/ui/Badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Slider } from "@/components/ui/Slider"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import type OpenAI from "openai"

const MAX_CHARS = 4096

const VOICES = ["alloy", "echo", "fable", "onyx", "nova", "shimmer"] as const
type Voice = (typeof VOICES)[number]

const FORMATS = ["mp3", "opus", "aac", "flac", "wav"] as const
type AudioFormat = (typeof FORMATS)[number]

const FORMAT_MIME: Record<AudioFormat, string> = {
  mp3: "audio/mpeg",
  opus: "audio/opus",
  aac: "audio/aac",
  flac: "audio/flac",
  wav: "audio/wav",
}

interface GeneratedSpeech {
  id: string
  text: string
  model: string
  voice: Voice
  format: AudioFormat
  speed: number
  blobUrl: string
  blob: Blob
  timestamp: Date
  latencyMs: number
}

interface SpeechPanelProps {
  openaiClient: OpenAI | null
  isReady: boolean
  selectedModel: string
}

export function SpeechPanel({ openaiClient, isReady, selectedModel }: SpeechPanelProps) {
  const [text, setText] = useState("")
  const [voice, setVoice] = useState<Voice>("alloy")
  const [format, setFormat] = useState<AudioFormat>("mp3")
  const [speed, setSpeed] = useState(1.0)
  const [isGenerating, setIsGenerating] = useState(false)
  const [results, setResults] = useState<GeneratedSpeech[]>([])
  const [error, setError] = useState<string | null>(null)
  const blobUrlsRef = useRef<string[]>([])

  // Revoke all blob URLs on unmount
  useEffect(() => {
    return () => {
      for (const url of blobUrlsRef.current) {
        URL.revokeObjectURL(url)
      }
    }
  }, [])

  const handleGenerate = useCallback(async () => {
    if (!openaiClient || !text.trim() || !selectedModel) return
    if (text.length > MAX_CHARS) return

    setIsGenerating(true)
    setError(null)

    const startTime = performance.now()

    try {
      const response = await openaiClient.audio.speech.create({
        model: selectedModel,
        input: text.trim(),
        voice,
        response_format: format,
        speed,
      })

      const latencyMs = Math.round(performance.now() - startTime)

      // response is a Response object — extract blob
      const blob = new Blob(
        [await response.arrayBuffer()],
        { type: FORMAT_MIME[format] || "audio/mpeg" }
      )
      const blobUrl = URL.createObjectURL(blob)
      blobUrlsRef.current.push(blobUrl)

      const result: GeneratedSpeech = {
        id: crypto.randomUUID(),
        text: text.trim(),
        model: selectedModel,
        voice,
        format,
        speed,
        blobUrl,
        blob,
        timestamp: new Date(),
        latencyMs,
      }

      setResults((prev) => [result, ...prev])
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to generate speech")
    } finally {
      setIsGenerating(false)
    }
  }, [openaiClient, text, selectedModel, voice, format, speed])

  const handleDownload = (result: GeneratedSpeech) => {
    const link = document.createElement("a")
    link.href = result.blobUrl
    link.download = `speech-${result.id.slice(0, 8)}.${result.format}`
    link.click()
  }

  const charCount = text.length
  const overLimit = charCount > MAX_CHARS

  return (
    <div className="flex flex-col h-full gap-4">
      {/* Controls */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base flex items-center gap-2">
            <Volume2 className="h-4 w-4" />
            Text to Speech
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* Text Input */}
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <Label>Text</Label>
              <span className={`text-xs ${overLimit ? "text-destructive font-medium" : "text-muted-foreground"}`}>
                {charCount.toLocaleString()} / {MAX_CHARS.toLocaleString()}
              </span>
            </div>
            <Textarea
              placeholder="Enter text to synthesize into speech..."
              value={text}
              onChange={(e) => setText(e.target.value)}
              rows={4}
              disabled={isGenerating}
            />
          </div>

          {/* Options */}
          <div className="grid grid-cols-3 gap-4">
            <div className="space-y-2">
              <Label>Voice</Label>
              <Select value={voice} onValueChange={(v: string) => setVoice(v as Voice)}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {VOICES.map((v) => (
                    <SelectItem key={v} value={v}>
                      {v.charAt(0).toUpperCase() + v.slice(1)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <Label>Format</Label>
              <Select value={format} onValueChange={(v: string) => setFormat(v as AudioFormat)}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {FORMATS.map((f) => (
                    <SelectItem key={f} value={f}>
                      {f.toUpperCase()}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-2">
              <div className="flex items-center justify-between">
                <Label>Speed</Label>
                <span className="text-sm text-muted-foreground">{speed.toFixed(2)}x</span>
              </div>
              <Slider
                value={[speed]}
                onValueChange={(values: number[]) => setSpeed(values[0])}
                min={0.25}
                max={4.0}
                step={0.05}
              />
            </div>
          </div>

          {error && (
            <div className="p-3 bg-destructive/10 text-destructive rounded-md text-sm">
              {error}
            </div>
          )}

          <Button
            onClick={handleGenerate}
            disabled={!isReady || !text.trim() || !selectedModel || isGenerating || overLimit}
            className="w-full"
          >
            {isGenerating ? (
              <>
                <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                Generating...
              </>
            ) : (
              <>
                <Play className="h-4 w-4 mr-2" />
                Generate Speech
              </>
            )}
          </Button>
        </CardContent>
      </Card>

      {/* Results */}
      <Card className="flex-1 min-h-0">
        <CardHeader className="pb-3">
          <CardTitle className="text-base">Generated Audio ({results.length})</CardTitle>
        </CardHeader>
        <CardContent className="h-[calc(100%-4rem)]">
          <ScrollArea className="h-full">
            {results.length === 0 ? (
              <div className="flex items-center justify-center h-64 text-muted-foreground">
                <p className="text-sm">Generated audio will appear here</p>
              </div>
            ) : (
              <div className="space-y-3">
                {results.map((result) => (
                  <div key={result.id} className="border rounded-lg p-3 space-y-2">
                    <audio controls className="w-full" src={result.blobUrl} />
                    <p className="text-sm line-clamp-2">{result.text}</p>
                    <div className="flex items-center justify-between">
                      <div className="flex flex-wrap gap-1">
                        <Badge variant="outline" className="text-xs">
                          {result.model}
                        </Badge>
                        <Badge variant="secondary" className="text-xs">
                          {result.voice}
                        </Badge>
                        <Badge variant="secondary" className="text-xs">
                          {result.format}
                        </Badge>
                        <Badge variant="secondary" className="text-xs">
                          {result.speed}x
                        </Badge>
                        <Badge variant="secondary" className="text-xs">
                          {result.latencyMs}ms
                        </Badge>
                      </div>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => handleDownload(result)}
                      >
                        <Download className="h-4 w-4" />
                      </Button>
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
