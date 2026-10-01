import { useState, useCallback, useEffect, useRef } from "react"
import { ImagePlus, Download, RefreshCw, Sparkles, Wand2, Upload, X, Pencil } from "lucide-react"
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
import { cn } from "@/lib/utils"
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
  /** Produced by an edit of this many input images */
  editedFrom?: number
  timestamp: Date
}

/** An image picked as edit input (or mask), with a preview URL. */
interface InputImage {
  id: string
  file: File
  preview: string
}

interface ImagesPanelProps {
  openaiClient: OpenAI | null
  isReady: boolean
  selectedModel: string
  /** Whether the provider behind the model can edit images (null: unknown) */
  supportsEdits?: boolean | null
}

type Mode = "generate" | "edit"
type ImageQuality = "standard" | "hd"
type ImageStyle = "vivid" | "natural"

/** Keep the source image's size (the server's default for edits). */
const SAME_AS_INPUT = "same"

// Size options by model type
const SIZE_OPTIONS: Record<string, string[]> = {
  "dall-e-2": ["256x256", "512x512", "1024x1024"],
  "dall-e-3": ["1024x1024", "1024x1792", "1792x1024"],
  default: ["512x512", "768x768", "1024x1024", "1024x768", "768x1024", "1536x1024", "1024x1536"],
}

// Get available sizes for a model
function getSizesForModel(modelId: string): string[] {
  const lowerModel = modelId.toLowerCase()
  if (lowerModel.includes("dall-e-2")) return SIZE_OPTIONS["dall-e-2"]
  if (lowerModel.includes("dall-e-3")) return SIZE_OPTIONS["dall-e-3"]
  return SIZE_OPTIONS["default"]
}

// Check if model supports quality/style options (DALL-E 3 specific)
function supportsQualityAndStyle(modelId: string): boolean {
  return modelId.toLowerCase().includes("dall-e-3")
}

const ACCEPTED = "image/png,image/jpeg,image/webp"
const MAX_INPUT_IMAGES = 16

function toInput(file: File): InputImage {
  return { id: crypto.randomUUID(), file, preview: URL.createObjectURL(file) }
}

function b64ToFile(b64: string, name: string): File {
  const bytes = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0))
  return new File([bytes], name, { type: "image/png" })
}

export function ImagesPanel({ openaiClient, isReady, selectedModel, supportsEdits }: ImagesPanelProps) {
  const [mode, setMode] = useState<Mode>("generate")
  const [prompt, setPrompt] = useState("")
  const [size, setSize] = useState<string>("1024x1024")
  const [editSize, setEditSize] = useState<string>(SAME_AS_INPUT)
  const [count, setCount] = useState(1)
  const [quality, setQuality] = useState<ImageQuality>("standard")
  const [style, setStyle] = useState<ImageStyle>("vivid")
  const [isGenerating, setIsGenerating] = useState(false)
  const [images, setImages] = useState<GeneratedImage[]>([])
  const [inputs, setInputs] = useState<InputImage[]>([])
  const [mask, setMask] = useState<InputImage | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [dragging, setDragging] = useState(false)
  const fileRef = useRef<HTMLInputElement>(null)
  const maskRef = useRef<HTMLInputElement>(null)

  const editsUnavailable = supportsEdits === false

  // Fall back to generation when the provider cannot edit.
  useEffect(() => {
    if (editsUnavailable && mode === "edit") setMode("generate")
  }, [editsUnavailable, mode])

  // Release preview URLs of removed inputs.
  const inputsRef = useRef(inputs)
  inputsRef.current = inputs
  const maskStateRef = useRef(mask)
  maskStateRef.current = mask
  useEffect(
    () => () => {
      inputsRef.current.forEach((i) => URL.revokeObjectURL(i.preview))
      if (maskStateRef.current) URL.revokeObjectURL(maskStateRef.current.preview)
    },
    [],
  )

  const addFiles = (files: FileList | File[] | null) => {
    if (!files) return
    const picked = Array.from(files).filter((f) => ACCEPTED.split(",").includes(f.type))
    if (picked.length === 0) return
    // Allocate only retained previews and keep allocation outside the updater:
    // React StrictMode may invoke state updaters twice.
    const added = picked.slice(0, MAX_INPUT_IMAGES - inputsRef.current.length).map(toInput)
    const next = [...inputsRef.current, ...added]
    inputsRef.current = next
    setInputs(next)
    setError(null)
  }

  const removeInput = (id: string) => {
    const gone = inputsRef.current.find((i) => i.id === id)
    if (gone) URL.revokeObjectURL(gone.preview)
    const next = inputsRef.current.filter((i) => i.id !== id)
    inputsRef.current = next
    setInputs(next)
  }

  const setMaskFile = (file: File | null) => {
    if (maskStateRef.current) URL.revokeObjectURL(maskStateRef.current.preview)
    const next = file ? toInput(file) : null
    maskStateRef.current = next
    setMask(next)
  }

  const editFromResult = (image: GeneratedImage) => {
    if (!image.b64Json) return
    if (inputsRef.current.length >= MAX_INPUT_IMAGES) return
    addFiles([b64ToFile(image.b64Json, `image-${image.id.slice(0, 8)}.png`)])
    setMode("edit")
  }

  const handleRun = useCallback(async () => {
    if (!openaiClient || !prompt.trim() || !selectedModel) return
    if (mode === "edit" && inputs.length === 0) return

    setIsGenerating(true)
    setError(null)

    try {
      const hasQualityStyle = mode === "generate" && supportsQualityAndStyle(selectedModel)
      const response =
        mode === "generate"
          ? await openaiClient.images.generate({
              model: selectedModel,
              prompt: prompt.trim(),
              n: count,
              size: size as OpenAI.Images.ImageGenerateParams["size"],
              quality: hasQualityStyle ? quality : undefined,
              style: hasQualityStyle ? style : undefined,
              response_format: "b64_json",
            })
          : await openaiClient.images.edit({
              model: selectedModel,
              prompt: prompt.trim(),
              image: inputs.length === 1 ? inputs[0].file : inputs.map((i) => i.file),
              mask: mask?.file,
              n: count,
              size:
                editSize === SAME_AS_INPUT
                  ? undefined
                  : (editSize as OpenAI.Images.ImageEditParams["size"]),
              response_format: "b64_json",
            })

      if (!response.data || response.data.length === 0) {
        throw new Error("No image data returned")
      }
      const made: GeneratedImage[] = response.data.map((imageData) => ({
        id: crypto.randomUUID(),
        prompt: prompt.trim(),
        revisedPrompt: imageData.revised_prompt,
        b64Json: imageData.b64_json,
        url: imageData.url,
        model: selectedModel,
        size: mode === "generate" ? size : editSize === SAME_AS_INPUT ? "input size" : editSize,
        quality: hasQualityStyle ? quality : undefined,
        style: hasQualityStyle ? style : undefined,
        editedFrom: mode === "edit" ? inputs.length : undefined,
        timestamp: new Date(),
      }))
      setImages((prev) => [...made, ...prev])
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create the image")
    } finally {
      setIsGenerating(false)
    }
  }, [openaiClient, prompt, selectedModel, size, editSize, count, quality, style, mode, inputs, mask])

  const handleDownload = (image: GeneratedImage) => {
    if (!image.b64Json) return

    const link = document.createElement("a")
    link.href = `data:image/png;base64,${image.b64Json}`
    link.download = `image-${image.id.slice(0, 8)}.png`
    link.click()
  }

  const availableSizes = getSizesForModel(selectedModel)
  const showQualityStyle = mode === "generate" && supportsQualityAndStyle(selectedModel)
  const canRun =
    isReady && !!prompt.trim() && !!selectedModel && !isGenerating && (mode === "generate" || inputs.length > 0)

  return (
    <div className="flex flex-col h-full gap-4">
      <Card>
        <CardHeader className="pb-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <CardTitle className="text-base flex items-center gap-2">
              <Sparkles className="h-4 w-4" />
              Images
            </CardTitle>
            <div className="inline-flex rounded-md border p-0.5" role="tablist" aria-label="Image mode">
              {(["generate", "edit"] as const).map((m) => (
                <button
                  key={m}
                  type="button"
                  role="tab"
                  aria-selected={mode === m}
                  disabled={m === "edit" && editsUnavailable}
                  title={m === "edit" && editsUnavailable ? "This provider cannot edit images" : undefined}
                  onClick={() => setMode(m)}
                  className={cn(
                    "rounded px-3 py-1 text-xs font-medium transition-colors",
                    mode === m ? "bg-primary text-primary-foreground" : "text-muted-foreground hover:text-foreground",
                    m === "edit" && editsUnavailable && "cursor-not-allowed opacity-50 hover:text-muted-foreground",
                  )}
                >
                  {m === "generate" ? "Generate" : "Edit"}
                </button>
              ))}
            </div>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          {mode === "edit" && (
            <div className="space-y-2">
              <Label>Images to edit</Label>
              <div
                className={cn(
                  "rounded-md border border-dashed p-3 transition-colors",
                  dragging && "border-primary bg-primary/5",
                )}
                onDragOver={(e) => {
                  e.preventDefault()
                  setDragging(true)
                }}
                onDragLeave={() => setDragging(false)}
                onDrop={(e) => {
                  e.preventDefault()
                  setDragging(false)
                  addFiles(e.dataTransfer.files)
                }}
              >
                <div className="flex flex-wrap items-center gap-2">
                  {inputs.map((input, i) => (
                    <div key={input.id} className="relative h-20 w-20 overflow-hidden rounded border">
                      <img src={input.preview} alt={`Input ${i + 1}`} className="h-full w-full object-cover" />
                      <button
                        type="button"
                        onClick={() => removeInput(input.id)}
                        className="absolute right-0.5 top-0.5 rounded-full bg-background/80 p-0.5"
                        aria-label={`Remove input ${i + 1}`}
                      >
                        <X className="h-3 w-3" />
                      </button>
                    </div>
                  ))}
                  {inputs.length < MAX_INPUT_IMAGES && (
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => fileRef.current?.click()}
                      disabled={isGenerating}
                    >
                      <Upload className="mr-1 h-4 w-4" />
                      Add images
                    </Button>
                  )}
                </div>
                <p className="mt-2 text-xs text-muted-foreground">
                  PNG, JPEG or WebP. Drop files here or click Add images; several images are used
                  as references in order. You can also use a generated image below.
                </p>
                <input
                  ref={fileRef}
                  type="file"
                  accept={ACCEPTED}
                  multiple
                  className="hidden"
                  onChange={(e) => {
                    addFiles(e.target.files)
                    e.target.value = ""
                  }}
                />
              </div>
              <div className="flex flex-wrap items-center gap-2 text-sm">
                <Label className="text-sm">Mask (optional)</Label>
                {mask ? (
                  <>
                    <img src={mask.preview} alt="Mask" className="h-10 w-10 rounded border object-cover" />
                    <span className="text-xs text-muted-foreground">{mask.file.name}</span>
                    <Button type="button" variant="ghost" size="sm" onClick={() => setMaskFile(null)}>
                      <X className="h-4 w-4" />
                    </Button>
                  </>
                ) : (
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => maskRef.current?.click()}
                    disabled={isGenerating}
                  >
                    Choose mask
                  </Button>
                )}
                <span className="text-xs text-muted-foreground">
                  Transparent areas are edited; without a mask the model edits freely.
                </span>
                <input
                  ref={maskRef}
                  type="file"
                  accept={ACCEPTED}
                  className="hidden"
                  onChange={(e) => {
                    const f = e.target.files?.[0]
                    if (f) setMaskFile(f)
                    e.target.value = ""
                  }}
                />
              </div>
            </div>
          )}

          <div className="space-y-2">
            <Label>{mode === "generate" ? "Prompt" : "What to change"}</Label>
            <Textarea
              placeholder={
                mode === "generate"
                  ? "A futuristic cityscape at sunset with flying cars..."
                  : "Make the sky a dramatic sunset and add a small red boat"
              }
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              rows={3}
              disabled={isGenerating}
            />
          </div>

          <div className="grid grid-cols-2 gap-4 sm:grid-cols-4">
            <div className="space-y-2">
              <Label>Size</Label>
              {mode === "generate" ? (
                <Select value={size} onValueChange={setSize}>
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
              ) : (
                <Select value={editSize} onValueChange={setEditSize}>
                  <SelectTrigger>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value={SAME_AS_INPUT}>Same as input</SelectItem>
                    {availableSizes.map((s) => (
                      <SelectItem key={s} value={s}>
                        {s}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              )}
            </div>

            <div className="space-y-2">
              <Label>Images</Label>
              <Select value={String(count)} onValueChange={(v) => setCount(Number(v))}>
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {[1, 2, 3, 4].map((n) => (
                    <SelectItem key={n} value={String(n)}>
                      {n}
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
            <div className="whitespace-pre-wrap break-words rounded-md bg-destructive/10 p-3 text-sm text-destructive">
              {error}
            </div>
          )}

          <Button onClick={handleRun} disabled={!canRun} className="w-full">
            {isGenerating ? (
              <>
                <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                {mode === "generate" ? "Generating..." : "Editing..."}
              </>
            ) : mode === "generate" ? (
              <>
                <ImagePlus className="h-4 w-4 mr-2" />
                Generate {count > 1 ? `${count} images` : "image"}
              </>
            ) : (
              <>
                <Wand2 className="h-4 w-4 mr-2" />
                {inputs.length === 0 ? "Add an image to edit" : `Edit ${inputs.length > 1 ? `${inputs.length} images` : "image"}`}
              </>
            )}
          </Button>
        </CardContent>
      </Card>

      <Card className="flex-1 min-h-0">
        <CardHeader className="pb-3">
          <CardTitle className="text-base">Results ({images.length})</CardTitle>
        </CardHeader>
        <CardContent className="h-[calc(100%-4rem)]">
          <ScrollArea className="h-full">
            {images.length === 0 ? (
              <div className="flex items-center justify-center h-64 text-muted-foreground">
                <p className="text-sm">Generated and edited images will appear here</p>
              </div>
            ) : (
              <div className="grid grid-cols-2 gap-4">
                {images.map((image) => (
                  <div key={image.id} className="border rounded-lg overflow-hidden">
                    {(image.b64Json || image.url) && (
                      <img
                        src={image.b64Json ? `data:image/png;base64,${image.b64Json}` : image.url}
                        alt={image.prompt}
                        className="w-full aspect-square object-contain bg-muted"
                      />
                    )}
                    <div className="p-3 space-y-2">
                      <p className="text-sm line-clamp-2">{image.prompt}</p>
                      {image.revisedPrompt && image.revisedPrompt !== image.prompt && (
                        <p className="text-xs text-muted-foreground line-clamp-2">
                          Revised: {image.revisedPrompt}
                        </p>
                      )}
                      <div className="flex items-center justify-between gap-2">
                        <div className="flex flex-wrap gap-1">
                          <Badge variant="outline" className="text-xs">
                            {image.model}
                          </Badge>
                          <Badge variant="secondary" className="text-xs">
                            {image.size}
                          </Badge>
                          {image.editedFrom != null && (
                            <Badge variant="secondary" className="text-xs">
                              Edited
                            </Badge>
                          )}
                        </div>
                        <div className="flex">
                          {!editsUnavailable && image.b64Json && (
                            <Button
                              variant="ghost"
                              size="sm"
                              onClick={() => editFromResult(image)}
                              title="Edit this image"
                              aria-label="Edit this image"
                            >
                              <Pencil className="h-4 w-4" />
                            </Button>
                          )}
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => handleDownload(image)}
                            aria-label="Download image"
                          >
                            <Download className="h-4 w-4" />
                          </Button>
                        </div>
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
