import { Brain, Clock, HardDrive } from "lucide-react"
import type { SafetyModelConfig } from "@/types/tauri-commands"

interface ResourceRequirementsProps {
  models: SafetyModelConfig[]
}

function formatMb(mb: number): string {
  if (mb >= 1024) return `${(mb / 1024).toFixed(1)} GB`
  return `${mb} MB`
}

export function ResourceRequirements({ models }: ResourceRequirementsProps) {
  if (models.length === 0) return null

  const totalMemory = models.reduce((sum, m) => sum + (m.memory_mb ?? 0), 0)
  const maxLatency = Math.max(...models.map((m) => m.latency_ms ?? 0))
  const totalDisk = models.reduce((sum, m) => sum + (m.disk_size_mb ?? 0), 0)

  if (totalMemory === 0 && maxLatency === 0 && totalDisk === 0) return null

  return (
    <div className="flex gap-6 text-sm text-muted-foreground">
      {totalMemory > 0 && (
        <div className="flex items-center gap-1.5">
          <Brain className="h-4 w-4" />
          <span>{formatMb(totalMemory)} RAM</span>
        </div>
      )}
      {maxLatency > 0 && (
        <div className="flex items-center gap-1.5">
          <Clock className="h-4 w-4" />
          <span>~{maxLatency}ms latency</span>
        </div>
      )}
      {totalDisk > 0 && (
        <div className="flex items-center gap-1.5">
          <HardDrive className="h-4 w-4" />
          <span>{formatMb(totalDisk)} disk</span>
        </div>
      )}
    </div>
  )
}
