import { Link } from 'react-router-dom'
import { Button } from '@/components/ui/button'
import { Github, Menu, X } from 'lucide-react'
import { useState } from 'react'
import Logo from './Logo'

export default function Navigation() {
  const [mobileMenuOpen, setMobileMenuOpen] = useState(false)

  return (
    <nav className="sticky top-0 z-50 border-b bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
      <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
        <div className="flex h-16 items-center justify-between">
          <div className="flex items-center gap-8">
            <Link to="/" className="flex items-center gap-2">
              <Logo className="h-8 w-auto text-primary" />
              <span className="text-lg font-semibold">LocalRouter</span>
            </Link>
            <div className="hidden md:flex items-center gap-6">
              <Link to="/" className="text-sm font-medium text-muted-foreground hover:text-foreground transition-colors">
                Home
              </Link>
              <Link to="/download" className="text-sm font-medium text-muted-foreground hover:text-foreground transition-colors">
                Download
              </Link>
              <a
                href="https://github.com/LocalRouter/LocalRouter"
                target="_blank"
                rel="noopener noreferrer"
                className="text-sm font-medium text-muted-foreground hover:text-foreground transition-colors flex items-center gap-1"
              >
                <Github className="h-4 w-4" />
                GitHub
              </a>
            </div>
          </div>
          <div className="flex items-center gap-4">
            <Button asChild className="hidden sm:inline-flex">
              <Link to="/download">Download</Link>
            </Button>
            <button
              type="button"
              className="md:hidden -m-2.5 inline-flex items-center justify-center rounded-md p-2.5 text-muted-foreground"
              onClick={() => setMobileMenuOpen(!mobileMenuOpen)}
            >
              {mobileMenuOpen ? (
                <X className="h-6 w-6" />
              ) : (
                <Menu className="h-6 w-6" />
              )}
            </button>
          </div>
        </div>
      </div>

      {/* Mobile menu */}
      {mobileMenuOpen && (
        <div className="md:hidden border-t bg-background">
          <div className="space-y-1 px-4 py-3">
            <Link
              to="/"
              className="block rounded-md px-3 py-2 text-base font-medium text-muted-foreground hover:bg-accent hover:text-foreground"
              onClick={() => setMobileMenuOpen(false)}
            >
              Home
            </Link>
            <Link
              to="/download"
              className="block rounded-md px-3 py-2 text-base font-medium text-muted-foreground hover:bg-accent hover:text-foreground"
              onClick={() => setMobileMenuOpen(false)}
            >
              Download
            </Link>
            <a
              href="https://github.com/LocalRouter/LocalRouter"
              target="_blank"
              rel="noopener noreferrer"
              className="flex items-center gap-2 rounded-md px-3 py-2 text-base font-medium text-muted-foreground hover:bg-accent hover:text-foreground"
            >
              <Github className="h-4 w-4" />
              GitHub
            </a>
          </div>
        </div>
      )}
    </nav>
  )
}
