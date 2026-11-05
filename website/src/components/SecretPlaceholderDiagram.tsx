import { ArrowDown, ArrowRight, FileKey, KeyRound, LockKeyhole, Sparkles } from 'lucide-react'
import { Link } from 'react-router-dom'

const placeholder = '<localrouter-redacted id="abc123" usage="Unmasked when returned verbatim"/>'

export function SecretPlaceholderDiagram() {
  return (
    <section className="border-b bg-muted/20 py-16 sm:py-24" aria-labelledby="secret-placeholders-title">
      <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
        <div className="mb-10 max-w-2xl">
          <div className="mb-3 flex items-center gap-2 text-sm font-medium text-orange-500"><LockKeyhole className="h-4 w-4" />Reversible secret redaction</div>
          <h2 id="secret-placeholders-title" className="text-3xl font-bold tracking-tight sm:text-4xl">Keep the secret.<br />Send the placeholder.</h2>
          <p className="mt-5 text-lg text-muted-foreground">LocalRouter holds the original in your OS credential store. The model gets a unique reference. When it returns that reference, LocalRouter puts the secret back.</p>
        </div>
        <figure className="overflow-hidden rounded-2xl border bg-card shadow-sm" aria-label="A secret is stored locally, a placeholder goes to the model and comes back, and the original is restored in a reply or tool call.">
          <div className="grid divide-y lg:grid-cols-[1fr_auto_1fr_auto_1fr] lg:divide-y-0">
            <div className="p-6 sm:p-8">
              <div className="mb-5 flex items-center gap-2 text-sm font-semibold"><KeyRound className="h-4 w-4 text-amber-500" />1. Hold locally</div>
              <p className="mb-3 text-xs font-medium uppercase tracking-wider text-muted-foreground">Original credential</p>
              <code className="block break-all rounded-lg border border-amber-500/20 bg-amber-500/5 p-3 text-xs text-amber-700 dark:text-amber-300">API_KEY=demo-only-secret</code>
              <div className="mt-5 flex items-start gap-2 text-sm text-muted-foreground"><LockKeyhole className="mt-0.5 h-4 w-4 shrink-0" /><span>OS credential store<br /><span className="text-xs">Keychain · Credential Manager · Linux credential backend</span></span></div>
            </div>
            <div className="flex items-center justify-center p-3 text-muted-foreground" aria-hidden="true"><ArrowRight className="hidden h-5 w-5 lg:block" /><ArrowDown className="h-5 w-5 lg:hidden" /></div>
            <div className="bg-orange-500/[0.03] p-6 sm:p-8">
              <div className="mb-5 flex items-center gap-2 text-sm font-semibold"><Sparkles className="h-4 w-4 text-orange-500" />2. Send the reference</div>
              <p className="mb-3 text-xs font-medium uppercase tracking-wider text-muted-foreground">What the model sees</p>
              <code className="block break-all rounded-lg border border-orange-500/25 bg-orange-500/5 p-3 text-xs leading-relaxed text-orange-700 dark:text-orange-300">{placeholder}</code>
              <p className="mt-5 text-sm text-muted-foreground">The model can copy the reference into its reply or a tool call.</p>
            </div>
            <div className="flex items-center justify-center p-3 text-muted-foreground" aria-hidden="true"><ArrowRight className="hidden h-5 w-5 lg:block" /><ArrowDown className="h-5 w-5 lg:hidden" /></div>
            <div className="p-6 sm:p-8">
              <div className="mb-5 flex items-center gap-2 text-sm font-semibold"><FileKey className="h-4 w-4 text-emerald-500" />3. Restore on return</div>
              <p className="mb-3 text-xs font-medium uppercase tracking-wider text-muted-foreground">Reference returned → original restored</p>
              <code className="block break-all text-xs text-orange-700 dark:text-orange-300">{'<localrouter-redacted id="abc123"/>'}</code>
              <ArrowDown className="my-3 h-4 w-4 text-muted-foreground" aria-hidden="true" />
              <div className="rounded-lg border border-emerald-500/20 bg-emerald-500/5 p-3"><p className="mb-2 text-xs text-muted-foreground">write_file → .env</p><code className="break-all text-xs text-emerald-700 dark:text-emerald-300">API_KEY=demo-only-secret</code></div>
              <p className="mt-5 text-sm text-muted-foreground">The file gets the original. Tool results are protected again before the next model call.</p>
            </div>
          </div>
          <figcaption className="border-t px-6 py-4 text-xs leading-relaxed text-muted-foreground">Synthetic example with a shortened ID. Applies to detected secrets on text gateway requests. Tool permissions still apply; the model can request use of a reference. No extra system prompt is added.</figcaption>
        </figure>
        <div className="mt-6 flex flex-wrap gap-x-6 gap-y-3 text-sm font-medium">
          <Link to="/docs/secret-scanning" className="inline-flex items-center gap-1 text-orange-500 hover:underline">Secret scanning <ArrowRight className="h-4 w-4" /></Link>
          <Link to="/research/reversible-secret-redaction" className="inline-flex items-center gap-1 text-orange-500 hover:underline">Read the research <ArrowRight className="h-4 w-4" /></Link>
        </div>
      </div>
    </section>
  )
}
