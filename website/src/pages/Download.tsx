import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Check, ArrowRight, ExternalLink } from 'lucide-react'

const platforms = [
  {
    name: 'macOS',
    iconSrc: '/icons/apple.svg',
    requirements: ['macOS 10.15+ (Catalina)', 'Intel or Apple Silicon', '200MB disk space'],
    downloadUrl: 'https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_aarch64.dmg',
    downloadLabel: 'Download for Apple Silicon',
    altDownloads: [
      { label: 'Intel Mac', url: 'https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_x64.dmg' },
    ],
  },
  {
    name: 'Windows',
    iconSrc: '/icons/microsoft-windows.svg',
    requirements: ['Windows 10+ (64-bit)', '200MB disk space'],
    downloadUrl: 'https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_x64-setup.exe',
    downloadLabel: 'Download .exe',
    altDownloads: [
      { label: '.msi', url: 'https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_x64.msi' },
    ],
  },
  {
    name: 'Linux',
    iconSrc: '/icons/penguin.svg',
    requirements: ['Modern Linux (glibc 2.31+)', 'DEB or AppImage'],
    downloadUrl: 'https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_amd64.deb',
    downloadLabel: 'Download .deb',
    altDownloads: [
      { label: '.AppImage', url: 'https://github.com/LocalRouter/LocalRouter/releases/latest/download/LocalRouter_amd64.AppImage' },
    ],
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
                  <img src={platform.iconSrc} alt={platform.name} className="mx-auto h-12 w-12 opacity-70 dark:invert" />
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
