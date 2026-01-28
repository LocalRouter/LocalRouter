/**
 * Step 0: Welcome (First Launch Only)
 *
 * Introductory step shown only on first app launch to guide users.
 */

import { Sparkles, Shield, Zap, Server } from "lucide-react"

export function StepWelcome() {
  return (
    <div className="space-y-6">
      <div className="text-center space-y-2">
        <div className="flex justify-center">
          <div className="p-3 rounded-full bg-primary/10">
            <Sparkles className="h-8 w-8 text-primary" />
          </div>
        </div>
        <h3 className="text-lg font-semibold">Welcome to LocalRouter</h3>
        <p className="text-sm text-muted-foreground max-w-md mx-auto">
          Your local gateway to AI providers. Route requests to multiple LLM providers
          through a single OpenAI-compatible API endpoint.
        </p>
      </div>

      <div className="grid gap-3">
        <FeatureItem
          icon={<Shield className="h-4 w-4" />}
          title="Secure & Private"
          description="All credentials stay on your machine. No data leaves your device."
        />
        <FeatureItem
          icon={<Zap className="h-4 w-4" />}
          title="Smart Selection"
          description="Automatically select the best model or let clients choose from allowed models."
        />
        <FeatureItem
          icon={<Server className="h-4 w-4" />}
          title="MCP Support"
          description="Proxy MCP servers to give your AI applications access to tools and data."
        />
      </div>

      <div className="pt-2 border-t">
        <p className="text-sm text-muted-foreground text-center">
          Let&apos;s create your first client to get started. A client represents
          an application that will connect to LocalRouter.
        </p>
      </div>
    </div>
  )
}

function FeatureItem({
  icon,
  title,
  description,
}: {
  icon: React.ReactNode
  title: string
  description: string
}) {
  return (
    <div className="flex gap-3 p-3 rounded-lg bg-muted/50">
      <div className="flex-shrink-0 mt-0.5 text-primary">{icon}</div>
      <div>
        <p className="text-sm font-medium">{title}</p>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>
    </div>
  )
}
