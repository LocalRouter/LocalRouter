import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { Card, CardContent } from '@/components/ui/card'
import { Badge } from '@/components/ui/badge'
import ArchitectureGraph from '@/components/ArchitectureGraph'
import {
  ArrowRight,
  Shield,
  Zap,
  DollarSign,
  Eye,
  Lock,
  Server,
  Layers,
  GitBranch,
  Check,
} from 'lucide-react'

const providers = [
  { name: 'Ollama', type: 'local' },
  { name: 'LM Studio', type: 'local' },
  { name: 'OpenAI', type: 'cloud' },
  { name: 'Anthropic', type: 'cloud' },
  { name: 'Google Gemini', type: 'cloud' },
  { name: 'Mistral', type: 'cloud' },
  { name: 'OpenRouter', type: 'aggregator' },
  { name: 'Together AI', type: 'aggregator' },
  { name: 'Groq', type: 'cloud' },
  { name: 'Perplexity', type: 'cloud' },
]

const features = [
  {
    icon: Zap,
    title: 'OpenAI-Compatible API',
    description: 'Drop-in replacement. Change one URL, access all providers. Works with any tool expecting OpenAI API.',
  },
  {
    icon: DollarSign,
    title: 'Cost-Aware Routing',
    description: 'Route requests to cheapest viable model. Set cost limits per API key. Track spending in real-time.',
  },
  {
    icon: Lock,
    title: 'Keep Data Local',
    description: 'Sensitive work stays on Ollama. Simple tasks go to cloud. You control the routing policy.',
  },
  {
    icon: Eye,
    title: 'Unified Monitoring',
    description: 'One dashboard for all providers. Track requests, tokens, latency, and costs across everything.',
  },
  {
    icon: Shield,
    title: 'Per-Client Access Control',
    description: 'Create API keys with different routing rules. Limit which providers each app can access.',
  },
  {
    icon: Server,
    title: 'MCP Gateway',
    description: 'Unified access to MCP servers. STDIO, SSE, WebSocket transports. OAuth and custom auth support.',
  },
]

const useCases = [
  {
    title: 'For Developers',
    items: [
      'Write code once, swap providers without changes',
      'Experiment with new models instantly',
      'Local development with Ollama, production with cloud',
    ],
  },
  {
    title: 'For Teams',
    items: [
      'Centralized AI spending visibility',
      'Per-project API keys with cost limits',
      'Audit trail for every request',
    ],
  },
  {
    title: 'For Privacy',
    items: [
      'Sensitive code never leaves your machine',
      'Route by data sensitivity automatically',
      'No telemetry, no tracking, fully local',
    ],
  },
]

export default function Home() {
  return (
    <div className="flex flex-col">
      {/* Hero */}
      <section className="relative overflow-hidden border-b bg-gradient-to-b from-muted/50 to-background">
        <div className="mx-auto max-w-7xl px-4 py-24 sm:px-6 sm:py-32 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <Badge variant="secondary" className="mb-4">
              Open Source Desktop App
            </Badge>
            <h1 className="text-4xl font-bold tracking-tight sm:text-5xl lg:text-6xl">
              One Local API.
              <br />
              <span className="text-primary">All AI Providers.</span>
            </h1>
            <p className="mt-6 text-lg text-muted-foreground sm:text-xl">
              LocalRouter is a desktop gateway that unifies OpenAI, Anthropic, Ollama, and 10+ other providers behind a single OpenAI-compatible API. Route by cost, keep data local, monitor everything.
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

          {/* Architecture Visual */}
          <div className="mx-auto mt-16 max-w-4xl">
            <ArchitectureGraph />
          </div>
        </div>
      </section>

      {/* Problem Statement */}
      <section className="border-b py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <h2 className="text-2xl font-bold sm:text-3xl">The Problem</h2>
            <p className="mt-4 text-muted-foreground">
              AI APIs are fragmented. You&apos;re locked into one provider, costs are invisible, and every tool needs separate configuration.
            </p>
          </div>

          <div className="mt-12 grid gap-6 md:grid-cols-3">
            <Card>
              <CardContent className="pt-6">
                <Layers className="h-8 w-8 text-muted-foreground" />
                <h3 className="mt-4 font-semibold">Provider Lock-in</h3>
                <p className="mt-2 text-sm text-muted-foreground">
                  Your apps are tied to one provider. Switching means rewriting integrations and changing configs everywhere.
                </p>
              </CardContent>
            </Card>
            <Card>
              <CardContent className="pt-6">
                <DollarSign className="h-8 w-8 text-muted-foreground" />
                <h3 className="mt-4 font-semibold">Hidden Costs</h3>
                <p className="mt-2 text-sm text-muted-foreground">
                  No unified view of spending. Different dashboards for each provider. Surprise bills at month end.
                </p>
              </CardContent>
            </Card>
            <Card>
              <CardContent className="pt-6">
                <GitBranch className="h-8 w-8 text-muted-foreground" />
                <h3 className="mt-4 font-semibold">No Routing Control</h3>
                <p className="mt-2 text-sm text-muted-foreground">
                  Can&apos;t route by cost or privacy. Sensitive work goes to cloud. Simple tasks use expensive models.
                </p>
              </CardContent>
            </Card>
          </div>
        </div>
      </section>

      {/* Features */}
      <section className="border-b py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <h2 className="text-2xl font-bold sm:text-3xl">How LocalRouter Solves This</h2>
            <p className="mt-4 text-muted-foreground">
              A single local gateway that gives you control over routing, costs, and privacy.
            </p>
          </div>

          <div className="mt-12 grid gap-6 sm:grid-cols-2 lg:grid-cols-3">
            {features.map((feature) => (
              <Card key={feature.title}>
                <CardContent className="pt-6">
                  <feature.icon className="h-8 w-8 text-primary" />
                  <h3 className="mt-4 font-semibold">{feature.title}</h3>
                  <p className="mt-2 text-sm text-muted-foreground">{feature.description}</p>
                </CardContent>
              </Card>
            ))}
          </div>
        </div>
      </section>

      {/* Use Cases */}
      <section className="border-b py-16 sm:py-24 bg-muted/30">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <h2 className="text-2xl font-bold sm:text-3xl">Who It&apos;s For</h2>
          </div>

          <div className="mt-12 grid gap-8 md:grid-cols-3">
            {useCases.map((useCase) => (
              <div key={useCase.title} className="rounded-lg border bg-card p-6">
                <h3 className="font-semibold text-lg">{useCase.title}</h3>
                <ul className="mt-4 space-y-3">
                  {useCase.items.map((item) => (
                    <li key={item} className="flex gap-3 text-sm text-muted-foreground">
                      <Check className="h-5 w-5 shrink-0 text-primary" />
                      {item}
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </div>
      </section>

      {/* Providers */}
      <section className="border-b py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <h2 className="text-2xl font-bold sm:text-3xl">Supported Providers</h2>
            <p className="mt-4 text-muted-foreground">
              Mix local models with cloud providers. Use them all through one API.
            </p>
          </div>

          <div className="mt-12 flex flex-wrap justify-center gap-3">
            {providers.map((provider) => (
              <Badge
                key={provider.name}
                variant={provider.type === 'local' ? 'default' : 'secondary'}
                className="text-sm py-1.5 px-3"
              >
                {provider.name}
                {provider.type === 'local' && (
                  <Lock className="ml-1.5 h-3 w-3" />
                )}
              </Badge>
            ))}
          </div>
        </div>
      </section>

      {/* Code Example */}
      <section className="border-b py-16 sm:py-24 bg-muted/30">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <h2 className="text-2xl font-bold sm:text-3xl">Works With Your Existing Code</h2>
            <p className="mt-4 text-muted-foreground">
              Just change the base URL. Your OpenAI SDK code works unchanged.
            </p>
          </div>

          <div className="mt-12 mx-auto max-w-2xl">
            <div className="rounded-lg border bg-zinc-950 p-4 text-sm">
              <pre className="overflow-x-auto text-zinc-100 font-mono">
                <code>{`from openai import OpenAI

client = OpenAI(
    base_url="http://localhost:3625/v1",
    api_key="lr-your-key"
)

# Routes to cheapest model matching your rules
response = client.chat.completions.create(
    model="gpt-4",  # or "claude-3", "llama3", etc.
    messages=[{"role": "user", "content": "Hello!"}]
)`}</code>
              </pre>
            </div>
          </div>
        </div>
      </section>

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
