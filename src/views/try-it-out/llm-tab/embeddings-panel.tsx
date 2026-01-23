import { useState, useCallback } from "react"
import { Hash, RefreshCw, Copy, Check, ArrowRightLeft } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { Badge } from "@/components/ui/Badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import type OpenAI from "openai"

interface EmbeddingResult {
  id: string
  text: string
  model: string
  dimensions: number
  embedding: number[]
  timestamp: Date
}

interface EmbeddingsPanelProps {
  openaiClient: OpenAI | null
  isReady: boolean
}

type EmbeddingModel = "text-embedding-3-small" | "text-embedding-3-large" | "text-embedding-ada-002"

const EMBEDDING_MODELS: { value: EmbeddingModel; label: string; dimensions: number }[] = [
  { value: "text-embedding-3-small", label: "text-embedding-3-small", dimensions: 1536 },
  { value: "text-embedding-3-large", label: "text-embedding-3-large", dimensions: 3072 },
  { value: "text-embedding-ada-002", label: "text-embedding-ada-002", dimensions: 1536 },
]

// Calculate cosine similarity between two vectors
function cosineSimilarity(a: number[], b: number[]): number {
  if (a.length !== b.length) return 0
  let dotProduct = 0
  let normA = 0
  let normB = 0
  for (let i = 0; i < a.length; i++) {
    dotProduct += a[i] * b[i]
    normA += a[i] * a[i]
    normB += b[i] * b[i]
  }
  return dotProduct / (Math.sqrt(normA) * Math.sqrt(normB))
}

export function EmbeddingsPanel({ openaiClient, isReady }: EmbeddingsPanelProps) {
  const [text, setText] = useState("")
  const [model, setModel] = useState<EmbeddingModel>("text-embedding-3-small")
  const [isGenerating, setIsGenerating] = useState(false)
  const [results, setResults] = useState<EmbeddingResult[]>([])
  const [error, setError] = useState<string | null>(null)
  const [copiedId, setCopiedId] = useState<string | null>(null)

  // Comparison mode
  const [compareMode, setCompareMode] = useState(false)
  const [selectedForCompare, setSelectedForCompare] = useState<string[]>([])

  const handleGenerate = useCallback(async () => {
    if (!openaiClient || !text.trim()) return

    setIsGenerating(true)
    setError(null)

    try {
      const response = await openaiClient.embeddings.create({
        model,
        input: text.trim(),
      })

      const embeddingData = response.data[0]
      const result: EmbeddingResult = {
        id: crypto.randomUUID(),
        text: text.trim(),
        model,
        dimensions: embeddingData.embedding.length,
        embedding: embeddingData.embedding,
        timestamp: new Date(),
      }

      setResults((prev) => [result, ...prev])
      setText("")
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to generate embedding")
    } finally {
      setIsGenerating(false)
    }
  }, [openaiClient, text, model])

  const handleCopy = async (result: EmbeddingResult) => {
    await navigator.clipboard.writeText(JSON.stringify(result.embedding))
    setCopiedId(result.id)
    setTimeout(() => setCopiedId(null), 2000)
  }

  const handleToggleCompare = (id: string) => {
    setSelectedForCompare((prev) => {
      if (prev.includes(id)) {
        return prev.filter((i) => i !== id)
      }
      if (prev.length < 2) {
        return [...prev, id]
      }
      return [prev[1], id]
    })
  }

  const getComparisonScore = (): number | null => {
    if (selectedForCompare.length !== 2) return null
    const a = results.find((r) => r.id === selectedForCompare[0])
    const b = results.find((r) => r.id === selectedForCompare[1])
    if (!a || !b) return null
    return cosineSimilarity(a.embedding, b.embedding)
  }

  const comparisonScore = getComparisonScore()

  return (
    <div className="flex flex-col h-full gap-4">
      {/* Generation Controls */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base flex items-center gap-2">
            <Hash className="h-4 w-4" />
            Embeddings
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* Text Input */}
          <div className="space-y-2">
            <Label>Text to embed</Label>
            <Textarea
              placeholder="Enter text to generate embedding..."
              value={text}
              onChange={(e) => setText(e.target.value)}
              rows={3}
              disabled={isGenerating}
            />
          </div>

          {/* Model Selection */}
          <div className="flex items-center gap-4">
            <div className="flex-1 space-y-2">
              <Label>Model</Label>
              <Select value={model} onValueChange={(v: string) => setModel(v as EmbeddingModel)}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {EMBEDDING_MODELS.map((m) => (
                    <SelectItem key={m.value} value={m.value}>
                      {m.label} ({m.dimensions}d)
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <Button
              onClick={handleGenerate}
              disabled={!isReady || !text.trim() || isGenerating}
              className="mt-6"
            >
              {isGenerating ? (
                <>
                  <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                  Generating...
                </>
              ) : (
                <>
                  <Hash className="h-4 w-4 mr-2" />
                  Generate
                </>
              )}
            </Button>
          </div>

          {error && (
            <div className="p-3 bg-destructive/10 text-destructive rounded-md text-sm">
              {error}
            </div>
          )}
        </CardContent>
      </Card>

      {/* Results */}
      <Card className="flex-1 min-h-0">
        <CardHeader className="pb-3">
          <div className="flex items-center justify-between">
            <CardTitle className="text-base">Results ({results.length})</CardTitle>
            <div className="flex items-center gap-2">
              {compareMode && comparisonScore !== null && (
                <Badge variant="outline" className="text-sm">
                  Similarity: {(comparisonScore * 100).toFixed(2)}%
                </Badge>
              )}
              <Button
                variant={compareMode ? "default" : "outline"}
                size="sm"
                onClick={() => {
                  setCompareMode(!compareMode)
                  setSelectedForCompare([])
                }}
              >
                <ArrowRightLeft className="h-4 w-4 mr-1" />
                Compare
              </Button>
            </div>
          </div>
          {compareMode && (
            <p className="text-xs text-muted-foreground mt-1">
              Select two embeddings to compare their cosine similarity
            </p>
          )}
        </CardHeader>
        <CardContent className="h-[calc(100%-5rem)]">
          <ScrollArea className="h-full">
            {results.length === 0 ? (
              <div className="flex items-center justify-center h-64 text-muted-foreground">
                <p className="text-sm">Generated embeddings will appear here</p>
              </div>
            ) : (
              <div className="space-y-3">
                {results.map((result) => {
                  const isSelected = selectedForCompare.includes(result.id)
                  return (
                    <div
                      key={result.id}
                      className={`border rounded-lg p-3 space-y-2 transition-colors ${
                        isSelected ? "border-primary bg-primary/5" : ""
                      } ${compareMode ? "cursor-pointer hover:border-primary/50" : ""}`}
                      onClick={() => compareMode && handleToggleCompare(result.id)}
                    >
                      <div className="flex items-start justify-between gap-2">
                        <p className="text-sm flex-1 line-clamp-2">{result.text}</p>
                        {!compareMode && (
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => handleCopy(result)}
                          >
                            {copiedId === result.id ? (
                              <Check className="h-4 w-4 text-green-500" />
                            ) : (
                              <Copy className="h-4 w-4" />
                            )}
                          </Button>
                        )}
                      </div>
                      <div className="flex items-center gap-2">
                        <Badge variant="outline" className="text-xs">
                          {result.model}
                        </Badge>
                        <Badge variant="secondary" className="text-xs">
                          {result.dimensions}d
                        </Badge>
                        <span className="text-xs text-muted-foreground">
                          {result.timestamp.toLocaleTimeString()}
                        </span>
                      </div>
                      <div className="p-2 bg-muted rounded text-xs font-mono overflow-hidden">
                        [{result.embedding.slice(0, 5).map((n) => n.toFixed(6)).join(", ")}, ...]
                      </div>
                    </div>
                  )
                })}
              </div>
            )}
          </ScrollArea>
        </CardContent>
      </Card>
    </div>
  )
}
