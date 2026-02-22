/**
 * Demo wrapper for the real DragThresholdModelSelector component.
 * Shows the Prioritized Models card from StrategyModelConfiguration.
 */
import { useState } from "react"
import { Bot } from "lucide-react"
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from "@app/components/ui/Card"
import { DragThresholdModelSelector } from "@app/components/strategy/DragThresholdModelSelector"
import type { Model } from "@app/components/strategy/DragThresholdModelSelector"

const DEMO_MODELS: Model[] = [
  { id: "claude-sonnet-4", provider: "Anthropic" },
  { id: "gpt-4o", provider: "OpenAI" },
  { id: "llama-3.3-70b", provider: "Ollama" },
  { id: "gpt-4o-mini", provider: "OpenAI" },
  { id: "claude-haiku-4", provider: "Anthropic" },
  { id: "gemini-2.0-flash", provider: "Google" },
]

const INITIAL_ENABLED: [string, string][] = [
  ["Anthropic", "claude-sonnet-4"],
  ["OpenAI", "gpt-4o"],
  ["Ollama", "llama-3.3-70b"],
]

export function ModelRoutingDemo() {
  const [enabledModels, setEnabledModels] = useState<[string, string][]>(INITIAL_ENABLED)

  return (
    <div className="dark max-w-md">
      <Card>
        <CardHeader>
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-lg bg-primary/10">
              <Bot className="h-4 w-4 text-primary" />
            </div>
            <div>
              <CardTitle className="text-base">Prioritized Models</CardTitle>
              <CardDescription>
                Models to try in order. Falls back to next on failures (outage, context limit, policy violation).
              </CardDescription>
            </div>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          <DragThresholdModelSelector
            availableModels={DEMO_MODELS}
            enabledModels={enabledModels}
            onChange={setEnabledModels}
          />
        </CardContent>
      </Card>
    </div>
  )
}
