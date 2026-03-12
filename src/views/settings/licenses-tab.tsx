import { open } from "@tauri-apps/plugin-shell"
import { Heart } from "lucide-react"
import { Card, CardContent, CardHeader, CardTitle, CardDescription } from "@/components/ui/Card"

interface Credit {
  name: string
  license?: string
  url: string
}

const credits: Credit[] = [
  // Inspirations & runtime resources
  { name: "RouteLLM", license: "Apache-2.0", url: "https://github.com/lm-sys/RouteLLM" },
  { name: "routellm/mf_gpt4_augmented", license: "Apache-2.0", url: "https://github.com/lm-sys/RouteLLM" },
  { name: "Microsoft MCP Gateway", license: "MIT", url: "https://github.com/microsoft/mcp-gateway" },
  { name: "Microsoft Presidio", license: "MIT", url: "https://github.com/microsoft/presidio" },
  { name: "LLM Guard (ProtectAI)", license: "MIT", url: "https://github.com/protectai/llm-guard" },
  { name: "models.dev (OpenCode)", license: "MIT", url: "https://models.dev" },
  // Backend (Rust)
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
  // Frontend (TypeScript/React)
  { name: "React", license: "MIT", url: "https://react.dev" },
  { name: "Radix UI", license: "MIT", url: "https://radix-ui.com" },
  { name: "Tailwind CSS", license: "MIT", url: "https://tailwindcss.com" },
  { name: "MCP SDK", license: "MIT", url: "https://github.com/modelcontextprotocol/typescript-sdk" },
  { name: "OpenAI SDK", license: "Apache-2.0", url: "https://github.com/openai/openai-node" },
  { name: "Vercel AI SDK", license: "Apache-2.0", url: "https://github.com/vercel/ai" },
  { name: "Recharts", license: "MIT", url: "https://recharts.org" },
  { name: "React Flow", license: "MIT", url: "https://reactflow.dev" },
  { name: "TanStack Table", license: "MIT", url: "https://tanstack.com/table" },
  { name: "dnd kit", license: "MIT", url: "https://dndkit.com" },
  { name: "react-markdown", license: "MIT", url: "https://github.com/remarkjs/react-markdown" },
  { name: "React Resizable Panels", license: "MIT", url: "https://github.com/bvaughn/react-resizable-panels" },
  { name: "Lucide Icons", license: "ISC", url: "https://lucide.dev" },
  { name: "Heroicons", license: "MIT", url: "https://heroicons.com" },
  { name: "cmdk", license: "MIT", url: "https://cmdk.paco.me" },
  { name: "Sonner", license: "MIT", url: "https://sonner.emilkowal.ski" },
  { name: "WinXP", license: "MIT", url: "https://github.com/nicholasyang/winXP" },
]

export function LicensesTab() {
  const handleOpenUrl = (url: string) => {
    open(url)
  }

  return (
    <div className="space-y-6 max-w-2xl">
      <Card>
        <CardHeader className="pb-3">
          <CardTitle className="text-sm flex items-center gap-2">
            <Heart className="h-4 w-4" />
            Licenses & Credits
          </CardTitle>
          <CardDescription>
            Open source projects, inspirations, and runtime resources used by LocalRouter.
          </CardDescription>
        </CardHeader>
        <CardContent className="grid grid-cols-2 gap-2">
          {credits.map((credit) => (
            <button
              key={credit.name}
              onClick={() => handleOpenUrl(credit.url)}
              className="flex items-center justify-between p-2 bg-muted/50 rounded border hover:bg-muted text-left text-xs"
            >
              <span>{credit.name}</span>
              {credit.license && (
                <span className="text-muted-foreground">{credit.license}</span>
              )}
            </button>
          ))}
        </CardContent>
      </Card>

      <div className="pt-4 border-t">
        <p className="text-xs text-muted-foreground text-center">
          LocalRouter is open source software licensed under AGPL-3.0-or-later. View the full source code on{" "}
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
