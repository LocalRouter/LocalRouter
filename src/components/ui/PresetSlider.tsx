import React from "react"
import { Button } from "@/components/ui/Button"
import { Badge } from "@/components/ui/Badge"
import { Label } from "@/components/ui/label"
import { Slider } from "@/components/ui/Slider"

export interface SliderPreset {
  name: string
  value: number
}

interface PresetSliderProps {
  label: string
  value: number
  onChange: (value: number) => void
  onCommit?: (value: number) => void
  presets: SliderPreset[]
  min: number
  max: number
  step: number
  minLabel?: string
  maxLabel?: string
  formatValue?: (value: number) => string
  disabled?: boolean
  /** Tolerance for matching preset values (default: 0.05) */
  presetTolerance?: number
}

export const PresetSlider: React.FC<PresetSliderProps> = ({
  label,
  value,
  onChange,
  onCommit,
  presets,
  min,
  max,
  step,
  minLabel,
  maxLabel,
  formatValue = (v) => v.toFixed(2),
  disabled = false,
  presetTolerance = 0.05,
}) => {
  const currentPreset = presets.find(p => Math.abs(p.value - value) < presetTolerance)

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <Label className="text-sm font-medium">{label}</Label>
        <div className="flex items-center gap-2">
          <span className="font-mono text-xs text-muted-foreground">{formatValue(value)}</span>
          {currentPreset && (
            <Badge variant="outline" className="text-xs">
              {currentPreset.name}
            </Badge>
          )}
        </div>
      </div>

      <div className="space-y-2">
        <Slider
          min={min}
          max={max}
          step={step}
          value={[value]}
          onValueChange={([v]) => onChange(v)}
          onValueCommit={onCommit ? ([v]) => onCommit(v) : undefined}
          disabled={disabled}
        />
        {(minLabel || maxLabel) && (
          <div className="flex justify-between text-xs text-muted-foreground">
            <span>{minLabel}</span>
            <span>{maxLabel}</span>
          </div>
        )}
      </div>

      <div className="flex gap-2">
        {presets.map((preset) => {
          const isActive = Math.abs(preset.value - value) < presetTolerance
          return (
            <Button
              key={preset.name}
              type="button"
              variant={isActive ? "default" : "outline"}
              size="sm"
              className="flex-1"
              onClick={() => {
                onChange(preset.value)
                onCommit?.(preset.value)
              }}
              disabled={disabled}
            >
              {preset.name}
            </Button>
          )
        })}
      </div>
    </div>
  )
}
