import { cn } from "@/lib/utils"

/** Normalized tool shape accepted by ToolList */
export interface ToolListItem {
  name: string
  description?: string | null
  inputSchema?: Record<string, unknown> | null
  /** Optional type label shown next to the name (e.g. "tool", "resource", "prompt") */
  itemType?: string | null
}

interface SchemaProperty {
  type?: string
  description?: string
  enum?: string[]
  items?: SchemaProperty
  properties?: Record<string, SchemaProperty>
  required?: string[]
  default?: unknown
  oneOf?: SchemaProperty[]
  anyOf?: SchemaProperty[]
}

interface ToolListProps {
  tools: ToolListItem[]
  /** Compact mode uses smaller text (default: false) */
  compact?: boolean
  className?: string
}

function formatType(schema: SchemaProperty): string {
  if (schema.enum) {
    return schema.enum.map((v) => `"${v}"`).join(" | ")
  }
  if (schema.oneOf || schema.anyOf) {
    const variants = (schema.oneOf || schema.anyOf)!
    return variants.map((v) => v.type || "unknown").join(" | ")
  }
  if (schema.type === "array" && schema.items) {
    return `${formatType(schema.items)}[]`
  }
  return schema.type || "unknown"
}

function SchemaProperties({
  properties,
  required = [],
  compact,
  depth = 0,
}: {
  properties: Record<string, SchemaProperty>
  required?: string[]
  compact?: boolean
  depth?: number
}) {
  const textSize = compact ? "text-[10px]" : "text-xs"

  return (
    <div className={cn("space-y-0.5", depth > 0 && "ml-3 border-l border-border/40 pl-2")}>
      {Object.entries(properties).map(([key, prop]) => {
        const isRequired = required.includes(key)
        const hasNestedProps = prop.type === "object" && prop.properties

        return (
          <div key={key}>
            <div className={cn("font-mono", textSize)}>
              <span className="text-foreground/80">{key}</span>
              <span className="text-muted-foreground/70">{`: ${formatType(prop)}`}</span>
              {isRequired && <span className="text-muted-foreground/70">*</span>}
              {prop.default !== undefined && (
                <span className="text-muted-foreground/60">{` = ${JSON.stringify(prop.default)}`}</span>
              )}
              {prop.description && (
                <span className="text-muted-foreground/60">{` — ${prop.description}`}</span>
              )}
            </div>
            {hasNestedProps && (
              <SchemaProperties
                properties={prop.properties!}
                required={prop.required}
                compact={compact}
                depth={depth + 1}
              />
            )}
          </div>
        )
      })}
    </div>
  )
}

const TYPE_COLORS: Record<string, string> = {
  tool: "text-muted-foreground/60",
  resource: "text-muted-foreground/60",
  prompt: "text-muted-foreground/60",
}

function ToolItem({ tool, compact }: { tool: ToolListItem; compact?: boolean }) {
  const schema = tool.inputSchema as SchemaProperty | null
  const properties = schema?.properties as Record<string, SchemaProperty> | undefined
  const required = Array.isArray(schema?.required) ? (schema!.required as string[]) : []
  const hasParams = properties && Object.keys(properties).length > 0

  return (
    <div className="rounded-md border border-border/50">
      <div className="w-full text-left flex items-center gap-2 px-3 py-2">
        <code className={cn("font-mono font-medium truncate", compact ? "text-[11px]" : "text-xs")}>
          {tool.name}
        </code>
        {tool.itemType && (
          <span className={cn("text-[9px] shrink-0", TYPE_COLORS[tool.itemType] || "text-muted-foreground/60")}>
            ({tool.itemType})
          </span>
        )}
      </div>

      {tool.description && (
        <div className="px-3 pb-2 -mt-1 ml-0">
          <p className={cn("text-muted-foreground", compact ? "text-[10px]" : "text-xs")}>
            {tool.description}
          </p>
        </div>
      )}

      {hasParams && properties && (
        <div className={cn("px-3 pb-3 pt-1 border-t border-border/30")}>
          <p className={cn("font-medium mb-1 text-muted-foreground", compact ? "text-[10px]" : "text-xs")}>
            Parameters
          </p>
          <SchemaProperties
            properties={properties}
            required={required}
            compact={compact}
          />
        </div>
      )}
    </div>
  )
}

export function ToolList({ tools, compact, className }: ToolListProps) {
  if (tools.length === 0) {
    return (
      <p className={cn("text-muted-foreground", compact ? "text-[10px]" : "text-xs")}>
        No tools available.
      </p>
    )
  }

  return (
    <div className={cn("space-y-1", className)}>
      {tools.map((tool) => (
        <ToolItem key={tool.name} tool={tool} compact={compact} />
      ))}
    </div>
  )
}
