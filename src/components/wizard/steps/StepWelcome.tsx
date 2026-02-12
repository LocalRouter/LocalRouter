/**
 * Step 0: Welcome (First Launch Only)
 *
 * Introductory step shown only on first app launch to guide users.
 */

export function StepWelcome() {
  return (
    <div className="flex flex-col items-center justify-center py-6 space-y-6">
      {/* LocalRouter logo */}
      <div className="relative">
        <div className="absolute inset-0 blur-2xl opacity-20 bg-primary rounded-full scale-150" />
        <svg
          width="80"
          height="80"
          viewBox="0 0 100 100"
          className="relative text-primary"
          aria-label="LocalRouter logo"
        >
          <circle cx="20" cy="20" r="12" fill="none" stroke="currentColor" strokeWidth="8" />
          <circle cx="80" cy="80" r="12" fill="none" stroke="currentColor" strokeWidth="8" />
          <path
            d="M 32 22 C 75 15, 90 40, 50 50 C 10 60, 25 85, 68 78"
            stroke="currentColor"
            strokeWidth="8"
            strokeLinecap="round"
            fill="none"
          />
        </svg>
      </div>

      {/* Title and tagline */}
      <div className="text-center space-y-2">
        <h3 className="text-xl font-semibold tracking-tight">Welcome to LocalRouter</h3>
        <p className="text-sm text-muted-foreground max-w-sm mx-auto leading-relaxed">
          Your local gateway to AI providers. One API endpoint, many models, fully private.
        </p>
      </div>

      {/* Minimal feature highlights */}
      <div className="flex gap-6 text-center text-xs text-muted-foreground pt-2">
        <div className="space-y-1">
          <p className="font-medium text-foreground">Private</p>
          <p>Runs locally</p>
        </div>
        <div className="w-px bg-border" />
        <div className="space-y-1">
          <p className="font-medium text-foreground">Smart</p>
          <p>Model routing</p>
        </div>
        <div className="w-px bg-border" />
        <div className="space-y-1">
          <p className="font-medium text-foreground">Extensible</p>
          <p>MCP & Skills</p>
        </div>
      </div>

      {/* CTA */}
      <p className="text-sm text-muted-foreground text-center pt-2">
        Let&apos;s create your first client to get started.
      </p>
    </div>
  )
}
