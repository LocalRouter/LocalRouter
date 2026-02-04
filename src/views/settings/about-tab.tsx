import { useState, useEffect } from "react"
import { invoke } from "@tauri-apps/api/core"
import { open } from "@tauri-apps/plugin-shell"
import { ExternalLink, ChevronDown, ChevronRight, Heart, Code, Cpu } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"
import { Badge } from "@/components/ui/Badge"
import { Button } from "@/components/ui/Button"
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible"

interface Inspiration {
  name: string
  license?: string
  url: string
  description: string
}

interface Dependency {
  name: string
  license: string
  url: string
}

const inspirations: Inspiration[] = [
  {
    name: "RouteLLM",
    license: "Apache-2.0",
    url: "https://github.com/lm-sys/RouteLLM",
    description: "ML-based intelligent routing framework. LocalRouter's Strong/Weak feature is a Rust reimplementation of their approach.",
  },
  {
    name: "Microsoft MCP Gateway",
    license: "MIT",
    url: "https://github.com/microsoft/mcp-gateway",
    description: "Inspiration for MCP gateway architecture and unified proxy design patterns.",
  },
  {
    name: "NotDiamond",
    url: "https://notdiamond.ai",
    description: "Inspiration for intelligent model selection and strategy configurations.",
  },
  {
    name: "models.dev (OpenCode)",
    license: "MIT",
    url: "https://models.dev",
    description: "Community-maintained model catalog providing pricing, capabilities, and metadata for AI models. Embedded at build time.",
  },
]

const rustDependencies: Dependency[] = [
  { name: "Tauri", license: "MIT/Apache-2.0", url: "https://tauri.app" },
  { name: "Axum", license: "MIT", url: "https://github.com/tokio-rs/axum" },
  { name: "Tokio", license: "MIT", url: "https://tokio.rs" },
  { name: "Reqwest", license: "MIT/Apache-2.0", url: "https://github.com/seanmonstar/reqwest" },
  { name: "Serde", license: "MIT/Apache-2.0", url: "https://serde.rs" },
  { name: "Candle", license: "MIT/Apache-2.0", url: "https://github.com/huggingface/candle" },
  { name: "Tokenizers", license: "Apache-2.0", url: "https://github.com/huggingface/tokenizers" },
  { name: "Ring", license: "ISC", url: "https://github.com/briansmith/ring" },
  { name: "rusqlite", license: "MIT", url: "https://github.com/rusqlite/rusqlite" },
  { name: "utoipa", license: "MIT/Apache-2.0", url: "https://github.com/juhaku/utoipa" },
  { name: "Tower", license: "MIT", url: "https://github.com/tower-rs/tower" },
  { name: "Tracing", license: "MIT", url: "https://github.com/tokio-rs/tracing" },
  { name: "Chrono", license: "MIT/Apache-2.0", url: "https://github.com/chronotope/chrono" },
  { name: "UUID", license: "MIT/Apache-2.0", url: "https://github.com/uuid-rs/uuid" },
  { name: "OAuth2", license: "MIT/Apache-2.0", url: "https://github.com/ramosbugs/oauth2-rs" },
  { name: "Keyring", license: "MIT/Apache-2.0", url: "https://github.com/hwchen/keyring-rs" },
]

const frontendDependencies: Dependency[] = [
  { name: "React", license: "MIT", url: "https://react.dev" },
  { name: "Radix UI", license: "MIT", url: "https://radix-ui.com" },
  { name: "Tailwind CSS", license: "MIT", url: "https://tailwindcss.com" },
  { name: "Recharts", license: "MIT", url: "https://recharts.org" },
  { name: "React Flow", license: "MIT", url: "https://reactflow.dev" },
  { name: "Lucide Icons", license: "ISC", url: "https://lucide.dev" },
  { name: "Heroicons", license: "MIT", url: "https://heroicons.com" },
  { name: "cmdk", license: "MIT", url: "https://cmdk.paco.me" },
  { name: "Sonner", license: "MIT", url: "https://sonner.emilkowal.ski" },
  { name: "OpenAI SDK", license: "Apache-2.0", url: "https://github.com/openai/openai-node" },
  { name: "WinXP", license: "MIT", url: "https://github.com/nicholasyang/winXP" },
]

export function AboutTab() {
  const [appVersion, setAppVersion] = useState<string>("")
  const [licensesExpanded, setLicensesExpanded] = useState(false)

  useEffect(() => {
    loadAppVersion()
  }, [])

  const loadAppVersion = async () => {
    try {
      const version = await invoke<string>("get_app_version")
      setAppVersion(version)
    } catch (error) {
      console.error("Failed to load app version:", error)
    }
  }

  const handleOpenUrl = (url: string) => {
    open(url)
  }

  return (
    <div className="space-y-6">
      {/* App Info */}
      <Card className="bg-gradient-to-r from-indigo-50 to-purple-50 dark:from-indigo-950/30 dark:to-purple-950/30 border-indigo-200 dark:border-indigo-800">
        <CardContent className="pt-6">
          <div className="flex items-center gap-4">
            <div className="text-4xl">🚀</div>
            <div>
              <h3 className="text-lg font-bold">LocalRouter</h3>
              <p className="text-sm text-muted-foreground">
                Version {appVersion || "0.0.1"} • Licensed under AGPL-3.0-or-later
              </p>
              <p className="text-xs text-muted-foreground mt-1">
                Intelligent AI model selection with OpenAI-compatible API
              </p>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* Inspirations & Credits */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-2">
            <Heart className="h-4 w-4" />
            Inspirations & Credits
          </CardTitle>
          <CardDescription>
            This project was inspired by the following projects. No code was directly used, but their ideas influenced the design.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          {inspirations.map((inspiration) => (
            <div
              key={inspiration.name}
              className="p-3 bg-muted/50 rounded-lg border"
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <span className="font-medium text-sm">{inspiration.name}</span>
                  {inspiration.license && (
                    <Badge variant="secondary" className="text-xs">
                      {inspiration.license}
                    </Badge>
                  )}
                </div>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => handleOpenUrl(inspiration.url)}
                >
                  <ExternalLink className="h-3 w-3" />
                </Button>
              </div>
              <p className="text-xs text-muted-foreground mt-1">
                {inspiration.description}
              </p>
            </div>
          ))}
        </CardContent>
      </Card>

      {/* Strong/Weak Model Licenses */}
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-2">
            <Cpu className="h-4 w-4" />
            Strong/Weak Model Licenses
          </CardTitle>
          <CardDescription>
            When using Strong/Weak intelligent routing, the following model weights are downloaded.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="p-3 bg-amber-50 dark:bg-amber-950/30 border border-amber-200 dark:border-amber-800 rounded-lg">
            <div className="flex items-center gap-2 mb-2">
              <span className="font-medium text-sm">routellm/mf_gpt4_augmented</span>
              <Badge variant="outline" className="text-xs bg-amber-100 dark:bg-amber-900/50">
                Apache-2.0
              </Badge>
            </div>
            <p className="text-xs text-muted-foreground">
              Matrix factorization router model trained on GPT-4 preference data. Hosted on Hugging Face.
            </p>
          </div>
        </CardContent>
      </Card>

      {/* Open Source Dependencies */}
      <Card>
        <CardHeader className="pb-3">
          <Collapsible open={licensesExpanded} onOpenChange={setLicensesExpanded}>
            <CollapsibleTrigger className="flex items-center justify-between w-full">
              <CardTitle className="text-sm flex items-center gap-2">
                <Code className="h-4 w-4" />
                Open Source Dependencies
              </CardTitle>
              {licensesExpanded ? (
                <ChevronDown className="h-4 w-4 text-muted-foreground" />
              ) : (
                <ChevronRight className="h-4 w-4 text-muted-foreground" />
              )}
            </CollapsibleTrigger>
            <CollapsibleContent>
              <CardContent className="pt-4 space-y-4">
                {/* Rust Dependencies */}
                <div>
                  <h4 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide mb-2">
                    Backend (Rust)
                  </h4>
                  <div className="grid grid-cols-2 gap-2">
                    {rustDependencies.map((dep) => (
                      <button
                        key={dep.name}
                        onClick={() => handleOpenUrl(dep.url)}
                        className="flex items-center justify-between p-2 bg-muted/50 rounded border hover:bg-muted text-left text-xs"
                      >
                        <span>{dep.name}</span>
                        <span className="text-muted-foreground">{dep.license}</span>
                      </button>
                    ))}
                  </div>
                </div>

                {/* Frontend Dependencies */}
                <div>
                  <h4 className="text-xs font-semibold text-muted-foreground uppercase tracking-wide mb-2">
                    Frontend (TypeScript/React)
                  </h4>
                  <div className="grid grid-cols-2 gap-2">
                    {frontendDependencies.map((dep) => (
                      <button
                        key={dep.name}
                        onClick={() => handleOpenUrl(dep.url)}
                        className="flex items-center justify-between p-2 bg-muted/50 rounded border hover:bg-muted text-left text-xs"
                      >
                        <span>{dep.name}</span>
                        <span className="text-muted-foreground">{dep.license}</span>
                      </button>
                    ))}
                  </div>
                </div>
              </CardContent>
            </CollapsibleContent>
          </Collapsible>
        </CardHeader>
      </Card>

      {/* Footer */}
      <div className="pt-4 border-t">
        <p className="text-xs text-muted-foreground text-center">
          LocalRouter is open source software. View the full source code and contribute on{" "}
          <button
            onClick={() => handleOpenUrl("https://github.com/mfaro-io/localrouterai")}
            className="text-primary hover:underline"
          >
            GitHub
          </button>
          .
        </p>
      </div>
    </div>
  )
}
