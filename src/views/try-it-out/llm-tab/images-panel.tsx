import { useState, useCallback } from "react"
import { ImagePlus, Download, RefreshCw, Sparkles } from "lucide-react"
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

interface GeneratedImage {
  id: string
  prompt: string
  revisedPrompt?: string
  url?: string
  b64Json?: string
  model: string
  size: string
  quality?: string
  style?: string
  timestamp: Date
}

interface ImagesPanelProps {
  openaiClient: OpenAI | null
  isReady: boolean
  selectedModel: string
}

type ImageSize = "256x256" | "512x512" | "1024x1024" | "1024x1792" | "1792x1024"
type ImageQuality = "standard" | "hd"
type ImageStyle = "vivid" | "natural"

// Size options by model type
const SIZE_OPTIONS: Record<string, ImageSize[]> = {
  "dall-e-2": ["256x256", "512x512", "1024x1024"],
  "dall-e-3": ["1024x1024", "1024x1792", "1792x1024"],
  default: ["512x512", "1024x1024"],
}

// Get available sizes for a model
function getSizesForModel(modelId: string): ImageSize[] {
  const lowerModel = modelId.toLowerCase()
  if (lowerModel.includes("dall-e-2")) return SIZE_OPTIONS["dall-e-2"]
  if (lowerModel.includes("dall-e-3")) return SIZE_OPTIONS["dall-e-3"]
  return SIZE_OPTIONS["default"]
}

// Check if model supports quality/style options (DALL-E 3 specific)
function supportsQualityAndStyle(modelId: string): boolean {
  return modelId.toLowerCase().includes("dall-e-3")
}

export function ImagesPanel({ openaiClient, isReady, selectedModel }: ImagesPanelProps) {
  const [prompt, setPrompt] = useState("")
  const [size, setSize] = useState<ImageSize>("1024x1024")
  const [quality, setQuality] = useState<ImageQuality>("standard")
  const [style, setStyle] = useState<ImageStyle>("vivid")
  const [isGenerating, setIsGenerating] = useState(false)
  const [images, setImages] = useState<GeneratedImage[]>([])
  const [error, setError] = useState<string | null>(null)

  const handleGenerate = useCallback(async () => {
    if (!openaiClient || !prompt.trim() || !selectedModel) return

    setIsGenerating(true)
    setError(null)

    try {
      const hasQualityStyle = supportsQualityAndStyle(selectedModel)

      const response = await openaiClient.images.generate({
        model: selectedModel,
        prompt: prompt.trim(),
        n: 1,
        size,
        quality: hasQualityStyle ? quality : undefined,
        style: hasQualityStyle ? style : undefined,
        response_format: "b64_json",
      })

      if (!response.data || response.data.length === 0) {
        throw new Error("No image data returned")
      }
      const imageData = response.data[0]
      const generatedImage: GeneratedImage = {
        id: crypto.randomUUID(),
        prompt: prompt.trim(),
        revisedPrompt: imageData.revised_prompt,
        b64Json: imageData.b64_json,
        url: imageData.url,
        model: selectedModel,
        size,
        quality: hasQualityStyle ? quality : undefined,
        style: hasQualityStyle ? style : undefined,
        timestamp: new Date(),
      }

      setImages((prev) => [generatedImage, ...prev])
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to generate image")
    } finally {
      setIsGenerating(false)
    }
  }, [openaiClient, prompt, selectedModel, size, quality, style])

  const handleDownload = (image: GeneratedImage) => {
    if (!image.b64Json) return

    const link = document.createElement("a")
    link.href = `data:image/png;base64,${image.b64Json}`
    link.download = `image-${image.id.slice(0, 8)}.png`
    link.click()
  }

  const availableSizes = getSizesForModel(selectedModel)
  const showQualityStyle = supportsQualityAndStyle(selectedModel)

  return (
    <div className="flex flex-col h-full gap-4">
      {/* Generation Controls */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-base flex items-center gap-2">
            <Sparkles className="h-4 w-4" />
            Image Generation
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          {/* Prompt */}
          <div className="space-y-2">
            <Label>Prompt</Label>
            <Textarea
              placeholder="A futuristic cityscape at sunset with flying cars..."
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              rows={3}
              disabled={isGenerating}
            />
          </div>

          {/* Generation Options */}
          <div className="grid grid-cols-3 gap-4">
            <div className="space-y-2">
              <Label>Size</Label>
              <Select value={size} onValueChange={(v: string) => setSize(v as ImageSize)}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {availableSizes.map((s) => (
                    <SelectItem key={s} value={s}>
                      {s}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            {showQualityStyle && (
              <>
                <div className="space-y-2">
                  <Label>Quality</Label>
                  <Select value={quality} onValueChange={(v: string) => setQuality(v as ImageQuality)}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="standard">Standard</SelectItem>
                      <SelectItem value="hd">HD</SelectItem>
                    </SelectContent>
                  </Select>
                </div>

                <div className="space-y-2">
                  <Label>Style</Label>
                  <Select value={style} onValueChange={(v: string) => setStyle(v as ImageStyle)}>
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="vivid">Vivid</SelectItem>
                      <SelectItem value="natural">Natural</SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </>
            )}
          </div>

          {error && (
            <div className="p-3 bg-destructive/10 text-destructive rounded-md text-sm">
              {error}
            </div>
          )}

          <Button
            onClick={handleGenerate}
            disabled={!isReady || !prompt.trim() || !selectedModel || isGenerating}
            className="w-full"
          >
            {isGenerating ? (
              <>
                <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                Generating...
              </>
            ) : (
              <>
                <ImagePlus className="h-4 w-4 mr-2" />
                Generate Image
              </>
            )}
          </Button>
        </CardContent>
      </Card>

      {/* Generated Images */}
      <Card className="flex-1 min-h-0">
        <CardHeader className="pb-3">
          <CardTitle className="text-base">Generated Images ({images.length})</CardTitle>
        </CardHeader>
        <CardContent className="h-[calc(100%-4rem)]">
          <ScrollArea className="h-full">
            {images.length === 0 ? (
              <div className="flex items-center justify-center h-64 text-muted-foreground">
                <p className="text-sm">Generated images will appear here</p>
              </div>
            ) : (
              <div className="grid grid-cols-2 gap-4">
                {images.map((image) => (
                  <div key={image.id} className="border rounded-lg overflow-hidden">
                    {image.b64Json && (
                      <img
                        src={`data:image/png;base64,${image.b64Json}`}
                        alt={image.prompt}
                        className="w-full aspect-square object-cover"
                      />
                    )}
                    <div className="p-3 space-y-2">
                      <p className="text-sm line-clamp-2">{image.prompt}</p>
                      {image.revisedPrompt && image.revisedPrompt !== image.prompt && (
                        <p className="text-xs text-muted-foreground line-clamp-2">
                          Revised: {image.revisedPrompt}
                        </p>
                      )}
                      <div className="flex items-center justify-between">
                        <div className="flex gap-1">
                          <Badge variant="outline" className="text-xs">
                            {image.model}
                          </Badge>
                          <Badge variant="secondary" className="text-xs">
                            {image.size}
                          </Badge>
                        </div>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => handleDownload(image)}
                        >
                          <Download className="h-4 w-4" />
                        </Button>
                      </div>
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
