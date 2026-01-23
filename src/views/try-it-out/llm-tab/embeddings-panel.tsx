import { useState, useCallback, useMemo } from "react"
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

interface Model {
  id: string
  object: string
  owned_by: string
}

interface EmbeddingsPanelProps {
  openaiClient: OpenAI | null
  isReady: boolean
  mode: "client" | "strategy" | "direct"
  selectedProvider?: string
  models: Model[]
}

// Known embedding model patterns by provider
const EMBEDDING_MODEL_PATTERNS: Record<string, RegExp[]> = {
  openai: [/^text-embedding/i, /^embedding/i],
  ollama: [/embed/i, /nomic/i, /bge/i, /e5/i, /gte/i, /mxbai/i],
  togetherai: [/bert/i, /bge/i, /e5/i, /embed/i],
  gemini: [/^text-embedding/i, /^embedding/i],
  voyage: [/^voyage/i],
  cohere: [/^embed/i],
  deepinfra: [/bge/i, /e5/i, /gte/i, /embed/i],
}

// Check if a model is likely an embedding model
function isEmbeddingModel(modelId: string, provider?: string): boolean {
  const lowerModelId = modelId.toLowerCase()

  // Check against known patterns for specific provider
  if (provider) {
    const providerLower = provider.toLowerCase()
    const patterns = EMBEDDING_MODEL_PATTERNS[providerLower]
    if (patterns) {
      for (const pattern of patterns) {
        if (pattern.test(lowerModelId)) return true
      }
    }
  }

  // Check all patterns if no provider or no match yet
  for (const patterns of Object.values(EMBEDDING_MODEL_PATTERNS)) {
    for (const pattern of patterns) {
      if (pattern.test(lowerModelId)) return true
    }
  }

  return false
}

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

export function EmbeddingsPanel({ openaiClient, isReady, mode, selectedProvider, models }: EmbeddingsPanelProps) {
  const [text, setText] = useState("")
  const [selectedModel, setSelectedModel] = useState<string>("")
  const [isGenerating, setIsGenerating] = useState(false)
  const [results, setResults] = useState<EmbeddingResult[]>([])
  const [error, setError] = useState<string | null>(null)
  const [copiedId, setCopiedId] = useState<string | null>(null)

  // Comparison mode
  const [compareMode, setCompareMode] = useState(false)
  const [selectedForCompare, setSelectedForCompare] = useState<string[]>([])

  // Filter to embedding-capable models
  const embeddingModels = useMemo(() => {
    return models.filter(m => isEmbeddingModel(m.id, mode === "direct" ? selectedProvider : undefined))
  }, [models, mode, selectedProvider])

  // Auto-select first embedding model when available
  useMemo(() => {
    if (!selectedModel && embeddingModels.length > 0) {
      setSelectedModel(embeddingModels[0].id)
    }
  }, [embeddingModels, selectedModel])

  // Get effective model string (with provider prefix for direct mode)
  const getEffectiveModel = useCallback(() => {
    if (mode === "direct" && selectedProvider && selectedModel) {
      return `${selectedProvider}/${selectedModel}`
    }
    return selectedModel
  }, [mode, selectedProvider, selectedModel])

  const handleGenerate = useCallback(async () => {
    if (!openaiClient || !text.trim() || !selectedModel) return

    setIsGenerating(true)
    setError(null)

    try {
      const effectiveModel = getEffectiveModel()
      const response = await openaiClient.embeddings.create({
        model: effectiveModel,
        input: text.trim(),
      })

      const embeddingData = response.data[0]
      const result: EmbeddingResult = {
        id: crypto.randomUUID(),
        text: text.trim(),
        model: selectedModel,
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
  }, [openaiClient, text, selectedModel, getEffectiveModel])

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
          {/* Model Selector */}
          <div className="space-y-2">
            <Label>Embedding Model</Label>
            <Select value={selectedModel} onValueChange={setSelectedModel}>
              <SelectTrigger>
                <SelectValue placeholder={embeddingModels.length === 0 ? "No embedding models" : "Select model"} />
              </SelectTrigger>
              <SelectContent>
                {embeddingModels.map((m) => (
                  <SelectItem key={m.id} value={m.id}>
                    {m.id}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            {embeddingModels.length === 0 && (
              <p className="text-xs text-muted-foreground">
                No embedding models found for current provider
              </p>
            )}
          </div>

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

          {/* Generate Button */}
          <Button
            onClick={handleGenerate}
            disabled={!isReady || !text.trim() || !selectedModel || isGenerating}
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
