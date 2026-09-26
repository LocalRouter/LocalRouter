import { Badge } from "@/components/ui/Badge"
import { cn } from "@/lib/utils"
import type {
  SystemOneAnswer,
  SystemOneBackend,
  SystemOneOptionDescription,
  SystemOneQuestion,
} from "@/types/systemone"

/** Human-readable label for the `x-localrouter-systemone-backend` header. */
export function systemOneBackendLabel(backend: SystemOneBackend | string): string {
  switch (backend) {
    case "native":
      return "Native System One"
    case "letter_logprobs":
      return "Translated (token logprobs)"
    case "json":
      return "Translated (JSON)"
    default:
      return backend
  }
}

/** Tooltip text explaining the backend header value. */
export function systemOneBackendDescription(backend: SystemOneBackend | string): string {
  switch (backend) {
    case "native":
      return "The provider answered the System One request directly."
    case "letter_logprobs":
      return "A chat model answered via LocalRouter's translation layer; probabilities come from the token logprobs of the option letters."
    case "json":
      return "A chat model answered via LocalRouter's translation layer; probabilities were self-reported as JSON."
    default:
      return ""
  }
}

/** Flatten an option description (text, null, or structured) to display text. */
export function describeSystemOneOption(desc: SystemOneOptionDescription | undefined): string {
  if (desc == null) return ""
  if (typeof desc === "string") return desc
  return JSON.stringify(desc)
}

function formatPercent(p: number): string {
  if (!Number.isFinite(p)) return "—"
  const pct = p * 100
  return pct >= 10 || pct === 0 ? `${pct.toFixed(0)}%` : `${pct.toFixed(1)}%`
}

/** One labelled horizontal probability bar. */
export function ProbabilityBar({
  label,
  value,
  highlight,
  title,
}: {
  label: string
  value: number
  highlight?: boolean
  title?: string
}) {
  const width = Math.max(0, Math.min(1, Number.isFinite(value) ? value : 0)) * 100
  return (
    <div className="flex items-center gap-2 text-xs" title={title}>
      <span
        className={cn(
          "w-32 shrink-0 truncate font-mono",
          highlight ? "font-semibold text-foreground" : "text-muted-foreground"
        )}
      >
        {label}
      </span>
      <div className="relative h-2 flex-1 overflow-hidden rounded-full bg-muted">
        <div
          className={cn("h-full rounded-full", highlight ? "bg-primary" : "bg-primary/40")}
          style={{ width: `${width}%` }}
        />
      </div>
      <span className={cn("w-12 shrink-0 text-right font-mono tabular-nums", highlight && "font-semibold")}>
        {formatPercent(value)}
      </span>
    </div>
  )
}

/**
 * Probability entries in the order the question offered them (falling back to
 * the answer's own key order for keys the question doesn't list).
 */
function orderedChoiceEntries(
  probabilities: Record<string, number>,
  question: SystemOneQuestion | undefined
): [string, number][] {
  const keys: string[] = []
  if (question?.type === "choice" && question.criteria && typeof question.criteria === "object") {
    keys.push(...Object.keys(question.criteria))
  }
  for (const k of Object.keys(probabilities)) {
    if (!keys.includes(k)) keys.push(k)
  }
  return keys.map((k) => [k, probabilities[k] ?? 0])
}

const TYPE_LABEL: Record<string, string> = {
  choice: "choice",
  score: "score",
  noul: "yes / no",
}

/** Result card for one System One answer, with probability bars. */
export function SystemOneAnswerView({
  id,
  answer,
  question,
}: {
  id: string
  answer: SystemOneAnswer
  question?: SystemOneQuestion
}) {
  return (
    <div className="space-y-2 rounded-lg border p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-sm font-medium">{id}</span>
        <Badge variant="secondary" className="text-[10px]">
          {TYPE_LABEL[answer.type] ?? answer.type}
        </Badge>
        {answer.type === "choice" && (
          <span className="text-sm">
            → <span className="font-mono font-semibold">{answer.choice}</span>
          </span>
        )}
        {answer.type === "score" && (
          <span className="text-sm">
            score <span className="font-mono font-semibold">{answer.score.toFixed(2)}</span>
          </span>
        )}
        {answer.type === "noul" && (
          <span className="text-sm">
            yes <span className="font-mono font-semibold">{formatPercent(answer.noul)}</span>
          </span>
        )}
        {(answer.type === "choice" || answer.type === "score") && answer.confidence != null && (
          <span className="ml-auto text-xs text-muted-foreground">
            confidence {formatPercent(answer.confidence)}
          </span>
        )}
      </div>
      {question?.instructions && (
        <p className="text-xs text-muted-foreground">{question.instructions}</p>
      )}

      {answer.type === "choice" && (
        <div className="space-y-1">
          {orderedChoiceEntries(answer.probabilities ?? {}, question).map(([key, p]) => (
            <ProbabilityBar
              key={key}
              label={key}
              value={p}
              highlight={key === answer.choice}
              title={
                question?.type === "choice"
                  ? describeSystemOneOption(question.criteria[key]) || undefined
                  : undefined
              }
            />
          ))}
        </div>
      )}

      {answer.type === "score" && (
        <div className="space-y-1">
          {Object.keys(answer.probabilities ?? {})
            .sort((a, b) => Number(a) - Number(b))
            .map((level) => {
              const legend =
                answer.legend?.[level] ??
                (question?.type === "score" ? question.criteria[Number(level)] : undefined)
              const p = answer.probabilities[level] ?? 0
              return (
                <ProbabilityBar
                  key={level}
                  label={legend ? `${level} · ${legend}` : level}
                  value={p}
                  highlight={Math.round(answer.score) === Number(level)}
                  title={legend}
                />
              )
            })}
        </div>
      )}

      {answer.type === "noul" && (
        <div className="space-y-1">
          <ProbabilityBar
            label={
              question?.type === "noul" && question.criteria?.true
                ? `yes · ${question.criteria.true}`
                : "yes"
            }
            value={answer.noul}
            highlight={answer.noul >= 0.5}
          />
          <ProbabilityBar
            label={
              question?.type === "noul" && question.criteria?.false
                ? `no · ${question.criteria.false}`
                : "no"
            }
            value={1 - answer.noul}
            highlight={answer.noul < 0.5}
          />
        </div>
      )}
    </div>
  )
}

/** Read-only view of one System One question (type, instructions, options). */
export function SystemOneQuestionView({ id, question }: { id: string; question: SystemOneQuestion }) {
  return (
    <div className="space-y-1.5 rounded-md border p-2 text-xs">
      <div className="flex items-center gap-2">
        <span className="font-mono font-medium">{id}</span>
        <Badge variant="secondary" className="text-[10px]">
          {TYPE_LABEL[question.type] ?? question.type}
        </Badge>
      </div>
      {question.instructions && <p className="whitespace-pre-wrap">{question.instructions}</p>}
      {question.type === "choice" && question.criteria && (
        <ul className="space-y-0.5">
          {Object.entries(question.criteria).map(([key, desc]) => (
            <li key={key} className="flex gap-2">
              <span className="font-mono text-muted-foreground">{key}</span>
              <span className="min-w-0 break-words">{describeSystemOneOption(desc)}</span>
            </li>
          ))}
        </ul>
      )}
      {question.type === "score" && Array.isArray(question.criteria) && (
        <ol className="space-y-0.5">
          {question.criteria.map((level, i) => (
            <li key={i} className="flex gap-2">
              <span className="font-mono text-muted-foreground">{i}</span>
              <span className="min-w-0 break-words">{level}</span>
            </li>
          ))}
        </ol>
      )}
      {question.type === "noul" && question.criteria && (
        <ul className="space-y-0.5">
          {question.criteria.true && (
            <li className="flex gap-2">
              <span className="font-mono text-muted-foreground">true</span>
              <span className="min-w-0 break-words">{question.criteria.true}</span>
            </li>
          )}
          {question.criteria.false && (
            <li className="flex gap-2">
              <span className="font-mono text-muted-foreground">false</span>
              <span className="min-w-0 break-words">{question.criteria.false}</span>
            </li>
          )}
        </ul>
      )}
    </div>
  )
}
