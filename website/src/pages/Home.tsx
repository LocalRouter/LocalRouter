import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import ArchitectureDiagram from '@/components/ArchitectureDiagram'
import Logo from '@/components/Logo'
import {
  ArrowRight,
  Shield,
  Check,
  Key,
  Route,
  Wrench,
  FlaskConical,
} from 'lucide-react'

export default function Home() {
  return (
    <div className="flex flex-col">
      {/* Hero */}
      <section className="relative overflow-hidden border-b bg-gradient-to-b from-muted/50 to-background">
        <div className="mx-auto max-w-7xl px-4 py-24 sm:px-6 sm:py-32 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <h1 className="text-4xl font-bold tracking-tight sm:text-5xl lg:text-6xl">
              One Local API.
              <br />
              <span className="text-primary">For LLMs and MCPs.</span>
            </h1>
            <p className="mt-6 text-lg text-muted-foreground sm:text-xl">
              A vault for managing your LLM and MCP keys. Give granular access to your local apps to specific models and MCP endpoints. LLM routing based on complexity and fallback to other providers or offline models.
            </p>
            <div className="mt-10 flex flex-col items-center justify-center gap-4 sm:flex-row">
              <Button asChild size="xl">
                <Link to="/download">
                  Download for Free
                  <ArrowRight className="ml-2 h-4 w-4" />
                </Link>
              </Button>
              <Button asChild variant="outline" size="xl">
                <a
                  href="https://github.com/LocalRouter/LocalRouter"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  View on GitHub
                </a>
              </Button>
            </div>
          </div>

          {/* Connection Graph Visual */}
          <div className="relative mx-auto mt-16 max-w-5xl h-[500px] sm:h-[550px]">
            {/* SVG Connection Lines - coordinates match node positions */}
            <svg className="absolute inset-0 w-full h-full pointer-events-none" viewBox="0 0 1000 550" preserveAspectRatio="xMidYMid meet">
              {/* Left to center connections (Apps to LocalRouter) - y coords: 94, 165, 237, 308 */}
              <path d="M 200 94 Q 350 94 470 275" stroke="url(#gradient-blue)" strokeWidth="2" fill="none" className="animate-pulse-flow" />
              <path d="M 200 165 Q 320 165 470 275" stroke="url(#gradient-blue)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.3s' }} />
              <path d="M 200 237 Q 320 237 470 275" stroke="url(#gradient-blue)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.6s' }} />
              <path d="M 200 308 Q 350 308 470 275" stroke="url(#gradient-blue)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.9s' }} />

              {/* Center to right-top connections (LocalRouter to Providers) - y coords: 83, 149 */}
              <path d="M 530 275 Q 650 83 800 83" stroke="url(#gradient-violet)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.2s' }} />
              <path d="M 530 275 Q 650 149 800 149" stroke="url(#gradient-violet)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.5s' }} />

              {/* Center to right-bottom connections (LocalRouter to MCP) - y coords: 374, 440 */}
              <path d="M 530 275 Q 650 374 800 374" stroke="url(#gradient-emerald)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.4s' }} />
              <path d="M 530 275 Q 650 440 800 440" stroke="url(#gradient-emerald)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.7s' }} />

              {/* Gradients */}
              <defs>
                <linearGradient id="gradient-blue" x1="0%" y1="0%" x2="100%" y2="0%">
                  <stop offset="0%" stopColor="#3b82f6" stopOpacity="0.3" />
                  <stop offset="50%" stopColor="#3b82f6" stopOpacity="0.8" />
                  <stop offset="100%" stopColor="#8b5cf6" stopOpacity="0.5" />
                </linearGradient>
                <linearGradient id="gradient-violet" x1="0%" y1="0%" x2="100%" y2="0%">
                  <stop offset="0%" stopColor="#8b5cf6" stopOpacity="0.5" />
                  <stop offset="50%" stopColor="#8b5cf6" stopOpacity="0.8" />
                  <stop offset="100%" stopColor="#8b5cf6" stopOpacity="0.3" />
                </linearGradient>
                <linearGradient id="gradient-emerald" x1="0%" y1="0%" x2="100%" y2="0%">
                  <stop offset="0%" stopColor="#10b981" stopOpacity="0.5" />
                  <stop offset="50%" stopColor="#10b981" stopOpacity="0.8" />
                  <stop offset="100%" stopColor="#10b981" stopOpacity="0.3" />
                </linearGradient>
              </defs>
            </svg>

            {/* Left side label */}
            <div className="absolute left-[12%] sm:left-[15%] top-[1%] text-left">
              <span className="text-xs font-medium text-blue-500 uppercase tracking-wide">Works With</span>
              <h3 className="text-base sm:text-lg font-semibold"><i>bring-your-own-key</i> Apps</h3>
            </div>

            {/* Left Apps - 17%, 30%, 43%, 56% of 550 = 94, 165, 237, 308 */}
            <div className="absolute left-[12%] sm:left-[15%] top-[17%] -translate-y-1/2">
              <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-blue-500/10 border border-blue-500/30 shadow-sm backdrop-blur-sm">
                <img src="/icons/cursor.svg" alt="Cursor" className="h-6 w-6" />
                <span className="text-sm font-medium hidden sm:inline">Cursor</span>
              </div>
            </div>
            <div className="absolute left-[12%] sm:left-[15%] top-[30%] -translate-y-1/2">
              <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-blue-500/10 border border-blue-500/30 shadow-sm backdrop-blur-sm">
                <div className="h-6 w-6 rounded bg-gradient-to-br from-indigo-500 to-purple-600 flex items-center justify-center text-white font-bold text-xs">{'</>'}</div>
                <span className="text-sm font-medium hidden sm:inline">OpenCode</span>
              </div>
            </div>
            <div className="absolute left-[12%] sm:left-[15%] top-[43%] -translate-y-1/2">
              <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-blue-500/10 border border-blue-500/30 shadow-sm backdrop-blur-sm">
                <img src="/icons/open-webui.png" alt="Open WebUI" className="h-6 w-6" />
                <span className="text-sm font-medium hidden sm:inline">Open WebUI</span>
              </div>
            </div>
            <div className="absolute left-[12%] sm:left-[15%] top-[56%] -translate-y-1/2">
              <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-blue-500/10 border border-blue-500/30 shadow-sm backdrop-blur-sm">
                <div className="h-6 w-6 rounded bg-gradient-to-br from-cyan-500 to-blue-600 flex items-center justify-center text-white font-bold text-xs">C</div>
                <span className="text-sm font-medium hidden sm:inline">Cline</span>
              </div>
            </div>
            <div className="absolute left-[12%] sm:left-[15%] top-[68%] text-muted-foreground text-xs">
              + any OpenAI-compatible app
            </div>

            {/* Center: LocalRouter Hub */}
            <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 z-10">
              <div className="relative rounded-2xl border-2 border-primary bg-gradient-to-br from-primary/20 to-violet-500/20 p-4 sm:p-6 shadow-2xl backdrop-blur">
                <div className="text-center">
                  <div className="h-12 w-12 sm:h-16 sm:w-16 rounded-xl bg-gradient-to-br from-primary to-violet-600 flex items-center justify-center mx-auto mb-2 shadow-lg">
                    <Logo className="h-8 w-auto sm:h-10 text-white" />
                  </div>
                  <div className="font-bold text-sm sm:text-lg">LocalRouter</div>
                  <div className="text-[10px] sm:text-xs text-muted-foreground">localhost:3625</div>
                </div>
              </div>
            </div>

            {/* Right side - LLM Providers label */}
            <div className="absolute right-[12%] sm:right-[15%] top-[-1%] text-right">
              <span className="text-xs font-medium text-violet-500 uppercase tracking-wide">Connects to</span>
              <h3 className="text-base sm:text-lg font-semibold">Any LLM Provider</h3>
            </div>

            {/* Right LLM Providers - 15%, 27% of 550 = 83, 149 */}
            <div className="absolute right-[12%] sm:right-[15%] top-[15%] -translate-y-1/2">
              <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-violet-500/10 border border-violet-500/30 shadow-sm backdrop-blur-sm">
                <img src="/icons/chatgpt.svg" alt="OpenAI" className="h-6 w-6" />
                <span className="text-sm font-medium hidden sm:inline">OpenAI</span>
              </div>
            </div>
            <div className="absolute right-[12%] sm:right-[15%] top-[27%] -translate-y-1/2">
              <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-violet-500/10 border border-violet-500/30 shadow-sm backdrop-blur-sm">
                <img src="/icons/ollama.svg" alt="Ollama" className="h-6 w-6" />
                <span className="text-sm font-medium hidden sm:inline">Ollama</span>
              </div>
            </div>
            <div className="absolute right-[12%] sm:right-[15%] top-[36%] text-muted-foreground text-xs text-right">
              + Anthropic, Gemini, more...
            </div>

            {/* Right side - MCP Servers label */}
            <div className="absolute right-[12%] sm:right-[15%] top-[52%] text-right">
              <span className="text-xs font-medium text-emerald-500 uppercase tracking-wide">Connects to</span>
              <h3 className="text-base sm:text-lg font-semibold">Any MCP Server</h3>
            </div>

            {/* Right MCP Servers - 68%, 80% of 550 = 374, 440 */}
            <div className="absolute right-[12%] sm:right-[15%] top-[68%] -translate-y-1/2">
              <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-emerald-500/10 border border-emerald-500/30 shadow-sm backdrop-blur-sm">
                <img src="/icons/github.svg" alt="GitHub" className="h-6 w-6" />
                <span className="text-sm font-medium hidden sm:inline">GitHub</span>
              </div>
            </div>
            <div className="absolute right-[12%] sm:right-[15%] top-[80%] -translate-y-1/2">
              <div className="flex items-center gap-2 px-3 py-2 rounded-lg bg-emerald-500/10 border border-emerald-500/30 shadow-sm backdrop-blur-sm">
                <img src="/icons/filesystem.svg" alt="Filesystem" className="h-6 w-6" />
                <span className="text-sm font-medium hidden sm:inline">Filesystem</span>
              </div>
            </div>
            <div className="absolute right-[12%] sm:right-[15%] top-[89%] text-muted-foreground text-xs text-right">
              + Jira, Slack, more...
            </div>
          </div>
        </div>
      </section>

      {/* Feature 1: Credential Management */}
      <section className="border-b py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="grid gap-12 lg:grid-cols-2 lg:gap-16 items-center">
            <div>
              <div className="flex items-center gap-2 mb-4">
                <Key className="h-5 w-5 text-blue-500" />
                <span className="text-sm font-medium text-blue-500 uppercase tracking-wide">Credential Management</span>
              </div>
              <h2 className="text-3xl font-bold tracking-tight sm:text-4xl">
                One Place for All Your Credentials
              </h2>
              <p className="mt-4 text-lg text-muted-foreground">
                Stop scattering API keys across config files. LocalRouter securely stores all your provider credentials and gives each app exactly the access it needs.
              </p>
              <ul className="mt-8 space-y-4">
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-blue-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Flexible client authentication</span>
                    <p className="text-sm text-muted-foreground">API keys, OAuth, or STDIO auth for each connected app</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-blue-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Per-client permissions</span>
                    <p className="text-sm text-muted-foreground">Assign specific models and MCP servers to each client</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-blue-500 mt-0.5" />
                  <div>
                    <span className="font-medium">OS keychain integration</span>
                    <p className="text-sm text-muted-foreground">Secrets stored securely using your system&apos;s native keychain</p>
                  </div>
                </li>
              </ul>
            </div>
            {/* Visual: Credential Vault */}
            <div className="relative">
              <div className="rounded-xl border-2 border-slate-700 bg-gradient-to-br from-slate-900 to-slate-800 p-6 shadow-2xl">
                {/* Vault Header */}
                <div className="flex items-center gap-3 mb-5 pb-4 border-b border-slate-700">
                  <div className="h-10 w-10 rounded-lg bg-gradient-to-br from-amber-500 to-yellow-600 flex items-center justify-center">
                    <Shield className="h-5 w-5 text-white" />
                  </div>
                  <div>
                    <div className="text-white font-semibold">Credential Vault</div>
                    <div className="text-slate-400 text-xs">Secured by OS Keychain</div>
                  </div>
                  <div className="ml-auto flex items-center gap-1.5">
                    <div className="h-2 w-2 rounded-full bg-emerald-500 animate-pulse" />
                    <span className="text-emerald-400 text-xs">Encrypted</span>
                  </div>
                </div>
                {/* Credentials Grid */}
                <div className="space-y-3">
                  {/* API Key Credentials */}
                  <div className="rounded-lg bg-white/5 border border-white/10 p-3">
                    <div className="flex items-center gap-2 mb-2">
                      <Key className="h-4 w-4 text-blue-400" />
                      <span className="text-slate-300 text-xs font-medium uppercase tracking-wide">API Keys</span>
                    </div>
                    <div className="grid grid-cols-2 gap-2">
                      <div className="flex items-center gap-2 px-2 py-1.5 rounded bg-emerald-500/10 border border-emerald-500/20">
                        <img src="/icons/chatgpt.svg" alt="OpenAI" className="h-4 w-4" />
                        <span className="text-white text-xs">OpenAI</span>
                        <span className="ml-auto text-emerald-400 text-xs font-mono">sk-...4f2x</span>
                      </div>
                      <div className="flex items-center gap-2 px-2 py-1.5 rounded bg-violet-500/10 border border-violet-500/20">
                        <img src="/icons/anthropic.svg" alt="Anthropic" className="h-4 w-4" />
                        <span className="text-white text-xs">Anthropic</span>
                        <span className="ml-auto text-violet-400 text-xs font-mono">sk-...8k1m</span>
                      </div>
                      <div className="flex items-center gap-2 px-2 py-1.5 rounded bg-blue-500/10 border border-blue-500/20">
                        <img src="/icons/openrouter.svg" alt="OpenRouter" className="h-4 w-4" />
                        <span className="text-white text-xs">OpenRouter</span>
                        <span className="ml-auto text-blue-400 text-xs font-mono">sk-...9p3q</span>
                      </div>
                      <div className="flex items-center gap-2 px-2 py-1.5 rounded bg-orange-500/10 border border-orange-500/20">
                        <div className="h-4 w-4 rounded bg-orange-500/30 flex items-center justify-center text-orange-400 text-[8px] font-bold">G</div>
                        <span className="text-white text-xs">Groq</span>
                        <span className="ml-auto text-orange-400 text-xs font-mono">gsk-...2n7b</span>
                      </div>
                    </div>
                  </div>
                  {/* OAuth Credentials */}
                  <div className="rounded-lg bg-white/5 border border-white/10 p-3">
                    <div className="flex items-center gap-2 mb-2">
                      <svg className="h-4 w-4 text-amber-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                        <path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4M10 17l5-5-5-5M13.8 12H3" />
                      </svg>
                      <span className="text-slate-300 text-xs font-medium uppercase tracking-wide">OAuth Tokens</span>
                    </div>
                    <div className="grid grid-cols-2 gap-2">
                      <div className="flex items-center gap-2 px-2 py-1.5 rounded bg-slate-500/10 border border-slate-500/20">
                        <img src="/icons/github.svg" alt="GitHub" className="h-4 w-4" />
                        <span className="text-white text-xs">GitHub</span>
                        <span className="ml-auto text-slate-400 text-xs">Connected</span>
                      </div>
                      <div className="flex items-center gap-2 px-2 py-1.5 rounded bg-blue-500/10 border border-blue-500/20">
                        <img src="/icons/jira.svg" alt="Jira" className="h-4 w-4" />
                        <span className="text-white text-xs">Jira</span>
                        <span className="ml-auto text-blue-400 text-xs">Connected</span>
                      </div>
                      <div className="flex items-center gap-2 px-2 py-1.5 rounded bg-red-500/10 border border-red-500/20">
                        <img src="/icons/gmail.svg" alt="Gmail" className="h-4 w-4" />
                        <span className="text-white text-xs">Gmail</span>
                        <span className="ml-auto text-red-400 text-xs">Connected</span>
                      </div>
                      <div className="flex items-center gap-2 px-2 py-1.5 rounded bg-purple-500/10 border border-purple-500/20">
                        <div className="h-4 w-4 rounded bg-purple-500/30 flex items-center justify-center text-purple-400 text-[8px] font-bold">S</div>
                        <span className="text-white text-xs">Slack</span>
                        <span className="ml-auto text-purple-400 text-xs">Connected</span>
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>

      {/* Feature 2: Auto Router */}
      <section className="border-b py-16 sm:py-24 bg-muted/30">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="grid gap-12 lg:grid-cols-2 lg:gap-16 items-center">
            {/* Visual: Decision Tree */}
            <div className="relative order-2 lg:order-1">
              <div className="rounded-xl border bg-gradient-to-br from-violet-950 to-slate-900 p-6 shadow-2xl">
                {/* Incoming Request */}
                <div className="flex justify-center mb-3">
                  <div className="px-4 py-2 rounded-lg bg-white/10 border border-white/10 text-white text-sm">
                    model: <span className="text-violet-400">&quot;auto&quot;</span>
                  </div>
                </div>
                {/* Arrow down */}
                <div className="flex justify-center mb-2">
                  <div className="w-0.5 h-4 bg-violet-500/50" />
                </div>
                {/* Decision Node */}
                <div className="flex justify-center mb-3">
                  <div className="px-4 py-2 rounded-full bg-violet-500/20 border-2 border-violet-500/50 text-violet-300 text-sm font-medium">
                    Is request complex?
                  </div>
                </div>
                {/* Branches */}
                <div className="grid grid-cols-2 gap-4">
                  {/* Complex Branch */}
                  <div>
                    <div className="flex justify-center mb-2">
                      <div className="flex items-center gap-1">
                        <div className="w-8 h-0.5 bg-emerald-500/50" />
                        <span className="text-emerald-400 text-xs font-medium">Yes</span>
                        <div className="w-0.5 h-4 bg-emerald-500/50" />
                      </div>
                    </div>
                    <div className="rounded-lg bg-emerald-500/10 border border-emerald-500/30 p-3">
                      <div className="text-emerald-400 text-xs font-medium uppercase tracking-wide mb-2 text-center">Strong Models</div>
                      <div className="space-y-1.5">
                        <div className="flex items-center justify-between px-2 py-1 rounded bg-white/5">
                          <div className="flex items-center gap-1.5">
                            <img src="/icons/chatgpt.svg" alt="OpenAI" className="h-3.5 w-3.5" />
                            <span className="text-white text-xs">GPT-5.2</span>
                          </div>
                          <span className="text-emerald-400 text-[10px]">Primary</span>
                        </div>
                        <div className="flex items-center justify-between px-2 py-1 rounded bg-white/5">
                          <div className="flex items-center gap-1.5">
                            <img src="/icons/anthropic.svg" alt="Anthropic" className="h-3.5 w-3.5" />
                            <span className="text-white text-xs">Opus 4.5</span>
                          </div>
                          <span className="text-blue-400 text-[10px]">Secondary</span>
                        </div>
                        <div className="flex items-center justify-between px-2 py-1 rounded bg-white/5">
                          <div className="flex items-center gap-1.5">
                            <img src="/icons/ollama.svg" alt="Ollama" className="h-3.5 w-3.5" />
                            <span className="text-white text-xs">Llama 4 405B</span>
                          </div>
                          <span className="text-slate-400 text-[10px]">Offline</span>
                        </div>
                      </div>
                    </div>
                  </div>
                  {/* Simple Branch */}
                  <div>
                    <div className="flex justify-center mb-2">
                      <div className="flex items-center gap-1">
                        <div className="w-8 h-0.5 bg-amber-500/50" />
                        <span className="text-amber-400 text-xs font-medium">No</span>
                        <div className="w-0.5 h-4 bg-amber-500/50" />
                      </div>
                    </div>
                    <div className="rounded-lg bg-amber-500/10 border border-amber-500/30 p-3">
                      <div className="text-amber-400 text-xs font-medium uppercase tracking-wide mb-2 text-center">Fast Models</div>
                      <div className="space-y-1.5">
                        <div className="flex items-center justify-between px-2 py-1 rounded bg-white/5">
                          <div className="flex items-center gap-1.5">
                            <img src="/icons/chatgpt.svg" alt="OpenAI" className="h-3.5 w-3.5" />
                            <span className="text-white text-xs">GPT-4o mini</span>
                          </div>
                          <span className="text-emerald-400 text-[10px]">Primary</span>
                        </div>
                        <div className="flex items-center justify-between px-2 py-1 rounded bg-white/5">
                          <div className="flex items-center gap-1.5">
                            <img src="/icons/anthropic.svg" alt="Anthropic" className="h-3.5 w-3.5" />
                            <span className="text-white text-xs">Haiku 3.5</span>
                          </div>
                          <span className="text-blue-400 text-[10px]">Secondary</span>
                        </div>
                        <div className="flex items-center justify-between px-2 py-1 rounded bg-white/5">
                          <div className="flex items-center gap-1.5">
                            <img src="/icons/ollama.svg" alt="Ollama" className="h-3.5 w-3.5" />
                            <span className="text-white text-xs">Llama 3.2 3B</span>
                          </div>
                          <span className="text-slate-400 text-[10px]">Offline</span>
                        </div>
                      </div>
                    </div>
                  </div>
                </div>
                {/* Legend */}
                <div className="mt-4 pt-3 border-t border-white/10 flex justify-center gap-4">
                  <div className="flex items-center gap-1.5">
                    <div className="h-2 w-2 rounded-full bg-emerald-500" />
                    <span className="text-slate-400 text-[10px]">Primary Online</span>
                  </div>
                  <div className="flex items-center gap-1.5">
                    <div className="h-2 w-2 rounded-full bg-blue-500" />
                    <span className="text-slate-400 text-[10px]">Secondary Online</span>
                  </div>
                  <div className="flex items-center gap-1.5">
                    <div className="h-2 w-2 rounded-full bg-slate-500" />
                    <span className="text-slate-400 text-[10px]">Offline Fallback</span>
                  </div>
                </div>
              </div>
            </div>
            <div className="order-1 lg:order-2">
              <div className="flex items-center gap-2 mb-4">
                <Route className="h-5 w-5 text-violet-500" />
                <span className="text-sm font-medium text-violet-500 uppercase tracking-wide">Smart Routing</span>
              </div>
              <h2 className="text-3xl font-bold tracking-tight sm:text-4xl">
                Intelligent LLM Auto Router
              </h2>
              <p className="mt-4 text-lg text-muted-foreground">
                Never worry about provider outages, rate limits, policy violation again. LocalRouter automatically routes requests to the next available model and based on prompt complexity.
              </p>
              <ul className="mt-8 space-y-4">
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-violet-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Complexity-based routing</span>
                    <p className="text-sm text-muted-foreground">Route complex requests to powerful models, simple ones to fast models</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-violet-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Automatic offline fallback</span>
                    <p className="text-sm text-muted-foreground">Seamlessly fall back to local models when offline</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-violet-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Multi-provider redundancy</span>
                    <p className="text-sm text-muted-foreground">Primary and secondary providers ensure high availability</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <FlaskConical className="h-5 w-5 shrink-0 text-amber-500 mt-0.5" />
                  <div>
                    <span className="font-medium text-amber-600 dark:text-amber-400">Experimental: Strong/Weak model selection</span>
                    <p className="text-sm text-muted-foreground">ML model to determine if prompt is complex/simple to determine whether strong/weak LLM should be used</p>
                  </div>
                </li>
              </ul>
            </div>
          </div>
        </div>
      </section>

      {/* Feature 3: Unified MCP */}
      <section className="border-b py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="grid gap-12 lg:grid-cols-2 lg:gap-16 items-center">
            <div>
              <div className="flex items-center gap-2 mb-4">
                <Wrench className="h-5 w-5 text-emerald-500" />
                <span className="text-sm font-medium text-emerald-500 uppercase tracking-wide">MCP Gateway</span>
              </div>
              <h2 className="text-3xl font-bold tracking-tight sm:text-4xl">
                Unified MCP Gateway
              </h2>
              <p className="mt-4 text-lg text-muted-foreground">
                Connect once, access all your tools. LocalRouter is a reverse proxy to unify multiple MCP servers into a single endpoint with per-client access control.
              </p>
              <ul className="mt-8 space-y-4">
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-emerald-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Merged tool namespace</span>
                    <p className="text-sm text-muted-foreground">Single MCP endpoint exposes tools from all allowed servers</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-emerald-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Per-client access control</span>
                    <p className="text-sm text-muted-foreground">Control which MCP servers each client can access</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <FlaskConical className="h-5 w-5 shrink-0 text-amber-500 mt-0.5" />
                  <div>
                    <span className="font-medium text-amber-600 dark:text-amber-400">Experimental: Deferred tool loading</span>
                    <p className="text-sm text-muted-foreground">Minimize token context with on-demand tool discovery</p>
                  </div>
                </li>
              </ul>
            </div>
            {/* Visual: MCP Architecture */}
            <div className="relative">
              <div className="rounded-xl border bg-gradient-to-br from-emerald-950 to-slate-900 p-6 shadow-2xl">
                {/* Three-column layout: Client → Gateway → Servers */}
                <div className="flex items-stretch gap-3">
                  {/* Client */}
                  <div className="flex flex-col items-center justify-center">
                    <div className="rounded-lg bg-blue-500/20 border border-blue-500/30 p-3 text-center">
                      <div className="h-8 w-8 rounded bg-blue-500/30 flex items-center justify-center mx-auto mb-1">
                        <span className="text-blue-400 text-sm font-bold">{'</>'}</span>
                      </div>
                      <div className="text-white text-xs font-medium">Client</div>
                      <div className="text-blue-400 text-[10px]">Cursor</div>
                    </div>
                  </div>

                  {/* STDIO Connection */}
                  <div className="flex flex-col items-center justify-center">
                    <div className="flex items-center gap-1">
                      <div className="w-6 h-0.5 bg-slate-500" />
                      <div className="px-1.5 py-0.5 rounded bg-slate-700 text-slate-400 text-[9px] font-mono">STDIO</div>
                      <div className="w-6 h-0.5 bg-slate-500" />
                    </div>
                  </div>

                  {/* Gateway */}
                  <div className="flex flex-col items-center justify-center">
                    <div className="rounded-lg bg-emerald-500/20 border-2 border-emerald-500/50 p-3 text-center">
                      <Wrench className="h-6 w-6 text-emerald-400 mx-auto mb-1" />
                      <div className="text-white text-xs font-medium">Unified MCP</div>
                      <div className="text-emerald-400 text-[10px]">Gateway</div>
                    </div>
                  </div>

                  {/* Connections to servers */}
                  <div className="flex flex-col items-center justify-center gap-2">
                    <div className="flex items-center">
                      <div className="w-3 h-0.5 bg-slate-600" />
                      <div className="px-1 py-0.5 rounded bg-slate-700/50 text-slate-500 text-[8px] font-mono">STDIO</div>
                    </div>
                    <div className="flex items-center">
                      <div className="w-3 h-0.5 bg-emerald-600" />
                      <div className="px-1 py-0.5 rounded bg-emerald-900/50 text-emerald-500 text-[8px] font-mono">API Key</div>
                    </div>
                    <div className="flex items-center">
                      <div className="w-3 h-0.5 bg-emerald-600" />
                      <div className="px-1 py-0.5 rounded bg-emerald-900/50 text-emerald-500 text-[8px] font-mono">OAuth</div>
                    </div>
                  </div>

                  {/* MCP Servers */}
                  <div className="flex flex-col gap-2">
                    {/* Local Server */}
                    <div className="rounded-lg bg-slate-500/10 border border-slate-500/30 p-2 flex items-center gap-2">
                      <img src="/icons/filesystem.svg" alt="Filesystem" className="h-5 w-5" />
                      <div>
                        <div className="text-white text-xs font-medium">Filesystem</div>
                        <div className="text-slate-400 text-[10px]">Local • Offline</div>
                      </div>
                    </div>
                    {/* Online Servers */}
                    <div className="rounded-lg bg-emerald-500/10 border border-emerald-500/30 p-2 flex items-center gap-2">
                      <img src="/icons/github.svg" alt="GitHub" className="h-5 w-5" />
                      <div>
                        <div className="text-white text-xs font-medium">GitHub</div>
                        <div className="text-emerald-400 text-[10px]">Cloud • Online</div>
                      </div>
                    </div>
                    <div className="rounded-lg bg-emerald-500/10 border border-emerald-500/30 p-2 flex items-center gap-2">
                      <img src="/icons/jira.svg" alt="Jira" className="h-5 w-5" />
                      <div>
                        <div className="text-white text-xs font-medium">Jira</div>
                        <div className="text-emerald-400 text-[10px]">Cloud • Online</div>
                      </div>
                    </div>
                  </div>
                </div>

                {/* Legend */}
                <div className="mt-4 pt-3 border-t border-white/10 flex justify-center gap-4">
                  <div className="flex items-center gap-1.5">
                    <div className="h-2 w-2 rounded-full bg-emerald-500" />
                    <span className="text-slate-400 text-[10px]">Online (OAuth)</span>
                  </div>
                  <div className="flex items-center gap-1.5">
                    <div className="h-2 w-2 rounded-full bg-slate-500" />
                    <span className="text-slate-400 text-[10px]">Local (STDIO)</span>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </section>

      {/* OLD CAROUSEL SECTIONS - COMMENTED OUT
      {/* Compatible Apps */}
      {/*<section className="border-b py-12 sm:py-16 bg-muted/30 overflow-hidden">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="mx-auto max-w-3xl text-center mb-10">
            <h2 className="text-2xl font-bold sm:text-3xl">Works With <i>bring-your-own-key</i> Apps</h2>
            <p className="mt-3 text-muted-foreground">
              Point any app that supports OpenAI-compatible API to LocalRouter.
            </p>
          </div>
        </div>
        <div className="group relative">
          <div className="flex gap-4 animate-scroll group-hover:[animation-play-state:paused]">
            {[...Array(2)].map((_, i) => (
              <div key={i} className="flex gap-4 shrink-0">
                ...
              </div>
            ))}
          </div>
        </div>
      </section>*/}

      {/* LLM Providers */}
      {/*<section className="border-b py-12 sm:py-16 overflow-hidden">
        ...
      </section>*/}

      {/* MCP Servers */}
      {/*<section className="border-b py-12 sm:py-16 bg-muted/30 overflow-hidden">
        ...
      </section>*/}
      {/* END OLD CAROUSEL SECTIONS */}

      {/* Privacy */}
      <section className="border-b py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <Shield className="mx-auto h-12 w-12 text-primary" />
            <h2 className="mt-4 text-2xl font-bold sm:text-3xl">Privacy First</h2>
            <p className="mt-4 text-muted-foreground">
              LocalRouter runs entirely on your machine. No telemetry, no analytics, no cloud sync.
              Your API keys and request data never leave your computer.
            </p>
            <div className="mt-8 flex flex-wrap justify-center gap-4 text-sm text-muted-foreground">
              <div className="flex items-center gap-2">
                <Check className="h-4 w-4 text-primary" />
                No telemetry
              </div>
              <div className="flex items-center gap-2">
                <Check className="h-4 w-4 text-primary" />
                No cloud sync
              </div>
              <div className="flex items-center gap-2">
                <Check className="h-4 w-4 text-primary" />
                Open source
              </div>
              <div className="flex items-center gap-2">
                <Check className="h-4 w-4 text-primary" />
                Runs offline
              </div>
            </div>
          </div>
        </div>
      </section>

      {/* CTA */}
      <section className="py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <h2 className="text-2xl font-bold sm:text-3xl">Get Started in Minutes</h2>
            <p className="mt-4 text-muted-foreground">
              Download LocalRouter, add your provider keys, and start routing.
            </p>
            <div className="mt-8">
              <Button asChild size="xl">
                <Link to="/download">
                  Download for Free
                  <ArrowRight className="ml-2 h-4 w-4" />
                </Link>
              </Button>
            </div>
          </div>
        </div>
      </section>
    </div>
  )
}
