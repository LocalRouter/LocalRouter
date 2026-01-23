import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Apple, Monitor, Terminal, Check, ArrowRight, ExternalLink } from 'lucide-react'

const platforms = [
  {
    name: 'macOS',
    icon: Apple,
    requirements: ['macOS 11+ (Big Sur)', 'Intel or Apple Silicon', '200MB disk space'],
    downloadUrl: 'https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_macOS.dmg',
    downloadLabel: 'Download .dmg',
  },
  {
    name: 'Windows',
    icon: Monitor,
    requirements: ['Windows 10+ (64-bit)', '200MB disk space'],
    downloadUrl: 'https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_Windows.msi',
    downloadLabel: 'Download .msi',
  },
  {
    name: 'Linux',
    icon: Terminal,
    requirements: ['Modern Linux (glibc 2.31+)', 'DEB, RPM, AppImage'],
    downloadUrl: 'https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_Linux.deb',
    downloadLabel: 'Download .deb',
    altDownloads: [
      { label: '.rpm', url: 'https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_Linux.rpm' },
    ],
  },
]

const steps = [
  {
    number: '1',
    title: 'Download & Install',
    description: 'Download the installer for your platform and run it. LocalRouter installs like any other app.',
  },
  {
    number: '2',
    title: 'Add Providers',
    description: 'Open LocalRouter and add your AI providers. Enter API keys for OpenAI, Anthropic, or configure local Ollama.',
  },
  {
    number: '3',
    title: 'Create an API Key',
    description: 'Create an API key in LocalRouter. Configure routing rules for cost, privacy, or performance.',
  },
  {
    number: '4',
    title: 'Start Using',
    description: 'Point your apps to localhost:3625. Use your LocalRouter API key instead of provider keys.',
  },
]

export default function Download() {
  return (
    <div className="flex flex-col">
      {/* Hero */}
      <section className="border-b bg-gradient-to-b from-muted/50 to-background py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <h1 className="text-4xl font-bold tracking-tight sm:text-5xl">
              Download LocalRouter
            </h1>
            <p className="mt-6 text-lg text-muted-foreground">
              Available for macOS, Windows, and Linux. Free and open source.
            </p>
          </div>
        </div>
      </section>

      {/* Platform Downloads */}
      <section className="py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="grid gap-6 md:grid-cols-3">
            {platforms.map((platform) => (
              <Card key={platform.name} className="relative overflow-hidden">
                <CardHeader className="text-center pb-4">
                  <platform.icon className="mx-auto h-12 w-12 text-muted-foreground" />
                  <CardTitle className="mt-4">{platform.name}</CardTitle>
                </CardHeader>
                <CardContent className="space-y-4">
                  <ul className="space-y-2">
                    {platform.requirements.map((req) => (
                      <li key={req} className="flex items-start gap-2 text-sm text-muted-foreground">
                        <Check className="h-4 w-4 shrink-0 text-primary mt-0.5" />
                        {req}
                      </li>
                    ))}
                  </ul>

                  <div className="pt-4 space-y-2">
                    <Button asChild className="w-full">
                      <a href={platform.downloadUrl}>
                        {platform.downloadLabel}
                      </a>
                    </Button>
                    {platform.altDownloads?.map((alt) => (
                      <Button key={alt.label} asChild variant="outline" className="w-full">
                        <a href={alt.url}>
                          Download {alt.label}
                        </a>
                      </Button>
                    ))}
                  </div>
                </CardContent>
              </Card>
            ))}
          </div>

          <div className="mt-8 text-center">
            <a
              href="https://github.com/LocalRouter/LocalRouter/releases/latest"
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground transition-colors"
            >
              View all releases on GitHub
              <ExternalLink className="h-4 w-4" />
            </a>
          </div>
        </div>
      </section>

      {/* Quick Start */}
      <section className="border-t bg-muted/30 py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <h2 className="text-2xl font-bold sm:text-3xl">Quick Start</h2>
            <p className="mt-4 text-muted-foreground">
              Get up and running in under 5 minutes.
            </p>
          </div>

          <div className="mt-12 grid gap-6 sm:grid-cols-2 lg:grid-cols-4">
            {steps.map((step) => (
              <div key={step.number} className="relative">
                <div className="flex items-center gap-4">
                  <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-primary text-primary-foreground font-bold">
                    {step.number}
                  </div>
                  <h3 className="font-semibold">{step.title}</h3>
                </div>
                <p className="mt-4 text-sm text-muted-foreground pl-14">
                  {step.description}
                </p>
              </div>
            ))}
          </div>
        </div>
      </section>

      {/* Code Example */}
      <section className="border-t py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <h2 className="text-2xl font-bold sm:text-3xl">Test Your Setup</h2>
            <p className="mt-4 text-muted-foreground">
              After installation, test with a simple curl command.
            </p>
          </div>

          <div className="mt-12 mx-auto max-w-2xl">
            <div className="rounded-lg border bg-zinc-950 p-4 text-sm">
              <pre className="overflow-x-auto text-zinc-100 font-mono">
                <code>{`curl http://localhost:3625/v1/chat/completions \\
  -H "Content-Type: application/json" \\
  -H "Authorization: Bearer lr-your-api-key" \\
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'`}</code>
              </pre>
            </div>
          </div>
        </div>
      </section>

      {/* System Requirements */}
      <section className="border-t bg-muted/30 py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <h2 className="text-2xl font-bold sm:text-3xl">System Requirements</h2>
          </div>

          <div className="mt-12 grid gap-6 md:grid-cols-3">
            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <Apple className="h-5 w-5" />
                  macOS
                </CardTitle>
              </CardHeader>
              <CardContent>
                <ul className="space-y-2 text-sm text-muted-foreground">
                  <li>macOS 11 (Big Sur) or later</li>
                  <li>Intel or Apple Silicon</li>
                  <li>4GB RAM minimum</li>
                  <li>200MB disk space</li>
                </ul>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <Monitor className="h-5 w-5" />
                  Windows
                </CardTitle>
              </CardHeader>
              <CardContent>
                <ul className="space-y-2 text-sm text-muted-foreground">
                  <li>Windows 10 or later (64-bit)</li>
                  <li>4GB RAM minimum</li>
                  <li>200MB disk space</li>
                </ul>
              </CardContent>
            </Card>

            <Card>
              <CardHeader>
                <CardTitle className="flex items-center gap-2">
                  <Terminal className="h-5 w-5" />
                  Linux
                </CardTitle>
              </CardHeader>
              <CardContent>
                <ul className="space-y-2 text-sm text-muted-foreground">
                  <li>Ubuntu 20.04+, Fedora 35+, or equivalent</li>
                  <li>glibc 2.31 or later</li>
                  <li>4GB RAM minimum</li>
                  <li>200MB disk space</li>
                </ul>
              </CardContent>
            </Card>
          </div>
        </div>
      </section>

      {/* Help */}
      <section className="border-t py-16 sm:py-24">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
          <div className="mx-auto max-w-3xl text-center">
            <h2 className="text-2xl font-bold sm:text-3xl">Need Help?</h2>
            <p className="mt-4 text-muted-foreground">
              Check out the documentation or open an issue on GitHub.
            </p>
            <div className="mt-8 flex flex-col items-center justify-center gap-4 sm:flex-row">
              <Button asChild variant="outline">
                <a
                  href="https://github.com/LocalRouter/LocalRouter/blob/master/README.md"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  View Documentation
                </a>
              </Button>
              <Button asChild variant="outline">
                <a
                  href="https://github.com/LocalRouter/LocalRouter/issues"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  Report an Issue
                </a>
              </Button>
            </div>
          </div>
        </div>
      </section>

      {/* Back to Home */}
      <section className="border-t bg-muted/30 py-12">
        <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8 text-center">
          <Link to="/" className="inline-flex items-center gap-2 text-sm text-muted-foreground hover:text-foreground transition-colors">
            <ArrowRight className="h-4 w-4 rotate-180" />
            Back to Home
          </Link>
        </div>
      </section>
    </div>
  )
}
