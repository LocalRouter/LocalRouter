/**
 * RateLimitEditor Component
 *
 * Allows configuring rate limits for a strategy.
 * Supports: requests, input tokens, output tokens, total tokens, and cost limits.
 */

import { useState } from "react"
import { Plus, Trash2 } from "lucide-react"
import { Button } from "@/components/ui/Button"
import { Input } from "@/components/ui/Input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/Select"

export interface StrategyRateLimit {
  limit_type: "requests" | "input_tokens" | "output_tokens" | "total_tokens" | "cost"
  value: number
  time_window: "minute" | "hour" | "day"
}

interface RateLimitEditorProps {
  limits: StrategyRateLimit[]
  onChange: (limits: StrategyRateLimit[]) => void
  disabled?: boolean
}

const LIMIT_TYPE_OPTIONS = [
  { value: "requests", label: "Requests" },
  { value: "input_tokens", label: "Input Tokens" },
  { value: "output_tokens", label: "Output Tokens" },
  { value: "total_tokens", label: "Total Tokens" },
  { value: "cost", label: "Cost (USD)" },
]

const TIME_WINDOW_OPTIONS = [
  { value: "minute", label: "Per Minute" },
  { value: "hour", label: "Per Hour" },
  { value: "day", label: "Per Day" },
]

export default function RateLimitEditor({
  limits,
  onChange,
  disabled = false,
}: RateLimitEditorProps) {
  const [showAdd, setShowAdd] = useState(false)
  const [newLimit, setNewLimit] = useState<StrategyRateLimit>({
    limit_type: "requests",
    value: 100,
    time_window: "hour",
  })

  const handleAdd = () => {
    onChange([...limits, { ...newLimit }])
    setShowAdd(false)
    setNewLimit({
      limit_type: "requests",
      value: 100,
      time_window: "hour",
    })
  }

  const handleRemove = (index: number) => {
    onChange(limits.filter((_, i) => i !== index))
  }

  const handleUpdate = (
    index: number,
    field: keyof StrategyRateLimit,
    value: string | number
  ) => {
    const updated = [...limits]
    updated[index] = { ...updated[index], [field]: value }
    onChange(updated)
  }

  const getStep = (limitType: string) => {
    if (limitType === "cost") return "0.01"
    if (limitType === "requests") return "1"
    return "100"
  }

  return (
    <div className="space-y-3">
      {limits.map((limit, index) => (
        <div
          key={index}
          className="flex items-end gap-3 p-3 rounded-lg border bg-card"
        >
          <div className="flex-1 grid grid-cols-3 gap-3">
            <div className="space-y-1.5">
              <label className="text-xs text-muted-foreground">Type</label>
              <Select
                value={limit.limit_type}
                onValueChange={(value) => handleUpdate(index, "limit_type", value)}
                disabled={disabled}
              >
                <SelectTrigger className="h-9">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {LIMIT_TYPE_OPTIONS.map((opt) => (
                    <SelectItem key={opt.value} value={opt.value}>
                      {opt.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-1.5">
              <label className="text-xs text-muted-foreground">Limit</label>
              <Input
                type="number"
                value={limit.value}
                onChange={(e) =>
                  handleUpdate(index, "value", parseFloat(e.target.value) || 0)
                }
                disabled={disabled}
                min="0"
                step={getStep(limit.limit_type)}
                className="h-9"
              />
            </div>

            <div className="space-y-1.5">
              <label className="text-xs text-muted-foreground">Window</label>
              <Select
                value={limit.time_window}
                onValueChange={(value) => handleUpdate(index, "time_window", value)}
                disabled={disabled}
              >
                <SelectTrigger className="h-9">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {TIME_WINDOW_OPTIONS.map((opt) => (
                    <SelectItem key={opt.value} value={opt.value}>
                      {opt.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          <Button
            variant="ghost"
            size="icon"
            onClick={() => handleRemove(index)}
            disabled={disabled}
            className="h-9 w-9 text-destructive hover:text-destructive hover:bg-destructive/10"
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      ))}

      {showAdd && (
        <div className="p-4 rounded-lg border bg-card space-y-4">
          <h4 className="font-medium text-sm">Add New Rate Limit</h4>

          <div className="grid grid-cols-3 gap-3">
            <div className="space-y-1.5">
              <label className="text-xs text-muted-foreground">Type</label>
              <Select
                value={newLimit.limit_type}
                onValueChange={(value) =>
                  setNewLimit({ ...newLimit, limit_type: value as any })
                }
              >
                <SelectTrigger className="h-9">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {LIMIT_TYPE_OPTIONS.map((opt) => (
                    <SelectItem key={opt.value} value={opt.value}>
                      {opt.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>

            <div className="space-y-1.5">
              <label className="text-xs text-muted-foreground">Limit</label>
              <Input
                type="number"
                value={newLimit.value}
                onChange={(e) =>
                  setNewLimit({
                    ...newLimit,
                    value: parseFloat(e.target.value) || 0,
                  })
                }
                min="0"
                step={getStep(newLimit.limit_type)}
                className="h-9"
              />
            </div>

            <div className="space-y-1.5">
              <label className="text-xs text-muted-foreground">Window</label>
              <Select
                value={newLimit.time_window}
                onValueChange={(value) =>
                  setNewLimit({ ...newLimit, time_window: value as any })
                }
              >
                <SelectTrigger className="h-9">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {TIME_WINDOW_OPTIONS.map((opt) => (
                    <SelectItem key={opt.value} value={opt.value}>
                      {opt.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          <div className="flex gap-2">
            <Button size="sm" onClick={handleAdd}>
              Add Limit
            </Button>
            <Button size="sm" variant="outline" onClick={() => setShowAdd(false)}>
              Cancel
            </Button>
          </div>
        </div>
      )}

      {!showAdd && !disabled && (
        <Button variant="outline" size="sm" onClick={() => setShowAdd(true)}>
          <Plus className="h-4 w-4 mr-1" />
          Add Rate Limit
        </Button>
      )}
    </div>
  )
}
