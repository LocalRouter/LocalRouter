import { useState, useCallback } from "react"
import {
  Scale,
  RefreshCw,
  Play,
  Plus,
  Trash2,
  ArrowUp,
  ArrowDown,
  Copy,
  Check,
  Braces,
  Clock,
} from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import { Badge } from "@/components/ui/Badge"
import { Switch } from "@/components/ui/Toggle"
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"
import {
  SystemOneAnswerView,
  describeSystemOneOption,
  systemOneBackendDescription,
  systemOneBackendLabel,
} from "@/components/shared/SystemOneAnswers"
import {
  SYSTEMONE_BACKEND_HEADER,
  type SystemOneQuestion,
  type SystemOneQuestionType,
  type SystemOneRequest,
  type SystemOneResponse,
} from "@/types/systemone"
import type OpenAI from "openai"

interface SystemOnePanelProps {
  openaiClient: OpenAI | null
  isReady: boolean
  selectedModel: string
}

interface OptionDraft {
  key: string
  description: string
}

/** Editable form of one question; every type's criteria is kept so switching type is lossless. */
interface QuestionDraft {
  uid: string
  id: string
  type: SystemOneQuestionType
  instructions: string
  options: OptionDraft[]
  levels: string[]
  trueDescription: string
  falseDescription: string
}

interface RunResult {
  request: SystemOneRequest
  response: SystemOneResponse
  backend: string | null
  requestId: string | null
  latencyMs: number
  timestamp: Date
}

const MAX_CHOICE_OPTIONS = 255
const MIN_SCORE_LEVELS = 2
const MAX_SCORE_LEVELS = 10

const DEFAULT_STATE =
  "Hi, I was charged twice for my subscription this month and I need a refund before Friday or I'll dispute it with my bank."

function newQuestion(partial: Partial<QuestionDraft> = {}): QuestionDraft {
  return {
    uid: crypto.randomUUID(),
    id: "",
    type: "choice",
    instructions: "",
    options: [
      { key: "", description: "" },
      { key: "", description: "" },
    ],
    levels: ["", ""],
    trueDescription: "",
    falseDescription: "",
    ...partial,
  }
}

function defaultQuestions(): QuestionDraft[] {
  return [
    newQuestion({
      id: "department",
      type: "choice",
      instructions: "Which team should handle this ticket?",
      options: [
        { key: "billing", description: "Payments, invoices and refunds" },
        { key: "technical", description: "Bugs, outages and how-to questions" },
        { key: "sales", description: "Pricing, plans and upgrades" },
      ],
    }),
    newQuestion({
      id: "urgency",
      type: "score",
      instructions: "How urgent is this ticket?",
      levels: ["Can wait", "Normal", "Needs attention today"],
    }),
    newQuestion({
      id: "refund_requested",
      type: "noul",
      instructions: "Does the customer ask for a refund?",
    }),
  ]
}

/** Text that parses as a JSON object/array is sent structured; anything else is a string. */
function parseState(text: string): unknown {
  const trimmed = text.trim()
  if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
    try {
      const parsed = JSON.parse(trimmed)
      if (parsed !== null && typeof parsed === "object") return parsed
    } catch {
      /* not JSON — send as text */
    }
  }
  return text
}

function stateToText(state: unknown): string {
  if (typeof state === "string") return state
  if (state == null) return ""
  return JSON.stringify(state, null, 2)
}

function questionToWire(q: QuestionDraft): SystemOneQuestion {
  switch (q.type) {
    case "choice":
      return {
        type: "choice",
        instructions: q.instructions,
        criteria: Object.fromEntries(
          q.options.map((o) => [o.key.trim(), o.description.trim() === "" ? null : o.description])
        ),
      }
    case "score":
      return { type: "score", instructions: q.instructions, criteria: [...q.levels] }
    case "noul": {
      const criteria: { true?: string; false?: string } = {}
      if (q.trueDescription.trim()) criteria.true = q.trueDescription
      if (q.falseDescription.trim()) criteria.false = q.falseDescription
      return Object.keys(criteria).length > 0
        ? { type: "noul", instructions: q.instructions, criteria }
        : { type: "noul", instructions: q.instructions }
    }
  }
}

function questionFromWire(id: string, wire: SystemOneQuestion): QuestionDraft {
  const type: SystemOneQuestionType =
    wire?.type === "score" || wire?.type === "noul" ? wire.type : "choice"
  const base = newQuestion({ id, type, instructions: wire?.instructions ?? "" })
  if (!wire) return base
  if (wire.type === "choice" && wire.criteria && typeof wire.criteria === "object") {
    base.options = Object.entries(wire.criteria).map(([key, desc]) => ({
      key,
      description: describeSystemOneOption(desc),
    }))
  } else if (wire.type === "score" && Array.isArray(wire.criteria)) {
    base.levels = wire.criteria.map((l) => String(l))
  } else if (wire.type === "noul") {
    base.trueDescription = wire.criteria?.true ?? ""
    base.falseDescription = wire.criteria?.false ?? ""
  }
  return base
}

/** Returns a human-readable problem with the builder input, or null when it is sendable. */
function validateQuestions(questions: QuestionDraft[]): string | null {
  if (questions.length === 0) return "Add at least one question."
  const seen = new Set<string>()
  for (const q of questions) {
    const id = q.id.trim()
    if (!id) return "Every question needs an id."
    if (seen.has(id)) return `Question id "${id}" is used more than once.`
    seen.add(id)
    if (!q.instructions.trim()) return `Question "${id}" needs instructions.`
    if (q.type === "choice") {
      if (q.options.length < 1 || q.options.length > MAX_CHOICE_OPTIONS) {
        return `Question "${id}" needs 1-${MAX_CHOICE_OPTIONS} options.`
      }
      const keys = new Set<string>()
      for (const o of q.options) {
        const key = o.key.trim()
        if (!key) return `Every option of "${id}" needs a key.`
        if (keys.has(key)) return `Option key "${key}" appears twice in "${id}".`
        keys.add(key)
      }
    } else if (q.type === "score") {
      if (q.levels.length < MIN_SCORE_LEVELS || q.levels.length > MAX_SCORE_LEVELS) {
        return `Question "${id}" needs ${MIN_SCORE_LEVELS}-${MAX_SCORE_LEVELS} levels.`
      }
      if (q.levels.some((l) => !l.trim())) return `Every level of "${id}" needs a description.`
    }
  }
  return null
}

function shellQuote(s: string): string {
  return `'${s.replace(/'/g, `'\\''`)}'`
}

export function SystemOnePanel({ openaiClient, isReady, selectedModel }: SystemOnePanelProps) {
  const [stateText, setStateText] = useState(DEFAULT_STATE)
  const [questions, setQuestions] = useState<QuestionDraft[]>(defaultQuestions)
  const [rawMode, setRawMode] = useState(false)
  const [rawText, setRawText] = useState("")
  const [isRunning, setIsRunning] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<RunResult | null>(null)
  const [copiedCurl, setCopiedCurl] = useState(false)

  const buildBody = useCallback((): SystemOneRequest => {
    const body: SystemOneRequest = {
      state: parseState(stateText),
      questions: Object.fromEntries(questions.map((q) => [q.id.trim(), questionToWire(q)])),
    }
    if (selectedModel) body.model = selectedModel
    return body
  }, [stateText, questions, selectedModel])

  /** The body that Run / copy-as-curl would send, or an error message. */
  const currentBody = useCallback((): { body: SystemOneRequest } | { error: string } => {
    if (rawMode) {
      try {
        const parsed = JSON.parse(rawText)
        if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
          return { error: "The request body must be a JSON object." }
        }
        return { body: parsed as SystemOneRequest }
      } catch (e) {
        return { error: `Invalid JSON: ${e instanceof Error ? e.message : String(e)}` }
      }
    }
    const problem = validateQuestions(questions)
    if (problem) return { error: problem }
    return { body: buildBody() }
  }, [rawMode, rawText, questions, buildBody])

  const handleToggleRaw = (next: boolean) => {
    setError(null)
    if (next) {
      setRawText(JSON.stringify(buildBody(), null, 2))
      setRawMode(true)
      return
    }
    // Leaving raw mode: carry the edited JSON back into the builder.
    try {
      const parsed = JSON.parse(rawText) as Partial<SystemOneRequest>
      if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("The request body must be a JSON object.")
      }
      const wireQuestions = parsed.questions ?? {}
      if (typeof wireQuestions !== "object" || Array.isArray(wireQuestions)) {
        throw new Error("`questions` must be an object keyed by question id.")
      }
      setStateText(stateToText(parsed.state))
      setQuestions(
        Object.entries(wireQuestions).map(([id, q]) => questionFromWire(id, q as SystemOneQuestion))
      )
      setRawMode(false)
    } catch (e) {
      setError(
        `Can't switch back to the builder: ${e instanceof Error ? e.message : String(e)}`
      )
    }
  }

  const handleRun = useCallback(async () => {
    if (!openaiClient) return
    const built = currentBody()
    if ("error" in built) {
      setError(built.error)
      return
    }

    setIsRunning(true)
    setError(null)
    const started = performance.now()
    try {
      const { data, response } = await openaiClient
        .post<SystemOneResponse>("/systemone", { body: built.body, maxRetries: 0 })
        .withResponse()
      setResult({
        request: built.body,
        response: data,
        backend: response.headers.get(SYSTEMONE_BACKEND_HEADER),
        requestId: response.headers.get("x-typesafe-request-id"),
        latencyMs: Math.round(performance.now() - started),
        timestamp: new Date(),
      })
    } catch (err) {
      setError(err instanceof Error ? err.message : "System One request failed")
    } finally {
      setIsRunning(false)
    }
  }, [openaiClient, currentBody])

  const handleCopyCurl = async () => {
    if (!openaiClient) return
    const built = currentBody()
    if ("error" in built) {
      setError(built.error)
      return
    }
    const url = `${openaiClient.baseURL.replace(/\/+$/, "")}/systemone`
    const curl = [
      `curl -sS ${shellQuote(url)}`,
      `  -H ${shellQuote(`Authorization: Bearer ${openaiClient.apiKey}`)}`,
      `  -H 'Content-Type: application/json'`,
      `  -d ${shellQuote(JSON.stringify(built.body, null, 2))}`,
    ].join(" \\\n")
    await navigator.clipboard.writeText(curl)
    setCopiedCurl(true)
    setTimeout(() => setCopiedCurl(false), 2000)
  }

  const updateQuestion = (uid: string, patch: Partial<QuestionDraft>) => {
    setQuestions((prev) => prev.map((q) => (q.uid === uid ? { ...q, ...patch } : q)))
  }

  const removeQuestion = (uid: string) => {
    setQuestions((prev) => prev.filter((q) => q.uid !== uid))
  }

  const addQuestion = () => {
    setQuestions((prev) => [...prev, newQuestion({ id: `q${prev.length + 1}` })])
  }

  const questionsById = result
    ? (result.request.questions ?? {})
    : ({} as Record<string, SystemOneQuestion>)

  return (
    <div className="flex flex-col h-full gap-4">
      <Card>
        <CardHeader className="pb-3">
          <div className="flex items-center justify-between gap-2">
            <CardTitle className="text-base flex items-center gap-2">
              <Scale className="h-4 w-4" />
              System One Decisions
            </CardTitle>
            <div className="flex items-center gap-2">
              <Label htmlFor="systemone-raw" className="text-xs text-muted-foreground flex items-center gap-1">
                <Braces className="h-3.5 w-3.5" />
                Raw JSON
              </Label>
              <Switch id="systemone-raw" checked={rawMode} onCheckedChange={handleToggleRaw} />
            </div>
          </div>
          <p className="text-xs text-muted-foreground">
            Ask typed questions about a state and get calibrated probabilities back instead of text.
            Native System One models answer directly; chat models answer through LocalRouter&apos;s
            translation layer.
          </p>
        </CardHeader>
        <CardContent className="space-y-4">
          {rawMode ? (
            <div className="space-y-2">
              <Label>Request body</Label>
              <Textarea
                value={rawText}
                onChange={(e) => setRawText(e.target.value)}
                rows={16}
                className="font-mono text-xs"
                spellCheck={false}
                disabled={isRunning}
              />
            </div>
          ) : (
            <>
              <div className="space-y-2">
                <Label>State</Label>
                <Textarea
                  placeholder="Text, or a JSON object/array describing the situation to judge"
                  value={stateText}
                  onChange={(e) => setStateText(e.target.value)}
                  rows={4}
                  disabled={isRunning}
                />
                <p className="text-xs text-muted-foreground">
                  Sent as JSON when it parses as an object or array, otherwise as text.
                </p>
              </div>

              <div className="space-y-2">
                <div className="flex items-center justify-between">
                  <Label>Questions ({questions.length})</Label>
                  <Button variant="outline" size="sm" onClick={addQuestion} disabled={isRunning}>
                    <Plus className="h-3.5 w-3.5 mr-1" />
                    Add question
                  </Button>
                </div>
                <div className="space-y-3">
                  {questions.map((q) => (
                    <QuestionEditor
                      key={q.uid}
                      question={q}
                      disabled={isRunning}
                      onChange={(patch) => updateQuestion(q.uid, patch)}
                      onRemove={() => removeQuestion(q.uid)}
                    />
                  ))}
                </div>
              </div>
            </>
          )}

          <div className="flex items-center gap-2">
            <Button
              onClick={handleRun}
              disabled={!isReady || !openaiClient || isRunning}
            >
              {isRunning ? (
                <>
                  <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                  Running...
                </>
              ) : (
                <>
                  <Play className="h-4 w-4 mr-2" />
                  Run
                </>
              )}
            </Button>
            <Button variant="outline" onClick={handleCopyCurl} disabled={!openaiClient}>
              {copiedCurl ? (
                <Check className="h-4 w-4 mr-2 text-green-500" />
              ) : (
                <Copy className="h-4 w-4 mr-2" />
              )}
              Copy as curl
            </Button>
          </div>

          {error && (
            <div className="p-3 bg-destructive/10 text-destructive rounded-md text-sm whitespace-pre-wrap break-words">
              {error}
            </div>
          )}
        </CardContent>
      </Card>

      <Card className="flex-1 min-h-0">
        <CardHeader className="pb-3">
          <div className="flex flex-wrap items-center gap-2">
            <CardTitle className="text-base">Answers</CardTitle>
            {result && (
              <>
                <Badge variant="outline" className="text-xs font-mono">
                  {result.response.model}
                </Badge>
                {result.backend && (
                  <Badge
                    variant={result.backend === "native" ? "success" : "info"}
                    className="text-xs"
                    title={systemOneBackendDescription(result.backend)}
                  >
                    {systemOneBackendLabel(result.backend)}
                  </Badge>
                )}
                <span className="ml-auto flex items-center gap-3 text-xs text-muted-foreground">
                  {result.response.usage && (
                    <span>
                      {result.response.usage.input_tokens ?? "?"} in / {result.response.usage.output_tokens ?? "?"} out tokens
                    </span>
                  )}
                  <span className="flex items-center gap-1">
                    <Clock className="h-3 w-3" />
                    {result.latencyMs}ms
                  </span>
                </span>
              </>
            )}
          </div>
          {result?.requestId && (
            <p className="text-xs text-muted-foreground font-mono">request id {result.requestId}</p>
          )}
        </CardHeader>
        <CardContent className="h-[calc(100%-5rem)]">
          <ScrollArea className="h-full">
            {!result ? (
              <div className="flex items-center justify-center h-40 text-muted-foreground">
                <p className="text-sm">Answers with probability bars will appear here</p>
              </div>
            ) : (
              <div className="space-y-3">
                {Object.entries(result.response.answers ?? {}).map(([id, answer]) => (
                  <SystemOneAnswerView
                    key={id}
                    id={id}
                    answer={answer}
                    question={questionsById[id]}
                  />
                ))}
              </div>
            )}
          </ScrollArea>
        </CardContent>
      </Card>
    </div>
  )
}

function QuestionEditor({
  question: q,
  disabled,
  onChange,
  onRemove,
}: {
  question: QuestionDraft
  disabled: boolean
  onChange: (patch: Partial<QuestionDraft>) => void
  onRemove: () => void
}) {
  const setOption = (index: number, patch: Partial<OptionDraft>) =>
    onChange({ options: q.options.map((o, i) => (i === index ? { ...o, ...patch } : o)) })

  const moveLevel = (index: number, delta: -1 | 1) => {
    const target = index + delta
    if (target < 0 || target >= q.levels.length) return
    const next = [...q.levels]
    ;[next[index], next[target]] = [next[target], next[index]]
    onChange({ levels: next })
  }

  return (
    <div className="border rounded-lg p-3 space-y-3">
      <div className="flex items-center gap-2">
        <Input
          value={q.id}
          onChange={(e) => onChange({ id: e.target.value })}
          placeholder="question_id"
          className="h-8 max-w-[200px] font-mono text-xs"
          disabled={disabled}
          aria-label="Question id"
        />
        <Select
          value={q.type}
          onValueChange={(v) => onChange({ type: v as SystemOneQuestionType })}
          disabled={disabled}
        >
          <SelectTrigger className="h-8 w-[150px]">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="choice">Choice</SelectItem>
            <SelectItem value="score">Score</SelectItem>
            <SelectItem value="noul">Yes / No</SelectItem>
          </SelectContent>
        </Select>
        <Button
          variant="ghost"
          size="icon"
          className="ml-auto h-8 w-8"
          onClick={onRemove}
          disabled={disabled}
          title="Remove question"
        >
          <Trash2 className="h-4 w-4" />
        </Button>
      </div>

      <Input
        value={q.instructions}
        onChange={(e) => onChange({ instructions: e.target.value })}
        placeholder="Instructions, e.g. Which team should handle this ticket?"
        className="h-8 text-sm"
        disabled={disabled}
        aria-label="Instructions"
      />

      {q.type === "choice" && (
        <div className="space-y-1.5">
          <Label className="text-xs text-muted-foreground">Options (key and optional description)</Label>
          {q.options.map((o, i) => (
            <div key={i} className="flex items-center gap-2">
              <Input
                value={o.key}
                onChange={(e) => setOption(i, { key: e.target.value })}
                placeholder="key"
                className="h-8 max-w-[160px] font-mono text-xs"
                disabled={disabled}
                aria-label={`Option ${i + 1} key`}
              />
              <Input
                value={o.description}
                onChange={(e) => setOption(i, { description: e.target.value })}
                placeholder="Description (optional)"
                className="h-8 text-xs"
                disabled={disabled}
                aria-label={`Option ${i + 1} description`}
              />
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 shrink-0"
                onClick={() => onChange({ options: q.options.filter((_, j) => j !== i) })}
                disabled={disabled || q.options.length <= 1}
                title="Remove option"
              >
                <Trash2 className="h-3.5 w-3.5" />
              </Button>
            </div>
          ))}
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onChange({ options: [...q.options, { key: "", description: "" }] })}
            disabled={disabled || q.options.length >= MAX_CHOICE_OPTIONS}
          >
            <Plus className="h-3.5 w-3.5 mr-1" />
            Add option
          </Button>
        </div>
      )}

      {q.type === "score" && (
        <div className="space-y-1.5">
          <Label className="text-xs text-muted-foreground">
            Levels, lowest first ({MIN_SCORE_LEVELS}-{MAX_SCORE_LEVELS})
          </Label>
          {q.levels.map((level, i) => (
            <div key={i} className="flex items-center gap-2">
              <span className="w-5 text-right font-mono text-xs text-muted-foreground">{i}</span>
              <Input
                value={level}
                onChange={(e) =>
                  onChange({ levels: q.levels.map((l, j) => (j === i ? e.target.value : l)) })
                }
                placeholder={`Level ${i} description`}
                className="h-8 text-xs"
                disabled={disabled}
                aria-label={`Level ${i}`}
              />
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 shrink-0"
                onClick={() => moveLevel(i, -1)}
                disabled={disabled || i === 0}
                title="Move up"
              >
                <ArrowUp className="h-3.5 w-3.5" />
              </Button>
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 shrink-0"
                onClick={() => moveLevel(i, 1)}
                disabled={disabled || i === q.levels.length - 1}
                title="Move down"
              >
                <ArrowDown className="h-3.5 w-3.5" />
              </Button>
              <Button
                variant="ghost"
                size="icon"
                className="h-8 w-8 shrink-0"
                onClick={() => onChange({ levels: q.levels.filter((_, j) => j !== i) })}
                disabled={disabled || q.levels.length <= MIN_SCORE_LEVELS}
                title="Remove level"
              >
                <Trash2 className="h-3.5 w-3.5" />
              </Button>
            </div>
          ))}
          <Button
            variant="ghost"
            size="sm"
            onClick={() => onChange({ levels: [...q.levels, ""] })}
            disabled={disabled || q.levels.length >= MAX_SCORE_LEVELS}
          >
            <Plus className="h-3.5 w-3.5 mr-1" />
            Add level
          </Button>
        </div>
      )}

      {q.type === "noul" && (
        <div className="grid grid-cols-2 gap-2">
          <Input
            value={q.trueDescription}
            onChange={(e) => onChange({ trueDescription: e.target.value })}
            placeholder="What “yes” means (optional)"
            className="h-8 text-xs"
            disabled={disabled}
            aria-label="True description"
          />
          <Input
            value={q.falseDescription}
            onChange={(e) => onChange({ falseDescription: e.target.value })}
            placeholder="What “no” means (optional)"
            className="h-8 text-xs"
            disabled={disabled}
            aria-label="False description"
          />
        </div>
      )}
    </div>
  )
}
