import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import Logo from '@/components/Logo'
import { FirewallApprovalDemo } from '@/components/FirewallApprovalDemo'
import {
  Shield,
  ShieldCheck,
  Check,
  Key,
  Route,
  Wrench,
  FlaskConical,
  Sparkles,
  Blocks,
  Store,
  Search,
  Download,
  Terminal,
} from 'lucide-react'

export default function Home() {
  return (
    <div className="flex flex-col">
      {/* Hero */}
      <section className="relative overflow-hidden bg-gradient-to-b from-muted/50 to-background">
        <div className="mx-auto max-w-7xl px-4 py-24 sm:px-6 sm:py-32 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <h1 className="text-4xl font-bold tracking-tight sm:text-5xl lg:text-6xl">
              Local Firewall for
              <br />
              <span className="text-primary">LLM</span>
              {"s, "}
              <span className="text-primary">MCP</span>
              {"s and "}
              <span className="text-primary">Skill</span>
              s.
            </h1>
            <p className="mt-6 text-lg text-muted-foreground sm:text-xl">
              Centralized API key storage with per-client access control. Automatic model failover across providers. Single Unified MCP Gateway aggregating all MCPs and skills.
            </p>
            <div className="mt-10 flex flex-col items-center justify-center gap-4 sm:flex-row">
              <Button asChild size="xl">
                <Link to="/download">
                  Download
                  <span className="ml-3 flex items-center gap-1.5">
                    <img src="/icons/apple.svg" alt="macOS" className="h-4 w-4 fill-current" style={{ filter: 'invert(1)' }} />
                    <img src="/icons/microsoft-windows.svg" alt="Windows" className="h-4 w-4 fill-current" style={{ filter: 'invert(1)' }} />
                    <img src="/icons/penguin.svg" alt="Linux" className="h-4 w-4 fill-current" style={{ filter: 'invert(1)' }} />
                  </span>
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
            <p className="mt-6 text-sm text-muted-foreground sm:text-md italic">
              Built with Claude and a hammer.
            </p>
          </div>

          {/* Connection Graph Visual */}
          <div className="relative mx-auto mt-16 w-full max-w-5xl aspect-[1000/550]">
            {/* SVG Connection Lines */}
            <svg className="absolute inset-0 w-full h-full pointer-events-none" viewBox="0 0 1000 550" preserveAspectRatio="none">
              {/* Left to center connections - nodes at 20%, 34%, 48%, 62% (y: 110, 187, 264, 341) */}
              <path d="M 120 110 Q 280 110 470 275" stroke="url(#gradient-blue)" strokeWidth="2" fill="none" className="animate-pulse-flow" />
              <path d="M 120 187 Q 260 187 470 275" stroke="url(#gradient-blue)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.3s' }} />
              <path d="M 120 264 Q 260 264 470 275" stroke="url(#gradient-blue)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.6s' }} />
              <path d="M 120 341 Q 280 341 470 275" stroke="url(#gradient-blue)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.9s' }} />

              {/* Center to right-top connections - nodes at 18%, 30% (y: 99, 165) */}
              <path d="M 530 275 Q 700 99 880 99" stroke="url(#gradient-violet)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.2s' }} />
              <path d="M 530 275 Q 700 165 880 165" stroke="url(#gradient-violet)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.5s' }} />

              {/* Center to right-bottom connections - nodes at 74%, 86% (y: 407, 473) */}
              <path d="M 530 275 Q 700 407 880 407" stroke="url(#gradient-emerald)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.4s' }} />
              <path d="M 530 275 Q 700 473 880 473" stroke="url(#gradient-emerald)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.7s' }} />

              {/* Center to permission dialog - at 35%, 92% (x: 350, y: 506) */}
              <path d="M 470 310 Q 400 420 350 506" stroke="url(#gradient-orange)" strokeWidth="2" fill="none" className="animate-pulse-flow" style={{ animationDelay: '0.1s' }} />

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
                <linearGradient id="gradient-orange" x1="0%" y1="0%" x2="0%" y2="100%">
                  <stop offset="0%" stopColor="#f97316" stopOpacity="0.5" />
                  <stop offset="50%" stopColor="#f97316" stopOpacity="0.8" />
                  <stop offset="100%" stopColor="#f97316" stopOpacity="0.3" />
                </linearGradient>
              </defs>
            </svg>

            {/* Left side label - anchored from bottom to stay above first node */}
            <div className="absolute left-[5%] top-[20%] -translate-y-full pb-8 text-left">
              <span className="text-[8px] sm:text-xs font-medium text-blue-500 uppercase tracking-wide">Works With</span>
              <h3 className="text-[10px] sm:text-lg font-semibold leading-tight"><i>bring-your-own-key</i> Apps</h3>
            </div>

            {/* Left Apps - spread from 20% to 62% */}
            <div className="absolute left-[5%] top-[20%] -translate-y-1/2">
              <div className="flex items-center gap-1 sm:gap-2 px-1.5 sm:px-3 py-1 sm:py-2 rounded-lg bg-blue-500/10 border border-blue-500/30 shadow-sm backdrop-blur-sm">
                <img src="/icons/cursor.svg" alt="Cursor" className="h-4 w-4 sm:h-6 sm:w-6" />
                <span className="text-xs sm:text-sm font-medium hidden sm:inline">Cursor</span>
              </div>
            </div>
            <div className="absolute left-[5%] top-[34%] -translate-y-1/2">
              <div className="flex items-center gap-1 sm:gap-2 px-1.5 sm:px-3 py-1 sm:py-2 rounded-lg bg-blue-500/10 border border-blue-500/30 shadow-sm backdrop-blur-sm">
                <div className="h-4 w-4 sm:h-6 sm:w-6 rounded bg-gradient-to-br from-indigo-500 to-purple-600 flex items-center justify-center text-white font-bold text-[8px] sm:text-xs">{'</>'}</div>
                <span className="text-xs sm:text-sm font-medium hidden sm:inline">OpenCode</span>
              </div>
            </div>
            <div className="absolute left-[5%] top-[48%] -translate-y-1/2">
              <div className="flex items-center gap-1 sm:gap-2 px-1.5 sm:px-3 py-1 sm:py-2 rounded-lg bg-blue-500/10 border border-blue-500/30 shadow-sm backdrop-blur-sm">
                <img src="/icons/open-webui.png" alt="Open WebUI" className="h-4 w-4 sm:h-6 sm:w-6" />
                <span className="text-xs sm:text-sm font-medium hidden sm:inline">Open WebUI</span>
              </div>
            </div>
            <div className="absolute left-[5%] top-[62%] -translate-y-1/2">
              <div className="flex items-center gap-1 sm:gap-2 px-1.5 sm:px-3 py-1 sm:py-2 rounded-lg bg-blue-500/10 border border-blue-500/30 shadow-sm backdrop-blur-sm">
                <div className="h-4 w-4 sm:h-6 sm:w-6 rounded bg-gradient-to-br from-cyan-500 to-blue-600 flex items-center justify-center text-white font-bold text-[8px] sm:text-xs">C</div>
                <span className="text-xs sm:text-sm font-medium hidden sm:inline">Cline</span>
              </div>
            </div>
            <div className="absolute left-[5%] top-[74%] text-muted-foreground text-xs hidden sm:block">
              + any OpenAI-compatible app
            </div>

            {/* Center: LocalRouter Hub */}
            <div className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 z-10">
              <div className="relative rounded-xl sm:rounded-2xl border-2 border-primary bg-gradient-to-br from-primary/20 to-violet-500/20 p-2 sm:p-5 shadow-2xl backdrop-blur">
                <div className="text-center">
                  <div className="h-8 w-8 sm:h-14 sm:w-14 rounded-lg sm:rounded-xl bg-gradient-to-br from-primary to-violet-600 flex items-center justify-center mx-auto mb-1 sm:mb-2 shadow-lg">
                    <Logo className="h-5 w-auto sm:h-9 text-white" />
                  </div>
                  <div className="font-bold text-[10px] sm:text-base">LocalRouter</div>
                </div>
              </div>
            </div>

            {/* Permission Dialog */}
            <div className="absolute left-[35%] top-[92%] -translate-y-1/2 -translate-x-1/2 z-10">
              <div className="rounded-lg sm:rounded-xl border border-orange-500/30 bg-orange-500/10 p-1 sm:p-4 shadow-sm backdrop-blur-sm">
                <div className="flex items-center gap-1 sm:gap-2 mb-0.5 sm:mb-2">
                  <Shield className="h-2 w-2 sm:h-4 sm:w-4 text-orange-500" />
                  <span className="text-orange-600 dark:text-orange-400 text-[6px] sm:text-xs font-medium uppercase tracking-wide">Permission Request</span>
                </div>
                <div className="text-foreground text-[7px] sm:text-sm mb-1 sm:mb-3">
                  <span className="text-blue-600 dark:text-blue-400 font-medium">Cursor</span> wants <span className="text-emerald-600 dark:text-emerald-400 font-medium">GitHub</span>
                </div>
                <div className="flex gap-0.5 sm:gap-2">
                  <div className="px-1 sm:px-3 py-0.5 sm:py-1 rounded bg-emerald-500/20 text-emerald-600 dark:text-emerald-400 text-[6px] sm:text-xs font-medium">
                    Allow
                  </div>
                  <div className="px-1 sm:px-3 py-0.5 sm:py-1 rounded bg-red-500/20 text-red-600 dark:text-red-400 text-[6px] sm:text-xs font-medium">
                    Deny
                  </div>
                </div>
              </div>
            </div>

            {/* Right side - LLM Providers label - anchored from bottom */}
            <div className="absolute right-[5%] top-[18%] -translate-y-full pb-8 text-right">
              <span className="text-[8px] sm:text-xs font-medium text-violet-500 uppercase tracking-wide">Connects to</span>
              <h3 className="text-[10px] sm:text-lg font-semibold leading-tight">Any LLM Provider</h3>
            </div>

            {/* Right LLM Providers - spread from 18% to 36% */}
            <div className="absolute right-[5%] top-[18%] -translate-y-1/2">
              <div className="flex items-center gap-1 sm:gap-2 px-1.5 sm:px-3 py-1 sm:py-2 rounded-lg bg-violet-500/10 border border-violet-500/30 shadow-sm backdrop-blur-sm">
                <img src="/icons/chatgpt.svg" alt="OpenAI" className="h-4 w-4 sm:h-6 sm:w-6" />
                <span className="text-xs sm:text-sm font-medium hidden sm:inline">OpenAI</span>
              </div>
            </div>
            <div className="absolute right-[5%] top-[30%] -translate-y-1/2">
              <div className="flex items-center gap-1 sm:gap-2 px-1.5 sm:px-3 py-1 sm:py-2 rounded-lg bg-violet-500/10 border border-violet-500/30 shadow-sm backdrop-blur-sm">
                <img src="/icons/ollama.svg" alt="Ollama" className="h-4 w-4 sm:h-6 sm:w-6" />
                <span className="text-xs sm:text-sm font-medium hidden sm:inline">Ollama</span>
              </div>
            </div>
            <div className="absolute right-[5%] top-[40%] text-muted-foreground text-xs text-right hidden sm:block">
              + Anthropic, Gemini, more...
            </div>

            {/* Right side - MCP Servers label - anchored from bottom */}
            <div className="absolute right-[5%] top-[74%] -translate-y-full pb-8 text-right">
              <span className="text-[8px] sm:text-xs font-medium text-emerald-500 uppercase tracking-wide">Connects to</span>
              <h3 className="text-[10px] sm:text-lg font-semibold leading-tight">Any MCP / Skill</h3>
            </div>

            {/* Right MCP Servers - spread from 74% to 96% */}
            <div className="absolute right-[5%] top-[74%] -translate-y-1/2">
              <div className="flex items-center gap-1 sm:gap-2 px-1.5 sm:px-3 py-1 sm:py-2 rounded-lg bg-emerald-500/10 border border-emerald-500/30 shadow-sm backdrop-blur-sm">
                <img src="/icons/github.svg" alt="GitHub" className="h-4 w-4 sm:h-6 sm:w-6" />
                <span className="text-xs sm:text-sm font-medium hidden sm:inline">GitHub</span>
              </div>
            </div>
            <div className="absolute right-[5%] top-[86%] -translate-y-1/2">
              <div className="flex items-center gap-1 sm:gap-2 px-1.5 sm:px-3 py-1 sm:py-2 rounded-lg bg-emerald-500/10 border border-emerald-500/30 shadow-sm backdrop-blur-sm">
                <Blocks className="h-4 w-4 sm:h-6 sm:w-6 text-emerald-400" />
                <span className="text-xs sm:text-sm font-medium hidden sm:inline">Project Manager</span>
              </div>
            </div>
            <div className="absolute right-[5%] top-[96%] text-muted-foreground text-xs text-right hidden sm:block">
              + Jira, Slack, more...
            </div>
          </div>
        </div>
      </section>

      {/* Windows XP Demo - Full Width */}
      <section className="border-b">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8 py-8 text-center">
          <h2 className="text-2xl font-bold sm:text-3xl">App Demo</h2>
        </div>
        <iframe
          src="/winxp/index.html"
          className="w-full bg-[#235cdc]"
          style={{ height: '700px' }}
          title="LocalRouter Windows XP Demo"
        />
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
                Centralized Credential Store
              </h2>
              <p className="mt-4 text-lg text-muted-foreground">
                Store provider API keys once. Issue per-client keys with scoped permissions. All secrets encrypted in OS keychain.
              </p>
              <ul className="mt-8 space-y-4">
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-blue-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Multiple auth methods</span>
                    <p className="text-sm text-muted-foreground">API key, OAuth, or STDIO-based authentication per client</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-blue-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Scoped access</span>
                    <p className="text-sm text-muted-foreground">Restrict each client to specific models, providers, and MCP servers</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-blue-500 mt-0.5" />
                  <div>
                    <span className="font-medium">OS keychain storage</span>
                    <p className="text-sm text-muted-foreground">Secrets encrypted via macOS Keychain, Windows Credential Manager, or libsecret</p>
                  </div>
                </li>
              </ul>
            </div>
            {/* Visual: Auth Flow Diagram */}
            <div className="relative overflow-x-auto">
              <div className="rounded-xl border-2 border-slate-700 bg-gradient-to-br from-slate-900 to-slate-800 p-6 shadow-2xl min-w-[480px]">
                {/* Five-column flow layout: Clients → Connector → LocalRouter → Connector → Providers */}
                <div className="flex items-stretch gap-2">
                  {/* Left: Clients */}
                  <div className="flex flex-col gap-2">
                    <div className="text-center mb-1">
                      <span className="text-blue-400 text-[10px] font-medium uppercase tracking-wide">Clients</span>
                    </div>
                    {/* Client 1 */}
                    <div className="flex items-center gap-2 px-2 py-1.5 rounded-lg bg-blue-500/10 border border-blue-500/30">
                      <img src="/icons/cursor.svg" alt="Cursor" className="h-4 w-4" />
                      <span className="text-white text-xs">Cursor</span>
                      <div className="ml-auto flex items-center gap-1">
                        <Key className="h-3 w-3 text-blue-400" />
                        <span className="text-blue-400 text-[10px] font-mono">key-1</span>
                      </div>
                    </div>
                    {/* Client 2 */}
                    <div className="flex items-center gap-2 px-2 py-1.5 rounded-lg bg-blue-500/10 border border-blue-500/30">
                      <div className="h-4 w-4 rounded bg-gradient-to-br from-cyan-500 to-blue-600 flex items-center justify-center text-white font-bold text-[8px]">C</div>
                      <span className="text-white text-xs">Cline</span>
                      <div className="ml-auto flex items-center gap-1">
                        <Key className="h-3 w-3 text-blue-400" />
                        <span className="text-blue-400 text-[10px] font-mono">key-2</span>
                      </div>
                    </div>
                    {/* Client 3 */}
                    <div className="flex items-center gap-2 px-2 py-1.5 rounded-lg bg-blue-500/10 border border-blue-500/30">
                      <img src="/icons/open-webui.png" alt="Open WebUI" className="h-4 w-4" />
                      <span className="text-white text-xs">WebUI</span>
                      <div className="ml-auto flex items-center gap-1">
                        <Key className="h-3 w-3 text-blue-400" />
                        <span className="text-blue-400 text-[10px] font-mono">key-3</span>
                      </div>
                    </div>
                  </div>

                  {/* Left Connector */}
                  <div className="flex flex-col items-center justify-center">
                    <div className="flex items-center gap-1">
                      <div className="w-4 h-0.5 bg-blue-500/50" />
                      <div className="px-1 py-0.5 rounded bg-blue-900/50 text-blue-400 text-[8px] font-mono">Auth</div>
                      <div className="w-4 h-0.5 bg-blue-500/50" />
                    </div>
                  </div>

                  {/* Center: LocalRouter Hub */}
                  <div className="flex flex-col items-center justify-center">
                    <div className="rounded-xl border-2 border-primary bg-gradient-to-br from-primary/20 to-violet-500/20 p-3 shadow-lg">
                      <div className="h-10 w-10 rounded-lg bg-gradient-to-br from-primary to-violet-600 flex items-center justify-center mx-auto mb-1">
                        <Shield className="h-5 w-5 text-white" />
                      </div>
                      <div className="text-white text-xs font-semibold text-center">LocalRouter</div>
                    </div>
                  </div>

                  {/* Right Connector */}
                  <div className="flex flex-col items-center justify-center">
                    <div className="flex items-center gap-1">
                      <div className="w-4 h-0.5 bg-violet-500/50" />
                      <div className="px-1 py-0.5 rounded bg-violet-900/50 text-violet-400 text-[8px] font-mono">API</div>
                      <div className="w-4 h-0.5 bg-violet-500/50" />
                    </div>
                  </div>

                  {/* Right: Providers */}
                  <div className="flex flex-col gap-2">
                    <div className="text-center mb-1">
                      <span className="text-violet-400 text-[10px] font-medium uppercase tracking-wide">Providers</span>
                    </div>
                    {/* Provider 1 */}
                    <div className="flex items-center gap-2 px-2 py-1.5 rounded-lg bg-violet-500/10 border border-violet-500/30">
                      <div className="flex items-center gap-1">
                        <Key className="h-3 w-3 text-violet-400" />
                        <span className="text-violet-400 text-[10px] font-mono">sk-...4f</span>
                      </div>
                      <img src="/icons/chatgpt.svg" alt="OpenAI" className="h-4 w-4 ml-auto" />
                      <span className="text-white text-xs">OpenAI</span>
                    </div>
                    {/* Provider 2 */}
                    <div className="flex items-center gap-2 px-2 py-1.5 rounded-lg bg-violet-500/10 border border-violet-500/30">
                      <div className="flex items-center gap-1">
                        <Key className="h-3 w-3 text-violet-400" />
                        <span className="text-violet-400 text-[10px] font-mono">sk-...8k</span>
                      </div>
                      <img src="/icons/anthropic.svg" alt="Anthropic" className="h-4 w-4 ml-auto" />
                      <span className="text-white text-xs">Anthropic</span>
                    </div>
                    {/* Provider 3 */}
                    <div className="flex items-center gap-2 px-2 py-1.5 rounded-lg bg-violet-500/10 border border-violet-500/30">
                      <div className="flex items-center gap-1">
                        <Key className="h-3 w-3 text-violet-400" />
                        <span className="text-violet-400 text-[10px] font-mono">gsk-...2n</span>
                      </div>
                      <div className="h-4 w-4 rounded bg-orange-500/30 flex items-center justify-center text-orange-400 text-[8px] font-bold ml-auto">G</div>
                      <span className="text-white text-xs">Groq</span>
                    </div>
                  </div>
                </div>

                {/* Bottom: System Keychain */}
                <div className="mt-5 pt-4 border-t border-slate-700">
                  <div className="flex items-center justify-center gap-3">
                    <div className="rounded-lg bg-amber-500/10 border border-amber-500/30 px-4 py-2 flex items-center gap-3">
                      <div className="h-8 w-8 rounded-lg bg-gradient-to-br from-amber-500 to-yellow-600 flex items-center justify-center">
                        <svg className="h-4 w-4 text-white" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                          <rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
                          <path d="M7 11V7a5 5 0 0 1 10 0v4" />
                        </svg>
                      </div>
                      <div>
                        <div className="text-white text-xs font-medium">System Keychain</div>
                        <div className="text-amber-400 text-[10px]">All keys securely stored</div>
                      </div>
                      <div className="flex items-center gap-1.5 ml-2">
                        <div className="h-2 w-2 rounded-full bg-emerald-500 animate-pulse" />
                        <span className="text-emerald-400 text-[10px]">Encrypted</span>
                      </div>
                    </div>
                  </div>
                  {/* Key indicators */}
                  <div className="flex justify-center gap-8 mt-3">
                    <div className="flex items-center gap-1">
                      <Key className="h-3 w-3 text-blue-400" />
                      <span className="text-slate-500 text-[10px]">Client Keys</span>
                    </div>
                    <div className="flex items-center gap-1">
                      <Key className="h-3 w-3 text-violet-400" />
                      <span className="text-slate-500 text-[10px]">Provider Keys</span>
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
            {/* Visual: Decision Tree Flowchart */}
            <div className="relative order-2 lg:order-1">
              <div className="rounded-xl border bg-gradient-to-br from-violet-950 to-slate-900 p-6 shadow-2xl">
                {/* SVG Flowchart */}
                <svg viewBox="0 0 400 340" className="w-full h-auto" preserveAspectRatio="xMidYMid meet">
                  {/* Definitions */}
                  <defs>
                    <marker id="arrowhead" markerWidth="8" markerHeight="6" refX="8" refY="3" orient="auto">
                      <polygon points="0 0, 8 3, 0 6" fill="#8b5cf6" fillOpacity="0.7" />
                    </marker>
                    <marker id="arrowhead-amber" markerWidth="8" markerHeight="6" refX="8" refY="3" orient="auto">
                      <polygon points="0 0, 8 3, 0 6" fill="#f59e0b" fillOpacity="0.7" />
                    </marker>
                    <marker id="arrowhead-emerald" markerWidth="8" markerHeight="6" refX="8" refY="3" orient="auto">
                      <polygon points="0 0, 8 3, 0 6" fill="#10b981" fillOpacity="0.7" />
                    </marker>
                    <marker id="arrowhead-red" markerWidth="8" markerHeight="6" refX="8" refY="3" orient="auto">
                      <polygon points="0 0, 8 3, 0 6" fill="#ef4444" fillOpacity="0.7" />
                    </marker>
                    <marker id="arrowhead-slate" markerWidth="8" markerHeight="6" refX="8" refY="3" orient="auto">
                      <polygon points="0 0, 8 3, 0 6" fill="#64748b" fillOpacity="0.7" />
                    </marker>
                  </defs>

                  {/* Incoming Request */}
                  <rect x="140" y="8" width="120" height="28" rx="6" fill="rgba(255,255,255,0.1)" stroke="rgba(255,255,255,0.2)" />
                  <text x="200" y="26" textAnchor="middle" fill="white" fontSize="11" fontFamily="system-ui">
                    <tspan fill="#a78bfa">model:</tspan> &quot;auto&quot;
                  </text>

                  {/* Arrow to decision */}
                  <line x1="200" y1="36" x2="200" y2="54" stroke="#8b5cf6" strokeWidth="2" markerEnd="url(#arrowhead)" />

                  {/* Decision Diamond */}
                  <polygon points="200,58 260,88 200,118 140,88" fill="rgba(139,92,246,0.2)" stroke="rgba(139,92,246,0.5)" strokeWidth="2" />
                  <text x="200" y="85" textAnchor="middle" fill="#c4b5fd" fontSize="9" fontFamily="system-ui">Complex?</text>
                  <text x="200" y="97" textAnchor="middle" fill="#a78bfa" fontSize="8" fontFamily="system-ui">(RouteLLM)</text>

                  {/* Yes branch - to Strong */}
                  <line x1="140" y1="88" x2="80" y2="88" stroke="#10b981" strokeWidth="2" />
                  <line x1="80" y1="88" x2="80" y2="130" stroke="#10b981" strokeWidth="2" markerEnd="url(#arrowhead-emerald)" />
                  <text x="108" y="82" textAnchor="middle" fill="#10b981" fontSize="9" fontWeight="600">Yes</text>

                  {/* No branch - to Weak */}
                  <line x1="260" y1="88" x2="320" y2="88" stroke="#f59e0b" strokeWidth="2" />
                  <line x1="320" y1="88" x2="320" y2="130" stroke="#f59e0b" strokeWidth="2" markerEnd="url(#arrowhead-amber)" />
                  <text x="292" y="82" textAnchor="middle" fill="#f59e0b" fontSize="9" fontWeight="600">No</text>

                  {/* Strong Models Box */}
                  <rect x="20" y="134" width="120" height="32" rx="6" fill="rgba(16,185,129,0.15)" stroke="rgba(16,185,129,0.4)" strokeWidth="1.5" />
                  <text x="80" y="145" textAnchor="middle" fill="#34d399" fontSize="8" fontWeight="600" textTransform="uppercase">STRONG</text>
                  <text x="80" y="158" textAnchor="middle" fill="white" fontSize="10">GPT-5.2 / Opus</text>

                  {/* Weak Models Box */}
                  <rect x="260" y="134" width="120" height="32" rx="6" fill="rgba(245,158,11,0.15)" stroke="rgba(245,158,11,0.4)" strokeWidth="1.5" />
                  <text x="320" y="145" textAnchor="middle" fill="#fbbf24" fontSize="8" fontWeight="600" textTransform="uppercase">WEAK</text>
                  <text x="320" y="158" textAnchor="middle" fill="white" fontSize="10">GPT-4o mini / Haiku</text>

                  {/* Fallback arrows from Strong/Weak to Secondary Provider */}
                  {/* Strong fail arrow */}
                  <line x1="80" y1="166" x2="80" y2="194" stroke="#ef4444" strokeWidth="1.5" strokeDasharray="4,2" markerEnd="url(#arrowhead-red)" />
                  <text x="92" y="185" fill="#f87171" fontSize="7">fail</text>

                  {/* Weak fail arrow */}
                  <line x1="320" y1="166" x2="320" y2="194" stroke="#ef4444" strokeWidth="1.5" strokeDasharray="4,2" markerEnd="url(#arrowhead-red)" />
                  <text x="332" y="185" fill="#f87171" fontSize="7">fail</text>

                  {/* Secondary Provider - Strong side */}
                  <rect x="20" y="198" width="120" height="32" rx="6" fill="rgba(59,130,246,0.15)" stroke="rgba(59,130,246,0.4)" strokeWidth="1.5" />
                  <text x="80" y="209" textAnchor="middle" fill="#60a5fa" fontSize="8" fontWeight="600" textTransform="uppercase">FALLBACK PROVIDER</text>
                  <text x="80" y="222" textAnchor="middle" fill="white" fontSize="10">Anthropic / Gemini</text>

                  {/* Secondary Provider - Weak side */}
                  <rect x="260" y="198" width="120" height="32" rx="6" fill="rgba(59,130,246,0.15)" stroke="rgba(59,130,246,0.4)" strokeWidth="1.5" />
                  <text x="320" y="209" textAnchor="middle" fill="#60a5fa" fontSize="8" fontWeight="600" textTransform="uppercase">FALLBACK PROVIDER</text>
                  <text x="320" y="222" textAnchor="middle" fill="white" fontSize="10">Anthropic / Groq</text>

                  {/* Fallback arrows to Offline */}
                  {/* Strong side to offline */}
                  <line x1="80" y1="230" x2="80" y2="258" stroke="#ef4444" strokeWidth="1.5" strokeDasharray="4,2" markerEnd="url(#arrowhead-red)" />
                  <text x="92" y="249" fill="#f87171" fontSize="7">fail</text>

                  {/* Weak side to offline */}
                  <line x1="320" y1="230" x2="320" y2="258" stroke="#ef4444" strokeWidth="1.5" strokeDasharray="4,2" markerEnd="url(#arrowhead-red)" />
                  <text x="332" y="249" fill="#f87171" fontSize="7">fail</text>

                  {/* Offline Models - Strong side */}
                  <rect x="20" y="262" width="120" height="32" rx="6" fill="rgba(100,116,139,0.2)" stroke="rgba(100,116,139,0.4)" strokeWidth="1.5" />
                  <text x="80" y="273" textAnchor="middle" fill="#94a3b8" fontSize="8" fontWeight="600" textTransform="uppercase">OFFLINE FALLBACK</text>
                  <text x="80" y="286" textAnchor="middle" fill="white" fontSize="10">Llama 4 405B (local)</text>

                  {/* Offline Models - Weak side */}
                  <rect x="260" y="262" width="120" height="32" rx="6" fill="rgba(100,116,139,0.2)" stroke="rgba(100,116,139,0.4)" strokeWidth="1.5" />
                  <text x="320" y="273" textAnchor="middle" fill="#94a3b8" fontSize="8" fontWeight="600" textTransform="uppercase">OFFLINE FALLBACK</text>
                  <text x="320" y="286" textAnchor="middle" fill="white" fontSize="10">Llama 3.2 3B (local)</text>

                  {/* Success checkmarks */}
                  <circle cx="150" cy="150" r="8" fill="rgba(16,185,129,0.3)" />
                  <text x="150" y="154" textAnchor="middle" fill="#10b981" fontSize="10">✓</text>

                  <circle cx="250" cy="150" r="8" fill="rgba(245,158,11,0.3)" />
                  <text x="250" y="154" textAnchor="middle" fill="#fbbf24" fontSize="10">✓</text>

                  <circle cx="150" cy="214" r="8" fill="rgba(59,130,246,0.3)" />
                  <text x="150" y="218" textAnchor="middle" fill="#60a5fa" fontSize="10">✓</text>

                  <circle cx="250" cy="214" r="8" fill="rgba(59,130,246,0.3)" />
                  <text x="250" y="218" textAnchor="middle" fill="#60a5fa" fontSize="10">✓</text>

                  <circle cx="150" cy="278" r="8" fill="rgba(100,116,139,0.3)" />
                  <text x="150" y="282" textAnchor="middle" fill="#94a3b8" fontSize="10">✓</text>

                  <circle cx="250" cy="278" r="8" fill="rgba(100,116,139,0.3)" />
                  <text x="250" y="282" textAnchor="middle" fill="#94a3b8" fontSize="10">✓</text>
                </svg>

                {/* Legend */}
                <div className="mt-4 pt-3 border-t border-white/10 flex flex-wrap justify-center gap-3">
                  <div className="flex items-center gap-1.5">
                    <div className="h-2 w-2 rounded-full bg-emerald-500" />
                    <span className="text-slate-400 text-[10px]">Strong Model</span>
                  </div>
                  <div className="flex items-center gap-1.5">
                    <div className="h-2 w-2 rounded-full bg-amber-500" />
                    <span className="text-slate-400 text-[10px]">Weak Model</span>
                  </div>
                  <div className="flex items-center gap-1.5">
                    <div className="h-2 w-2 rounded-full bg-blue-500" />
                    <span className="text-slate-400 text-[10px]">Provider Fallback</span>
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
                Automatic Model Routing
              </h2>
              <p className="mt-4 text-lg text-muted-foreground">
                Request <code className="text-sm bg-muted px-1 rounded">model: &quot;auto&quot;</code> to route by prompt complexity. Automatic failover on rate limits, outages, or policy errors.
              </p>
              <ul className="mt-8 space-y-4">
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-violet-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Prompt complexity routing</span>
                    <p className="text-sm text-muted-foreground">RouteLLM classifier routes complex prompts to capable models, simple ones to fast/cheap models</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-violet-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Offline fallback</span>
                    <p className="text-sm text-muted-foreground">Falls back to local Ollama/LMStudio models when cloud providers are unreachable</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-violet-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Provider failover chain</span>
                    <p className="text-sm text-muted-foreground">Configure primary → secondary → offline provider sequence per model tier</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <FlaskConical className="h-5 w-5 shrink-0 text-amber-500 mt-0.5" />
                  <div>
                    <span className="font-medium text-amber-600 dark:text-amber-400">Experimental: Strong/Weak classification</span>
                    <p className="text-sm text-muted-foreground">ML classifier determines prompt complexity to select strong vs weak model tier</p>
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
                Reverse proxy merging multiple MCP servers into one endpoint. Tools from all servers appear in a single namespace with client-level access control.
              </p>
              <ul className="mt-8 space-y-4">
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-emerald-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Unified tool namespace</span>
                    <p className="text-sm text-muted-foreground">All MCP server tools aggregated under one STDIO or SSE endpoint</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-emerald-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Client-level server restrictions</span>
                    <p className="text-sm text-muted-foreground">Whitelist specific MCP servers per client—others are hidden</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <FlaskConical className="h-5 w-5 shrink-0 text-amber-500 mt-0.5" />
                  <div>
                    <span className="font-medium text-amber-600 dark:text-amber-400">Experimental: Deferred tool loading</span>
                    <p className="text-sm text-muted-foreground">Load tool schemas on-demand to reduce context token usage</p>
                  </div>
                </li>
              </ul>
            </div>
            {/* Visual: MCP Architecture */}
            <div className="relative overflow-x-auto">
              <div className="rounded-xl border bg-gradient-to-br from-emerald-950 to-slate-900 p-6 shadow-2xl min-w-[480px]">
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
                    {/* Skill Server */}
                    <div className="rounded-lg bg-violet-500/10 border border-violet-500/30 p-2 flex items-center gap-2">
                      <Blocks className="h-5 w-5 text-violet-400" />
                      <div>
                        <div className="text-white text-xs font-medium">Project Manager</div>
                        <div className="text-violet-400 text-[10px]">Skill • Multi-step</div>
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

      {/* Feature 4: Unified Skills */}
      <section className="border-b py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="grid gap-12 lg:grid-cols-2 lg:gap-16 items-center">
            {/* Visual: Skills Architecture */}
            <div className="relative order-2 lg:order-1 overflow-x-auto">
              <div className="rounded-xl border bg-gradient-to-br from-violet-950 to-slate-900 p-6 shadow-2xl min-w-[480px]">
                {/* Three-column layout: App → Gateway → Skills */}
                <div className="flex items-stretch gap-3">
                  {/* App */}
                  <div className="flex flex-col items-center justify-center">
                    <div className="rounded-lg bg-blue-500/20 border border-blue-500/30 p-3 text-center">
                      <div className="h-8 w-8 rounded bg-blue-500/30 flex items-center justify-center mx-auto mb-1">
                        <span className="text-blue-400 text-sm font-bold">{'</>'}</span>
                      </div>
                      <div className="text-white text-xs font-medium">App</div>
                      <div className="text-blue-400 text-[10px]">Claude Code</div>
                    </div>
                  </div>

                  {/* Connection */}
                  <div className="flex flex-col items-center justify-center">
                    <div className="flex items-center gap-1">
                      <div className="w-6 h-0.5 bg-slate-500" />
                      <div className="px-1.5 py-0.5 rounded bg-slate-700 text-slate-400 text-[9px] font-mono">MCP</div>
                      <div className="w-6 h-0.5 bg-slate-500" />
                    </div>
                  </div>

                  {/* Gateway */}
                  <div className="flex flex-col items-center justify-center">
                    <div className="rounded-lg bg-violet-500/20 border-2 border-violet-500/50 p-3 text-center">
                      <Sparkles className="h-6 w-6 text-violet-400 mx-auto mb-1" />
                      <div className="text-white text-xs font-medium">Skills</div>
                      <div className="text-violet-400 text-[10px]">Gateway</div>
                    </div>
                  </div>

                  {/* Arrow */}
                  <div className="flex flex-col items-center justify-center">
                    <div className="w-3 h-0.5 bg-violet-600" />
                  </div>

                  {/* Skills */}
                  <div className="flex flex-col gap-2">
                    <div className="rounded-lg bg-violet-500/10 border border-violet-500/30 p-2 flex items-center gap-2">
                      <Blocks className="h-4 w-4 text-violet-400" />
                      <div>
                        <div className="text-white text-xs font-medium">Web Search</div>
                        <div className="text-violet-400 text-[10px]">Multi-step</div>
                      </div>
                    </div>
                    <div className="rounded-lg bg-violet-500/10 border border-violet-500/30 p-2 flex items-center gap-2">
                      <Blocks className="h-4 w-4 text-violet-400" />
                      <div>
                        <div className="text-white text-xs font-medium">Code Review</div>
                        <div className="text-violet-400 text-[10px]">Multi-step</div>
                      </div>
                    </div>
                    <div className="rounded-lg bg-violet-500/10 border border-violet-500/30 p-2 flex items-center gap-2">
                      <Blocks className="h-4 w-4 text-violet-400" />
                      <div>
                        <div className="text-white text-xs font-medium">Summarize</div>
                        <div className="text-violet-400 text-[10px]">Multi-step</div>
                      </div>
                    </div>
                  </div>
                </div>

                {/* Legend */}
                <div className="mt-4 pt-3 border-t border-white/10 flex justify-center gap-4">
                  <div className="flex items-center gap-1.5">
                    <div className="h-2 w-2 rounded-full bg-violet-500" />
                    <span className="text-slate-400 text-[10px]">Skills (multi-step tools)</span>
                  </div>
                  <div className="flex items-center gap-1.5">
                    <div className="h-2 w-2 rounded-full bg-slate-500" />
                    <span className="text-slate-400 text-[10px]">MCP transport</span>
                  </div>
                </div>
              </div>
            </div>

            <div className="order-1 lg:order-2">
              <div className="flex items-center gap-2 mb-4">
                <Sparkles className="h-5 w-5 text-violet-500" />
                <span className="text-sm font-medium text-violet-500 uppercase tracking-wide">Skills</span>
              </div>
              <h2 className="text-3xl font-bold tracking-tight sm:text-4xl">
                Skills as MCP Tools
              </h2>
              <p className="mt-4 text-lg text-muted-foreground">
                Multi-step workflows exposed as callable MCP tools. Web search, code review, document summarization—each skill chains multiple operations behind a single tool call.
              </p>
              <ul className="mt-8 space-y-4">
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-violet-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Composite workflows</span>
                    <p className="text-sm text-muted-foreground">Each skill orchestrates multiple sub-operations—API calls, file I/O, shell commands—as a single atomic tool</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-violet-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Per-client skill whitelist</span>
                    <p className="text-sm text-muted-foreground">Assign specific skills to each client—others remain inaccessible</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-violet-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Standard MCP interface</span>
                    <p className="text-sm text-muted-foreground">Skills exposed via MCP protocol—compatible with any MCP client without custom integration</p>
                  </div>
                </li>
              </ul>
            </div>
          </div>
        </div>
      </section>

      {/* Feature 5: Firewall */}
      <section className="border-b py-16 sm:py-24 bg-muted/30">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="grid gap-12 lg:grid-cols-2 lg:gap-16 items-center">
            {/* Visual: Firewall Architecture */}
            <div className="relative">
              <div className="relative w-full aspect-[4/3]">
                {/* SVG Connection Lines */}
                <svg className="absolute inset-0 w-full h-full pointer-events-none" viewBox="0 0 400 300" preserveAspectRatio="xMidYMid meet">
                  {/* Left clients to firewall */}
                  {/* Cursor (front) right edge → Firewall left edge */}
                  <path d="M 88 66 Q 120 66 155 112" stroke="url(#fw-gradient-blue)" strokeWidth="2" fill="none" opacity="0.7" />
                  {/* Cline (middle) right edge → Firewall left edge */}
                  <path d="M 94 78 Q 125 78 155 117" stroke="url(#fw-gradient-blue)" strokeWidth="2" fill="none" opacity="0.7" />
                  {/* WebUI (back) right edge → Firewall left edge */}
                  <path d="M 100 90 Q 128 90 155 122" stroke="url(#fw-gradient-blue)" strokeWidth="2" fill="none" opacity="0.7" />

                  {/* Firewall to right resources */}
                  {/* Firewall right edge → LLMs (front) left edge */}
                  <path d="M 245 109 Q 274 66 302 66" stroke="url(#fw-gradient-multi)" strokeWidth="2" fill="none" opacity="0.7" />
                  {/* Firewall right edge → MCPs left edge */}
                  <path d="M 245 114 Q 274 78 302 78" stroke="url(#fw-gradient-multi)" strokeWidth="2" fill="none" opacity="0.7" />
                  {/* Firewall right edge → Skills left edge */}
                  <path d="M 245 119 Q 270 90 296 90" stroke="url(#fw-gradient-multi)" strokeWidth="2" fill="none" opacity="0.7" />
                  {/* Firewall right edge → Marketplace left edge */}
                  <path d="M 245 124 Q 268 102 290 102" stroke="url(#fw-gradient-multi)" strokeWidth="2" fill="none" opacity="0.7" />

                  {/* Firewall down to permission dialog */}
                  <path d="M 200 159 L 200 204" stroke="url(#fw-gradient-amber)" strokeWidth="2.5" fill="none" />
                  <circle cx="200" cy="204" r="3" fill="#f59e0b" />

                  <defs>
                    <linearGradient id="fw-gradient-blue" x1="0%" y1="0%" x2="100%" y2="0%">
                      <stop offset="0%" stopColor="#3b82f6" stopOpacity="0.8" />
                      <stop offset="100%" stopColor="#f59e0b" stopOpacity="0.5" />
                    </linearGradient>
                    <linearGradient id="fw-gradient-multi" x1="0%" y1="0%" x2="100%" y2="0%">
                      <stop offset="0%" stopColor="#f59e0b" stopOpacity="0.5" />
                      <stop offset="100%" stopColor="#8b5cf6" stopOpacity="0.8" />
                    </linearGradient>
                    <linearGradient id="fw-gradient-amber" x1="0%" y1="0%" x2="0%" y2="100%">
                      <stop offset="0%" stopColor="#f59e0b" stopOpacity="0.8" />
                      <stop offset="100%" stopColor="#f59e0b" stopOpacity="0.5" />
                    </linearGradient>
                  </defs>
                </svg>

                {/* Left: Clients Stack (diagonally offset) */}
                <div className="absolute left-[2%] top-[12%]">
                  <div className="text-[9px] sm:text-xs font-medium text-blue-500 uppercase tracking-wide mb-2">Clients</div>
                  <div className="relative">
                    {/* Client 3 (back) */}
                    <div className="absolute top-8 left-4 flex items-center gap-1.5 px-2 py-1.5 rounded-lg bg-blue-500/10 border border-blue-500/30 shadow-sm backdrop-blur-sm">
                      <img src="/icons/open-webui.png" alt="Open WebUI" className="h-4 w-4" />
                      <span className="text-[10px] sm:text-xs font-medium text-white/80">WebUI</span>
                    </div>
                    {/* Client 2 (middle) */}
                    <div className="absolute top-4 left-2 flex items-center gap-1.5 px-2 py-1.5 rounded-lg bg-blue-500/15 border border-blue-500/40 shadow-sm backdrop-blur-sm">
                      <div className="h-4 w-4 rounded bg-gradient-to-br from-cyan-500 to-blue-600 flex items-center justify-center text-white font-bold text-[7px]">C</div>
                      <span className="text-[10px] sm:text-xs font-medium text-white/90">Cline</span>
                    </div>
                    {/* Client 1 (front) */}
                    <div className="relative flex items-center gap-1.5 px-2 py-1.5 rounded-lg bg-blue-500/20 border border-blue-500/50 shadow-lg backdrop-blur-sm">
                      <img src="/icons/cursor.svg" alt="Cursor" className="h-4 w-4" />
                      <span className="text-[10px] sm:text-xs font-medium text-white">Cursor</span>
                    </div>
                  </div>
                </div>

                {/* Center: LocalRouter Firewall */}
                <div className="absolute left-1/2 top-[28%] -translate-x-1/2">
                  <div className="rounded-xl border-2 border-amber-500/50 bg-gradient-to-br from-amber-500/20 to-orange-500/20 p-3 shadow-xl backdrop-blur-sm">
                    <div className="text-center">
                      <div className="h-10 w-10 rounded-lg bg-gradient-to-br from-amber-500 to-orange-600 flex items-center justify-center mx-auto mb-1 shadow-lg">
                        <Shield className="h-5 w-5 text-white" />
                      </div>
                      <div className="font-bold text-xs text-white">LocalRouter</div>
                      <div className="text-[9px] text-amber-400">Firewall</div>
                    </div>
                  </div>
                </div>

                {/* Right: Resources Stack (diagonally offset) */}
                <div className="absolute right-[2%] top-[12%]">
                  <div className="text-[9px] sm:text-xs font-medium text-violet-500 uppercase tracking-wide mb-2 text-right">Resources</div>
                  <div className="relative">
                    {/* Resource 4 (back) */}
                    <div className="absolute top-12 -left-4 flex items-center gap-1.5 px-2 py-1.5 rounded-lg bg-cyan-500/10 border border-cyan-500/30 shadow-sm backdrop-blur-sm">
                      <Store className="h-4 w-4 text-cyan-400" />
                      <span className="text-[10px] sm:text-xs font-medium text-white/80">Marketplace</span>
                    </div>
                    {/* Resource 3 */}
                    <div className="absolute top-8 -left-2 flex items-center gap-1.5 px-2 py-1.5 rounded-lg bg-violet-500/10 border border-violet-500/30 shadow-sm backdrop-blur-sm">
                      <Sparkles className="h-4 w-4 text-violet-400" />
                      <span className="text-[10px] sm:text-xs font-medium text-white/85">Skills</span>
                    </div>
                    {/* Resource 2 */}
                    <div className="absolute top-4 left-0 flex items-center gap-1.5 px-2 py-1.5 rounded-lg bg-emerald-500/15 border border-emerald-500/40 shadow-sm backdrop-blur-sm">
                      <Wrench className="h-4 w-4 text-emerald-400" />
                      <span className="text-[10px] sm:text-xs font-medium text-white/90">MCPs</span>
                    </div>
                    {/* Resource 1 (front) */}
                    <div className="relative flex items-center gap-1.5 px-2 py-1.5 rounded-lg bg-violet-500/20 border border-violet-500/50 shadow-lg backdrop-blur-sm">
                      <img src="/icons/chatgpt.svg" alt="LLM" className="h-4 w-4" />
                      <span className="text-[10px] sm:text-xs font-medium text-white">LLMs</span>
                    </div>
                  </div>
                </div>

                {/* Bottom: Permission Dialog */}
                <div className="absolute left-1/2 top-[68%] -translate-x-1/2 scale-75 origin-top">
                  <FirewallApprovalDemo />
                </div>
              </div>
            </div>

            <div>
              <div className="flex items-center gap-2 mb-4">
                <ShieldCheck className="h-5 w-5 text-amber-500" />
                <span className="text-sm font-medium text-amber-500 uppercase tracking-wide">Firewall</span>
              </div>
              <h2 className="text-3xl font-bold tracking-tight sm:text-4xl">
                Runtime Approval Firewall
              </h2>
              <p className="mt-4 text-lg text-muted-foreground">
                Interactive approval prompts for sensitive operations. Allow once, allow for session, or deny. Configurable per client, model, MCP server, and skill.
              </p>
              <ul className="mt-8 space-y-4">
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-amber-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Model/provider gating</span>
                    <p className="text-sm text-muted-foreground">Require approval before a client can access specific models or providers</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-amber-500 mt-0.5" />
                  <div>
                    <span className="font-medium">MCP operation approval</span>
                    <p className="text-sm text-muted-foreground">Gate tool calls, resource reads, and prompt injections on a per-server basis</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-amber-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Skill execution gates</span>
                    <p className="text-sm text-muted-foreground">Skills with shell/script actions require explicit user confirmation</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-amber-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Installation approval</span>
                    <p className="text-sm text-muted-foreground">Marketplace installs triggered by AI require user confirmation before execution</p>
                  </div>
                </li>
              </ul>
            </div>
          </div>
        </div>
      </section>

      {/* Feature 6: Marketplace */}
      <section className="border-b py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="grid gap-12 lg:grid-cols-2 lg:gap-16 items-center">
            <div className="order-1 lg:order-2">
              <div className="flex items-center gap-2 mb-4">
                <Store className="h-5 w-5 text-cyan-500" />
                <span className="text-sm font-medium text-cyan-500 uppercase tracking-wide">Marketplace</span>
              </div>
              <h2 className="text-3xl font-bold tracking-tight sm:text-4xl">
                MCP &amp; Skill Marketplace
              </h2>
              <p className="mt-4 text-lg text-muted-foreground">
                Browse and install MCP servers and skills from multiple registries. Search exposed as MCP tool for AI-assisted discovery. All installs require explicit user approval.
              </p>
              <ul className="mt-8 space-y-4">
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-cyan-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Multiple registry sources</span>
                    <p className="text-sm text-muted-foreground">Connect official, community, and private MCP/Skill registries</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-cyan-500 mt-0.5" />
                  <div>
                    <span className="font-medium">MCP-exposed search</span>
                    <p className="text-sm text-muted-foreground">Marketplace search available as MCP tool—AI agents can query and suggest installations</p>
                  </div>
                </li>
                <li className="flex gap-3">
                  <Check className="h-5 w-5 shrink-0 text-cyan-500 mt-0.5" />
                  <div>
                    <span className="font-medium">Gated installation</span>
                    <p className="text-sm text-muted-foreground">No package executes without explicit user confirmation via approval dialog</p>
                  </div>
                </li>
              </ul>
            </div>
            {/* Visual: Marketplace Browser */}
            <div className="relative overflow-x-auto order-2 lg:order-1">
              <div className="rounded-xl border-2 border-slate-700 bg-gradient-to-br from-slate-900 to-slate-800 p-6 shadow-2xl min-w-[480px]">
                {/* Header */}
                <div className="flex items-center gap-3 mb-5 pb-8 border-b border-slate-700">
                  <div className="h-10 w-10 rounded-lg bg-gradient-to-br from-cyan-500 to-blue-600 flex items-center justify-center">
                    <Store className="h-5 w-5 text-white" />
                  </div>
                  <div>
                    <div className="text-white font-semibold">Marketplace</div>
                    <div className="text-slate-400 text-xs">3 sources connected</div>
                  </div>
                  <div className="ml-auto">
                    <div className="flex items-center gap-2 px-3 py-1.5 rounded-lg bg-slate-800 border border-slate-600">
                      <Search className="h-4 w-4 text-slate-400" />
                      <span className="text-slate-400 text-xs">Search MCPs &amp; Skills...</span>
                    </div>
                  </div>
                </div>
                {/* Results */}
                <div className="space-y-3">
                  {/* MCP Server */}
                  <div className="rounded-lg bg-white/5 border border-white/10 p-3 hover:border-cyan-500/50 transition-colors cursor-pointer">
                    <div className="flex items-start gap-3">
                      <div className="h-10 w-10 rounded-lg bg-emerald-500/20 border border-emerald-500/30 flex items-center justify-center shrink-0">
                        <img src="/icons/github.svg" alt="GitHub" className="h-5 w-5" />
                      </div>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-white text-sm font-medium">GitHub MCP</span>
                          <span className="px-1.5 py-0.5 rounded text-[10px] bg-emerald-500/20 text-emerald-400">MCP</span>
                        </div>
                        <p className="text-slate-400 text-xs mt-0.5 truncate">Create issues, PRs, search repos, manage workflows</p>
                        <div className="flex items-center gap-3 mt-2">
                          <span className="text-slate-500 text-[10px]">Official Registry</span>
                          <span className="text-slate-500 text-[10px]">12 tools</span>
                        </div>
                      </div>
                      <button className="px-2.5 py-1 rounded bg-cyan-500/20 text-cyan-400 text-xs font-medium hover:bg-cyan-500/30 transition-colors flex items-center gap-1">
                        <Download className="h-3 w-3" />
                        Install
                      </button>
                    </div>
                  </div>
                  {/* Skill */}
                  <div className="rounded-lg bg-white/5 border border-white/10 p-3 hover:border-cyan-500/50 transition-colors cursor-pointer">
                    <div className="flex items-start gap-3">
                      <div className="h-10 w-10 rounded-lg bg-violet-500/20 border border-violet-500/30 flex items-center justify-center shrink-0">
                        <Terminal className="h-5 w-5 text-violet-400" />
                      </div>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-white text-sm font-medium">Code Review</span>
                          <span className="px-1.5 py-0.5 rounded text-[10px] bg-violet-500/20 text-violet-400">Skill</span>
                        </div>
                        <p className="text-slate-400 text-xs mt-0.5 truncate">Multi-step code analysis with security scanning</p>
                        <div className="flex items-center gap-3 mt-2">
                          <span className="text-slate-500 text-[10px]">Community</span>
                          <span className="text-slate-500 text-[10px]">4 steps</span>
                        </div>
                      </div>
                      <button className="px-2.5 py-1 rounded bg-cyan-500/20 text-cyan-400 text-xs font-medium hover:bg-cyan-500/30 transition-colors flex items-center gap-1">
                        <Download className="h-3 w-3" />
                        Install
                      </button>
                    </div>
                  </div>
                  {/* Another MCP */}
                  <div className="rounded-lg bg-white/5 border border-white/10 p-3 hover:border-cyan-500/50 transition-colors cursor-pointer">
                    <div className="flex items-start gap-3">
                      <div className="h-10 w-10 rounded-lg bg-blue-500/20 border border-blue-500/30 flex items-center justify-center shrink-0">
                        <img src="/icons/jira.svg" alt="Jira" className="h-5 w-5" />
                      </div>
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center gap-2">
                          <span className="text-white text-sm font-medium">Jira MCP</span>
                          <span className="px-1.5 py-0.5 rounded text-[10px] bg-emerald-500/20 text-emerald-400">MCP</span>
                        </div>
                        <p className="text-slate-400 text-xs mt-0.5 truncate">Create and manage Jira issues, sprints, boards</p>
                        <div className="flex items-center gap-3 mt-2">
                          <span className="text-slate-500 text-[10px]">Official Registry</span>
                          <span className="text-slate-500 text-[10px]">8 tools</span>
                        </div>
                      </div>
                      <div className="px-2.5 py-1 rounded bg-slate-700 text-slate-400 text-xs font-medium flex items-center gap-1">
                        <Check className="h-3 w-3" />
                        Installed
                      </div>
                    </div>
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
            <h2 className="mt-4 text-2xl font-bold sm:text-3xl">Local-Only by Design</h2>
            <p className="mt-4 text-muted-foreground">
              Runs entirely on your machine. Zero telemetry, zero cloud sync, zero analytics.
              <br/>
              API keys and request payloads never leave your device.
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
            <h2 className="text-2xl font-bold sm:text-3xl">Get Started</h2>
            <p className="mt-4 text-muted-foreground">
              Download, configure providers, point your apps to <code className="text-sm bg-muted px-1 rounded">localhost:3625</code>.
            </p>
            <div className="mt-8">
              <Button asChild size="xl">
                <Link to="/download">
                  Download
                  <span className="ml-3 flex items-center gap-1.5">
                    <img src="/icons/apple.svg" alt="macOS" className="h-4 w-4 fill-current" style={{ filter: 'invert(1)' }} />
                    <img src="/icons/microsoft-windows.svg" alt="Windows" className="h-4 w-4 fill-current" style={{ filter: 'invert(1)' }} />
                    <img src="/icons/penguin.svg" alt="Linux" className="h-4 w-4 fill-current" style={{ filter: 'invert(1)' }} />
                  </span>
                </Link>
              </Button>
            </div>
          </div>
        </div>
      </section>
    </div>
  )
}
