import { useEffect, useMemo, useRef, useState } from "react"
import { invoke } from "@tauri-apps/api/core"
import {
  AlertCircle,
  ArrowDown,
  ArrowUp,
  Braces,
  ChevronDown,
  Loader2,
  Plus,
  Search,
  Sparkles,
  Trash2,
  X,
} from "lucide-react"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import { Checkbox } from "@/components/ui/checkbox"
import { Input } from "@/components/ui/Input"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/Select"
import { Switch } from "@/components/ui/Toggle"
import { cn } from "@/lib/utils"
import type {
  FieldOp,
  FreeModelCondition,
  FreeModelRule,
  FreeModelRuleError,
  FreeModelRulesPreview,
  ModelIdPatternSyntax,
  PreviewFreeModelRulesParams,
  PriceSource,
} from "@/types/tauri-commands"
import { MetadataInspector } from "./MetadataInspector"
import {
  CONDITION_HELP,
  CONDITION_LABELS,
  type ConditionType,
  FIELD_OPS,
  type FreeModelsOnly,
  type ModelChoice,
  RULE_PRESETS,
  modelChoice,
  newCondition,
  newRule,
  opNeedsValue,
  parseValue,
  setModelChoice,
  valueToText,
} from "./rules"

interface FreeModelRulesEditorProps {
  instanceName: string
  value: FreeModelsOnly
  onChange: (next: FreeModelsOnly) => void
  /** The value has unsaved changes */
  dirty?: boolean
  /** Called with the number of rule problems the preview found */
  onErrorCount?: (count: number) => void
}

/** Rows rendered in the preview at most (filter to see others). */
const PREVIEW_ROW_LIMIT = 200

/**
 * Edits which models of a "free models only" provider are free: the
 * always/never-free lists and ordered rules, with a live preview of every
 * model and why it is or is not free.
 */
export function FreeModelRulesEditor({ instanceName, value, onChange, dirty = false, onErrorCount }: FreeModelRulesEditorProps) {
  const rules = value.rules ?? []
  const [preview, setPreview] = useState<FreeModelRulesPreview | null>(null)
  const [previewError, setPreviewError] = useState<string | null>(null)
  const [previewLoading, setPreviewLoading] = useState(false)
  const [inspecting, setInspecting] = useState<string | null>(null)
  const requestId = useRef(0)

  // Debounced live preview of the draft.
  useEffect(() => {
    const id = ++requestId.current
    setPreviewLoading(true)
    const timer = setTimeout(() => {
      invoke<FreeModelRulesPreview>("preview_free_model_rules", {
        instanceName,
        freeTier: value,
      } satisfies PreviewFreeModelRulesParams)
        .then((p) => {
          if (id !== requestId.current) return
          setPreview(p)
          setPreviewError(null)
          onErrorCount?.(p.errors.length)
        })
        .catch((e) => {
          if (id !== requestId.current) return
          setPreviewError(String(e))
        })
        .finally(() => {
          if (id === requestId.current) setPreviewLoading(false)
        })
    }, 300)
    return () => clearTimeout(timer)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [instanceName, value])

  const errorsFor = (index: number | null, listField?: string) =>
    (preview?.errors ?? []).filter(
      (e) => e.rule_index === index && (listField == null || e.field === listField),
    )

  const setRules = (next: FreeModelRule[]) => onChange({ ...value, rules: next })
  const updateRule = (index: number, rule: FreeModelRule) =>
    setRules(rules.map((r, i) => (i === index ? rule : r)))
  const moveRule = (index: number, delta: number) => {
    const target = index + delta
    if (target < 0 || target >= rules.length) return
    const next = [...rules]
    ;[next[index], next[target]] = [next[target], next[index]]
    setRules(next)
  }
  const addRule = (rule: FreeModelRule) => setRules([...rules, rule])

  return (
    <div className="space-y-5">
      {/* Lists */}
      <div className="grid gap-4 md:grid-cols-2">
        <PatternList
          title="Always free"
          help="Model IDs or globs (* matches anything). Checked before the rules."
          patterns={value.free_model_patterns}
          errors={errorsFor(null, "free_model_patterns")}
          onChange={(free_model_patterns) => onChange({ ...value, free_model_patterns })}
        />
        <PatternList
          title="Never free"
          help="Checked first: these are never free, whatever the rules say."
          patterns={value.excluded_models ?? []}
          errors={errorsFor(null, "excluded_models")}
          onChange={(excluded_models) => onChange({ ...value, excluded_models })}
        />
      </div>

      {/* Rules */}
      <div className="space-y-2">
        <div className="flex items-center justify-between gap-2">
          <div>
            <h4 className="text-sm font-medium">Rules</h4>
            <p className="text-xs text-muted-foreground">
              For models not on a list, the first enabled rule that matches decides. No match: not free.
            </p>
          </div>
          <AddRuleMenu onAdd={addRule} />
        </div>
        {rules.length === 0 ? (
          <p className="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
            No rules. Only models on the "Always free" list are free.
          </p>
        ) : (
          <ol className="space-y-2">
            {rules.map((rule, i) => (
              <RuleCard
                key={i}
                index={i}
                count={rules.length}
                rule={rule}
                errors={errorsFor(i)}
                matches={preview?.rule_matches[i]}
                onChange={(r) => updateRule(i, r)}
                onMove={(d) => moveRule(i, d)}
                onRemove={() => setRules(rules.filter((_, j) => j !== i))}
              />
            ))}
          </ol>
        )}
      </div>

      {/* Preview */}
      <PreviewTable
        preview={preview}
        loading={previewLoading}
        error={previewError}
        value={value}
        dirty={dirty}
        onChoice={(modelId, choice) => onChange(setModelChoice(value, modelId, choice))}
        onInspect={setInspecting}
      />

      <MetadataInspector
        instanceName={instanceName}
        modelId={inspecting}
        onClose={() => setInspecting(null)}
        onUseField={(pointer, v) =>
          addRule({
            enabled: true,
            effect: "include",
            when: {
              type: "field",
              pointer,
              op: v === null ? "not_exists" : "equals",
              value: v === null ? undefined : v,
            },
          })
        }
      />
    </div>
  )
}

function AddRuleMenu({ onAdd }: { onAdd: (rule: FreeModelRule) => void }) {
  const types: ConditionType[] = ["model_id", "field", "json_path", "zero_price"]
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="outline" size="sm" className="h-8 shrink-0">
          <Plus className="h-3.5 w-3.5 mr-1" />
          Add rule
          <ChevronDown className="h-3.5 w-3.5 ml-1" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-80">
        <DropdownMenuLabel className="text-xs">Rule type</DropdownMenuLabel>
        {types.map((t) => (
          <DropdownMenuItem key={t} onSelect={() => onAdd(newRule(t))} className="flex-col items-start gap-0.5">
            <span className="text-sm">{CONDITION_LABELS[t]}</span>
            <span className="text-[11px] text-muted-foreground whitespace-normal">{CONDITION_HELP[t]}</span>
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuLabel className="text-xs flex items-center gap-1">
          <Sparkles className="h-3 w-3" />
          Presets
        </DropdownMenuLabel>
        {RULE_PRESETS.map((p) => (
          <DropdownMenuItem
            key={p.id}
            onSelect={() => onAdd(structuredClone(p.rule))}
            className="flex-col items-start gap-0.5"
          >
            <span className="text-sm">{p.label}</span>
            <span className="text-[11px] text-muted-foreground">{p.description}</span>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function PatternList({
  title,
  help,
  patterns,
  errors,
  onChange,
}: {
  title: string
  help: string
  patterns: string[]
  errors: FreeModelRuleError[]
  onChange: (next: string[]) => void
}) {
  const [text, setText] = useState("")
  const add = () => {
    const p = text.trim()
    if (!p) return
    if (!patterns.includes(p)) onChange([...patterns, p])
    setText("")
  }
  return (
    <div className="space-y-1.5">
      <label className="text-xs font-medium">{title}</label>
      <div className="flex gap-1.5">
        <Input
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault()
              add()
            }
          }}
          placeholder="e.g. meta-llama/*:free"
          className="h-8 text-xs font-mono"
          aria-label={`Add to ${title}`}
        />
        <Button type="button" variant="outline" size="sm" className="h-8" onClick={add} disabled={!text.trim()}>
          Add
        </Button>
      </div>
      {patterns.length > 0 && (
        <div className="flex flex-wrap gap-1">
          {patterns.map((p) => (
            <span
              key={p}
              className="inline-flex max-w-full items-center gap-1 rounded bg-muted px-1.5 py-0.5 font-mono text-[11px]"
            >
              <span className="truncate">{p}</span>
              <button
                type="button"
                className="text-muted-foreground hover:text-foreground"
                onClick={() => onChange(patterns.filter((x) => x !== p))}
                aria-label={`Remove ${p}`}
              >
                <X className="h-3 w-3" />
              </button>
            </span>
          ))}
        </div>
      )}
      <p className="text-[11px] text-muted-foreground">{help}</p>
      {errors.map((e, i) => (
        <p key={i} className="text-[11px] text-red-600 dark:text-red-400">{e.message}</p>
      ))}
    </div>
  )
}

function RuleCard({
  index,
  count,
  rule,
  errors,
  matches,
  onChange,
  onMove,
  onRemove,
}: {
  index: number
  count: number
  rule: FreeModelRule
  errors: FreeModelRuleError[]
  matches: number | undefined
  onChange: (rule: FreeModelRule) => void
  onMove: (delta: number) => void
  onRemove: () => void
}) {
  const enabled = rule.enabled ?? true
  const effect = rule.effect ?? "include"
  const fieldError = (field: string) => errors.find((e) => e.field === field)
  const setWhen = (when: FreeModelCondition) => onChange({ ...rule, when })
  const hasError = errors.length > 0

  return (
    <li
      className={cn(
        "rounded-md border p-3 space-y-2.5",
        !enabled && "opacity-60",
        hasError && "border-destructive/60",
      )}
    >
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-xs font-medium text-muted-foreground w-5">{index + 1}.</span>
        <Switch
          checked={enabled}
          onCheckedChange={(on) => onChange({ ...rule, enabled: on })}
          aria-label={`Rule ${index + 1} enabled`}
        />
        <Select
          value={effect === "exclude" ? "exclude" : "include"}
          onValueChange={(v) => onChange({ ...rule, effect: v as "include" | "exclude" })}
        >
          <SelectTrigger className="h-8 w-[118px] text-xs" aria-label="Effect">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="include" className="text-xs">Free if</SelectItem>
            <SelectItem value="exclude" className="text-xs">Not free if</SelectItem>
          </SelectContent>
        </Select>
        <Select
          value={rule.when.type}
          onValueChange={(t) => setWhen(newCondition(t as ConditionType))}
        >
          <SelectTrigger className="h-8 w-[150px] text-xs" aria-label="Rule type">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {(["model_id", "field", "json_path", "zero_price"] as ConditionType[]).map((t) => (
              <SelectItem key={t} value={t} className="text-xs">{CONDITION_LABELS[t]}</SelectItem>
            ))}
            {rule.when.type === "unsupported" && (
              <SelectItem value="unsupported" className="text-xs" disabled>
                {CONDITION_LABELS.unsupported}
              </SelectItem>
            )}
          </SelectContent>
        </Select>
        <Input
          value={rule.label ?? ""}
          onChange={(e) => onChange({ ...rule, label: e.target.value || null })}
          placeholder="Note (optional)"
          className="h-8 min-w-[120px] flex-1 text-xs"
          aria-label="Note"
        />
        {matches != null && (
          <Badge variant="secondary" className="text-[10px] whitespace-nowrap" title="Models whose data this rule's condition matches">
            {matches} {matches === 1 ? "match" : "matches"}
          </Badge>
        )}
        <div className="ml-auto flex items-center">
          <Button variant="ghost" size="icon" className="h-7 w-7" disabled={index === 0} onClick={() => onMove(-1)} aria-label="Move up">
            <ArrowUp className="h-3.5 w-3.5" />
          </Button>
          <Button variant="ghost" size="icon" className="h-7 w-7" disabled={index === count - 1} onClick={() => onMove(1)} aria-label="Move down">
            <ArrowDown className="h-3.5 w-3.5" />
          </Button>
          <Button variant="ghost" size="icon" className="h-7 w-7" onClick={onRemove} aria-label="Remove rule">
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>

      <ConditionInputs when={rule.when} onChange={setWhen} fieldError={fieldError} />

      <p className="text-[11px] text-muted-foreground">{CONDITION_HELP[rule.when.type]}</p>
      {errors
        .filter((e) => !["pattern", "pointer", "value", "expression", "input_pointer", "output_pointer"].includes(e.field))
        .map((e, i) => (
          <p key={i} className="flex items-center gap-1 text-xs text-red-600 dark:text-red-400">
            <AlertCircle className="h-3.5 w-3.5 shrink-0" />
            {e.message}
          </p>
        ))}
    </li>
  )
}

function FieldInput({
  error,
  className,
  ...props
}: React.ComponentProps<typeof Input> & { error?: FreeModelRuleError }) {
  return (
    <div className={cn("space-y-1", className)}>
      <Input
        {...props}
        className={cn("h-8 text-xs font-mono", error && "border-destructive focus-visible:ring-destructive")}
        aria-invalid={error ? true : undefined}
      />
      {error && (
        <p className="flex items-start gap-1 text-[11px] text-red-600 dark:text-red-400">
          <AlertCircle className="h-3 w-3 mt-0.5 shrink-0" />
          <span className="break-words">{error.message}</span>
        </p>
      )}
    </div>
  )
}

function ConditionInputs({
  when,
  onChange,
  fieldError,
}: {
  when: FreeModelCondition
  onChange: (when: FreeModelCondition) => void
  fieldError: (field: string) => FreeModelRuleError | undefined
}) {
  switch (when.type) {
    case "model_id":
      return (
        <div className="flex flex-wrap items-start gap-2">
          <Select
            value={when.syntax ?? "glob"}
            onValueChange={(syntax) => onChange({ ...when, syntax: syntax as ModelIdPatternSyntax })}
          >
            <SelectTrigger className="h-8 w-[110px] text-xs" aria-label="Pattern syntax">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="glob" className="text-xs">Glob</SelectItem>
              <SelectItem value="exact" className="text-xs">Exact</SelectItem>
              <SelectItem value="regex" className="text-xs">Regex</SelectItem>
            </SelectContent>
          </Select>
          <FieldInput
            className="min-w-[200px] flex-1"
            value={when.pattern}
            onChange={(e) => onChange({ ...when, pattern: e.target.value })}
            placeholder={when.syntax === "regex" ? "-free$" : "*:free"}
            aria-label="Pattern"
            error={fieldError("pattern")}
          />
          <label className="flex h-8 items-center gap-1.5 text-xs">
            <Checkbox
              checked={when.case_insensitive ?? false}
              onCheckedChange={(c) => onChange({ ...when, case_insensitive: c === true })}
            />
            Any case
          </label>
        </div>
      )
    case "field": {
      const needsValue = opNeedsValue(when.op)
      return (
        <div className="flex flex-wrap items-start gap-2">
          <FieldInput
            className="w-[200px]"
            value={when.pointer}
            onChange={(e) => onChange({ ...when, pointer: e.target.value })}
            placeholder="/tier"
            aria-label="JSON Pointer"
            error={fieldError("pointer")}
          />
          <Select value={when.op} onValueChange={(op) => onChange({ ...when, op: op as FieldOp })}>
            <SelectTrigger className="h-8 w-[140px] text-xs" aria-label="Comparison">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {FIELD_OPS.map((o) => (
                <SelectItem key={o.op} value={o.op} className="text-xs">{o.label}</SelectItem>
              ))}
            </SelectContent>
          </Select>
          {needsValue && (
            <FieldInput
              className="min-w-[140px] flex-1"
              value={valueToText(when.value)}
              onChange={(e) => onChange({ ...when, value: parseValue(e.target.value) })}
              placeholder={when.op === "matches" ? "^free" : "free"}
              aria-label="Value"
              error={fieldError("value")}
            />
          )}
        </div>
      )
    }
    case "json_path":
      return (
        <FieldInput
          value={when.expression}
          onChange={(e) => onChange({ ...when, expression: e.target.value })}
          placeholder="@.tier == 'free' && @.pricing.prompt == '0'"
          aria-label="JSONPath filter"
          error={fieldError("expression")}
        />
      )
    case "zero_price": {
      const sources = when.sources ?? ["pricing_override", "provider_listing"]
      const toggle = (s: PriceSource, on: boolean) => {
        const next = on ? [...sources.filter((x) => x !== s), s] : sources.filter((x) => x !== s)
        // Keep the usual order: the override first.
        next.sort((a, b) => (a === "pricing_override" ? -1 : b === "pricing_override" ? 1 : 0))
        onChange({ ...when, sources: next })
      }
      return (
        <ZeroPriceInputs
          when={when}
          sources={sources}
          onToggle={toggle}
          onChange={onChange}
          fieldError={fieldError}
        />
      )
    }
    default:
      return null
  }
}

function ZeroPriceInputs({
  when,
  sources,
  onToggle,
  onChange,
  fieldError,
}: {
  when: Extract<FreeModelCondition, { type: "zero_price" }>
  sources: PriceSource[]
  onToggle: (s: PriceSource, on: boolean) => void
  onChange: (when: FreeModelCondition) => void
  fieldError: (field: string) => FreeModelRuleError | undefined
}) {
  const [custom, setCustom] = useState(Boolean(when.input_pointer || when.output_pointer))
  const sourcesError = fieldError("sources")
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-center gap-4 text-xs">
        <label className="flex items-center gap-1.5">
          <Checkbox
            checked={sources.includes("pricing_override")}
            onCheckedChange={(c) => onToggle("pricing_override", c === true)}
          />
          Price set on the Models tab
        </label>
        <label className="flex items-center gap-1.5">
          <Checkbox
            checked={sources.includes("provider_listing")}
            onCheckedChange={(c) => onToggle("provider_listing", c === true)}
          />
          Price in the provider's model list
        </label>
        <button
          type="button"
          className="text-muted-foreground underline-offset-2 hover:underline"
          onClick={() => {
            if (custom) onChange({ ...when, input_pointer: null, output_pointer: null })
            setCustom(!custom)
          }}
        >
          {custom ? "Use the usual pricing fields" : "Prices are elsewhere in the list…"}
        </button>
      </div>
      {sourcesError && <p className="text-[11px] text-red-600 dark:text-red-400">{sourcesError.message}</p>}
      {custom && (
        <div className="flex flex-wrap gap-2">
          <FieldInput
            className="min-w-[160px] flex-1"
            value={when.input_pointer ?? ""}
            onChange={(e) => onChange({ ...when, input_pointer: e.target.value || null })}
            placeholder="Input price, e.g. /cost/input"
            aria-label="Input price JSON Pointer"
            error={fieldError("input_pointer")}
          />
          <FieldInput
            className="min-w-[160px] flex-1"
            value={when.output_pointer ?? ""}
            onChange={(e) => onChange({ ...when, output_pointer: e.target.value || null })}
            placeholder="Output price (optional)"
            aria-label="Output price JSON Pointer"
            error={fieldError("output_pointer")}
          />
        </div>
      )}
    </div>
  )
}

type PreviewFilter = "all" | "free" | "paid"

function PreviewTable({
  preview,
  loading,
  error,
  value,
  dirty,
  onChoice,
  onInspect,
}: {
  preview: FreeModelRulesPreview | null
  loading: boolean
  error: string | null
  value: FreeModelsOnly
  dirty: boolean
  onChoice: (modelId: string, choice: ModelChoice) => void
  onInspect: (modelId: string) => void
}) {
  const [query, setQuery] = useState("")
  const [filter, setFilter] = useState<PreviewFilter>("all")
  const rows = useMemo(() => {
    const q = query.trim().toLowerCase()
    return (preview?.models ?? []).filter((m) => {
      if (filter === "free" && !m.free.is_free) return false
      if (filter === "paid" && m.free.is_free) return false
      return !q || m.model_id.toLowerCase().includes(q) || m.name.toLowerCase().includes(q)
    })
  }, [preview, query, filter])
  const total = preview?.models.length ?? 0

  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-end justify-between gap-2">
        <div>
          <h4 className="text-sm font-medium flex items-center gap-2">
            Preview
            {loading && <Loader2 className="h-3.5 w-3.5 animate-spin text-muted-foreground" />}
          </h4>
          <p className="text-xs text-muted-foreground">
            {preview
              ? `${preview.free_count} of ${total} model${total === 1 ? "" : "s"} free${dirty ? " with your unsaved changes" : ""}.`
              : "Checking the provider's models..."}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <div className="relative">
            <Search className="absolute left-2 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Filter models"
              className="h-8 w-48 pl-7 text-xs"
              aria-label="Filter preview"
            />
          </div>
          <Select value={filter} onValueChange={(v) => setFilter(v as PreviewFilter)}>
            <SelectTrigger className="h-8 w-[100px] text-xs" aria-label="Show">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all" className="text-xs">All</SelectItem>
              <SelectItem value="free" className="text-xs">Free</SelectItem>
              <SelectItem value="paid" className="text-xs">Not free</SelectItem>
            </SelectContent>
          </Select>
        </div>
      </div>

      {error && <p className="text-xs text-red-600 dark:text-red-400">{error}</p>}
      {preview?.discovery_error && (
        <p className="text-xs text-yellow-600 dark:text-yellow-500 break-all">
          Listing models failed; the preview uses the last known list. {preview.discovery_error}
        </p>
      )}
      {preview && !preview.metadata_available && total > 0 && (
        <p className="text-xs text-muted-foreground">
          This provider does not share model details, so listing-field, JSONPath and listed-price rules cannot match (they never make a model free).
        </p>
      )}

      <div className="rounded-md border">
        <div className="grid grid-cols-[minmax(0,2fr)_70px_minmax(0,2fr)_128px_32px] items-center gap-2 border-b bg-muted/40 px-3 py-1.5 text-[11px] font-medium text-muted-foreground">
          <span>Model</span>
          <span>Free</span>
          <span>Why</span>
          <span>This model</span>
          <span />
        </div>
        <div className="max-h-[420px] overflow-y-auto">
          {preview == null ? (
            <div className="flex items-center gap-2 p-3 text-xs text-muted-foreground">
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              Loading models...
            </div>
          ) : rows.length === 0 ? (
            <p className="p-3 text-xs text-muted-foreground">
              {total === 0 ? "This provider lists no models yet." : "No models match the filter."}
            </p>
          ) : (
            rows.slice(0, PREVIEW_ROW_LIMIT).map((m) => (
              <div
                key={m.model_id}
                className={cn(
                  "grid grid-cols-[minmax(0,2fr)_70px_minmax(0,2fr)_128px_32px] items-center gap-2 border-b px-3 py-1.5 last:border-b-0",
                  !m.enabled && "opacity-60",
                )}
              >
                <div className="min-w-0">
                  <p className="truncate font-mono text-xs" title={m.model_id}>{m.model_id}</p>
                  {(m.name !== m.model_id || !m.enabled) && (
                    <p className="truncate text-[11px] text-muted-foreground">
                      {m.name !== m.model_id ? m.name : ""}
                      {!m.enabled && (m.name !== m.model_id ? " · hidden" : "Hidden on the Models tab")}
                    </p>
                  )}
                </div>
                <span>
                  {m.free.is_free ? (
                    <Badge variant="success" className="text-[10px] px-1.5">Free</Badge>
                  ) : (
                    <Badge variant="secondary" className="text-[10px] px-1.5 whitespace-nowrap">Not free</Badge>
                  )}
                </span>
                <span className="text-[11px] text-muted-foreground break-words" title={m.free.reason}>
                  {m.free.reason}
                </span>
                <Select value={modelChoice(value, m.model_id)} onValueChange={(c) => onChoice(m.model_id, c as ModelChoice)}>
                  <SelectTrigger className="h-7 text-[11px]" aria-label={`Free status of ${m.model_id}`}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="auto" className="text-xs">Auto (rules)</SelectItem>
                    <SelectItem value="always" className="text-xs">Always free</SelectItem>
                    <SelectItem value="never" className="text-xs">Never free</SelectItem>
                  </SelectContent>
                </Select>
                <Button
                  variant="ghost"
                  size="icon"
                  className="h-7 w-7"
                  disabled={!m.has_metadata}
                  onClick={() => onInspect(m.model_id)}
                  title={m.has_metadata ? "Inspect its model list entry" : "No model list entry"}
                  aria-label={`Inspect ${m.model_id}`}
                >
                  <Braces className="h-3.5 w-3.5" />
                </Button>
              </div>
            ))
          )}
        </div>
        {rows.length > PREVIEW_ROW_LIMIT && (
          <p className="border-t px-3 py-1.5 text-[11px] text-muted-foreground">
            Showing {PREVIEW_ROW_LIMIT} of {rows.length}. Filter to see others.
          </p>
        )}
      </div>
    </div>
  )
}
