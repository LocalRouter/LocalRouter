import { useEffect, useState } from "react"
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/Card"
import { Input } from "@/components/ui/Input"
import type { PushRelayConfig, UpdatePushRelayConfigParams } from "@/types/tauri-commands"

/** Defaults and bounds; must match `crates/lr-config/src/push_relay.rs`. */
const SETTINGS = {
  channelMessagesPerMinute: { min: 1, max: 600, def: 10 },
  historyRetentionHours: { min: 1, max: 720, def: 24 },
  historyMaxEvents: { min: 100, max: 1_000_000, def: 10_000 },
} as const

interface NumberSettingProps {
  id: string
  label: string
  unit: string
  description: string
  value: number
  bounds: { min: number; max: number; def: number }
  disabled: boolean
  onCommit: (value: number) => void
}

function NumberSetting({ id, label, unit, description, value, bounds, disabled, onCommit }: NumberSettingProps) {
  const [draft, setDraft] = useState(String(value))
  useEffect(() => setDraft(String(value)), [value])

  const parsed = Number(draft)
  const valid = draft.trim() !== "" && Number.isInteger(parsed) && parsed >= bounds.min && parsed <= bounds.max

  const commit = () => {
    if (valid && parsed !== value) onCommit(parsed)
    else if (!valid) setDraft(String(value))
  }

  return (
    <div className="flex items-start justify-between gap-4 border-t pt-3 first:border-t-0 first:pt-0">
      <div className="min-w-0">
        <label htmlFor={id} className="text-sm font-medium">{label}</label>
        <p className="text-xs text-muted-foreground mt-0.5">
          {description} Default {bounds.def.toLocaleString()}; {bounds.min.toLocaleString()}–{bounds.max.toLocaleString()}.
        </p>
      </div>
      <div className="flex items-center gap-2 shrink-0">
        <Input
          id={id}
          type="number"
          min={bounds.min}
          max={bounds.max}
          step={1}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && (e.currentTarget as HTMLInputElement).blur()}
          disabled={disabled}
          aria-invalid={!valid}
          className={valid ? "h-8 w-28 text-right" : "h-8 w-28 text-right border-destructive"}
        />
        <span className="text-xs text-muted-foreground w-24">{unit}</span>
      </div>
    </div>
  )
}

interface RelaySettingsCardProps {
  config: PushRelayConfig
  disabled: boolean
  onUpdate: (params: UpdatePushRelayConfigParams) => void
}

/** The Claude Channels rate and the per-client event history. */
export function RelaySettingsCard({ config, disabled, onUpdate }: RelaySettingsCardProps) {
  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base">Settings</CardTitle>
        <CardDescription>Apply to every client as soon as they are saved.</CardDescription>
      </CardHeader>
      <CardContent className="pt-0 space-y-3">
        <NumberSetting
          id="push-relay-channel-rate"
          label="Channel messages per minute"
          unit="per session"
          description="Limits how often Claude is woken and how much of its context events use; beyond it, events are combined into one message pointing at EventPoll."
          value={config.channel_messages_per_minute}
          bounds={SETTINGS.channelMessagesPerMinute}
          disabled={disabled}
          onCommit={(channelMessagesPerMinute) => onUpdate({ channelMessagesPerMinute })}
        />
        <NumberSetting
          id="push-relay-history-hours"
          label="Event history"
          unit="hours"
          description="How far back a polling or reconnecting client can catch up."
          value={config.history_retention_hours}
          bounds={SETTINGS.historyRetentionHours}
          disabled={disabled}
          onCommit={(historyRetentionHours) => onUpdate({ historyRetentionHours })}
        />
        <NumberSetting
          id="push-relay-history-events"
          label="Event history size"
          unit="events per client"
          description="The most events kept for catching up; the oldest go first."
          value={config.history_max_events}
          bounds={SETTINGS.historyMaxEvents}
          disabled={disabled}
          onCommit={(historyMaxEvents) => onUpdate({ historyMaxEvents })}
        />
      </CardContent>
    </Card>
  )
}
