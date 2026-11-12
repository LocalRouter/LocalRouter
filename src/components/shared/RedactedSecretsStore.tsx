import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Copy, Eye, EyeOff, ShieldCheck, Loader2, RefreshCw, Trash2 } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/Button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/Card'
import { Badge } from '@/components/ui/Badge'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/Select'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { listenSafe } from '@/hooks/useTauriListener'
import type { ListRedactedSecretsParams, MarkRedactedSecretNotSecretParams, PurgeRedactedSecretsParams, RedactedSecretInventory, RedactedSecretEntry, RevealRedactedSecretParams } from '@/types/tauri-commands'

const entryKey = (entry: RedactedSecretEntry) => `${entry.client_id}:${entry.reference_id}`
const date = (seconds: number) => new Date(seconds * 1000).toLocaleString()
interface PurgeSelection { clientId: string | null; referenceId: string | null; label: string }

export function RedactedSecretsStore({ clientId }: { clientId?: string }) {
  const [entries, setEntries] = useState<RedactedSecretEntry[]>([])
  const [persistenceWarning, setPersistenceWarning] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [filter, setFilter] = useState('all')
  const [page, setPage] = useState(0)
  const [revealed, setRevealed] = useState<Record<string, string>>({})
  const [revealing, setRevealing] = useState<string | null>(null)
  const [purge, setPurge] = useState<PurgeSelection | null>(null)
  const [purging, setPurging] = useState(false)
  const [notSecret, setNotSecret] = useState<RedactedSecretEntry | null>(null)
  const [remembering, setRemembering] = useState(false)
  const busy = purging || remembering
  const generation = useRef(0)
  const load = useCallback(async () => {
    const current = ++generation.current
    setLoading(true); setError(null); setRevealed({}); setRevealing(null)
    try {
      const values = await invoke<RedactedSecretInventory>('list_redacted_secrets', { clientId: clientId ?? null } satisfies ListRedactedSecretsParams)
      if (current === generation.current) { setEntries(values.entries); setPersistenceWarning(values.persistence_warning); setPage(0) }
    } catch (error) { if (current === generation.current) setError(String(error)) }
    finally { if (current === generation.current) setLoading(false) }
  }, [clientId])
  useEffect(() => {
    setFilter('all'); void load()
    const listener = listenSafe('secret-store-changed', () => { void load() })
    return () => { generation.current++; listener.cleanup() }
  }, [load])
  const selectedClient = clientId ?? (filter === 'all' ? null : filter)
  const filtered = entries.filter(entry => !selectedClient || entry.client_id === selectedClient)
  const clients = [...new Map(entries.map(entry => [entry.client_id, entry.client_name])).entries()]
  const pages = Math.max(1, Math.ceil(filtered.length / 25))
  const currentPage = Math.min(page, pages - 1)
  const visible = filtered.slice(currentPage * 25, (currentPage + 1) * 25)
  const reveal = async (entry: RedactedSecretEntry) => {
    const key = entryKey(entry)
    if (revealed[key] !== undefined) { setRevealed(values => { const next = { ...values }; delete next[key]; return next }); return }
    const current = generation.current
    setRevealing(key); setError(null)
    try {
      const value = await invoke<string>('reveal_redacted_secret', { clientId: entry.client_id, referenceId: entry.reference_id } satisfies RevealRedactedSecretParams)
      if (current === generation.current) setRevealed(values => ({ ...values, [key]: value }))
    } catch (error) { if (current === generation.current) setError(String(error)) }
    finally { if (current === generation.current) setRevealing(null) }
  }
  const confirmPurge = async () => {
    if (!purge) return
    setPurging(true); generation.current++; setRevealed({}); setRevealing(null)
    let failure: string | null = null
    try {
      await invoke<number>('purge_redacted_secrets', { clientId: purge.clientId, referenceId: purge.referenceId } satisfies PurgeRedactedSecretsParams)
      toast.success('Stored secrets purged')
    } catch (error) { failure = String(error) }
    finally { setPurging(false); setPurge(null); await load(); if (failure) setError(failure) }
  }
  const confirmNotSecret = async () => {
    if (!notSecret) return
    setRemembering(true); generation.current++; setRevealed({}); setRevealing(null)
    let failure: string | null = null
    try {
      await invoke<void>('mark_redacted_secret_not_secret', { clientId: notSecret.client_id, referenceId: notSecret.reference_id } satisfies MarkRedactedSecretNotSecretParams)
      toast.success('Marked as not a secret for this client')
    } catch (error) { failure = String(error) }
    finally { setRemembering(false); setNotSecret(null); await load(); if (failure) setError(failure) }
  }
  return <>
    <Card>
      <CardHeader>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <CardTitle>Stored secrets</CardTitle>
          <div className="flex gap-2">
            <Button variant="outline" size="sm" disabled={loading || busy} onClick={() => { void load() }}><RefreshCw className="mr-2 h-4 w-4" />Refresh</Button>
            <Button variant="outline" size="sm" className="text-destructive" disabled={loading || busy || (!filtered.length && !clientId)} onClick={() => setPurge({ clientId: selectedClient, referenceId: null, label: selectedClient ? `all redacted secrets for ${entries.find(entry => entry.client_id === selectedClient)?.client_name ?? 'this client'}` : 'all redacted secrets for every client' })}><Trash2 className="mr-2 h-4 w-4" />{selectedClient ? 'Purge client' : 'Purge all'}</Button>
          </div>
        </div>
        <CardDescription>Originals are kept locally for 14 days of client inactivity. Browsing this list does not extend retention. Mark a false positive as Not a secret to remember an exception for that client. Existing references stay restorable until their normal expiry.</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {!clientId && clients.length > 0 && <Select disabled={loading || busy} value={filter} onValueChange={value => { setFilter(value); setPage(0); setRevealed({}); setRevealing(null); generation.current++ }}><SelectTrigger className="w-full sm:w-72" aria-label="Filter stored secrets by client"><SelectValue placeholder="All clients" /></SelectTrigger><SelectContent><SelectItem value="all">All clients</SelectItem>{clients.map(([id, name]) => <SelectItem key={id} value={id}>{name}</SelectItem>)}</SelectContent></Select>}
        {persistenceWarning && <p className="rounded-md border border-amber-500/40 p-3 text-sm" role="status">{persistenceWarning}</p>}
        {error && <div className="rounded-md border border-destructive/30 p-3 text-sm text-destructive" role="status">{error}</div>}
        {loading ? <div className="flex items-center gap-2 text-sm text-muted-foreground"><Loader2 className="h-4 w-4 animate-spin" />Loading stored secrets…</div> : !filtered.length ? <p className="text-sm text-muted-foreground">{persistenceWarning ? 'No in-memory or readable cached references are available' : 'No redacted secrets are stored'}{selectedClient ? ' for this client' : ''}.</p> : <>
          {visible.map(entry => {
            const key = entryKey(entry); const value = revealed[key]
            return <div key={key} className="space-y-3 rounded-lg border p-3">
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="min-w-0 space-y-1">
                  {!clientId && <p className="text-sm font-medium">{entry.client_name}</p>}
                  <code className="block break-all text-xs text-muted-foreground">{entry.reference_id}</code>
                </div>
                <div className="flex flex-wrap items-center justify-end gap-1">
                  {entry.ignored && <Badge variant="outline">Not a secret</Badge>}
                  {entry.revoked && <Badge variant="destructive">Purge pending</Badge>}
                  {entry.expired && <Badge variant="outline">Expired</Badge>}
                  <Button variant="ghost" size="sm" disabled={entry.expired || entry.revoked || busy || revealing !== null} onClick={() => { void reveal(entry) }} aria-label={value === undefined ? 'Reveal stored secret' : 'Hide stored secret'}>{revealing === key ? <Loader2 className="h-4 w-4 animate-spin" /> : value === undefined ? <Eye className="h-4 w-4" /> : <EyeOff className="h-4 w-4" />}<span className="ml-2">{value === undefined ? 'Reveal' : 'Hide'}</span></Button>
                  <Button variant="ghost" size="sm" disabled={busy} onClick={() => { void navigator.clipboard.writeText(entry.placeholder).then(() => toast.success('Reference copied')).catch(error => setError(String(error))) }} aria-label="Copy secret reference"><Copy className="h-4 w-4" /></Button>
                  <Button variant="outline" size="sm" disabled={entry.ignored || entry.expired || entry.revoked || busy} onClick={() => setNotSecret(entry)} aria-label="Mark stored value as not a secret"><ShieldCheck className="mr-2 h-4 w-4" />Not a secret</Button>
                  <Button variant="ghost" size="sm" className="text-destructive" disabled={busy} onClick={() => setPurge({ clientId: entry.client_id, referenceId: entry.reference_id, label: `the redacted secret for ${entry.client_name} and all its references (${entry.reference_id})` })} aria-label="Purge stored secret"><Trash2 className="h-4 w-4" /></Button>
                </div>
              </div>
              <code className="block break-all whitespace-pre-wrap rounded bg-muted p-2 text-sm">{value ?? entry.masked_value}</code>
              <p className="text-xs text-muted-foreground">Client last active {date(entry.last_used_at)} · Expires {date(entry.expires_at)}</p>
            </div>
          })}
          <div className="flex flex-wrap items-center justify-between gap-3 text-xs text-muted-foreground">
            <span>{currentPage * 25 + 1}–{Math.min((currentPage + 1) * 25, filtered.length)} of {filtered.length} references</span>
            <div className="flex gap-2"><Button variant="outline" size="sm" disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>Previous</Button><Button variant="outline" size="sm" disabled={currentPage + 1 >= pages} onClick={() => setPage(currentPage + 1)}>Next</Button></div>
          </div>
        </>}
      </CardContent>
    </Card>
    <AlertDialog open={notSecret !== null} onOpenChange={open => { if (!open && !remembering) setNotSecret(null) }}>
      <AlertDialogContent>
        <AlertDialogHeader><AlertDialogTitle>Mark as not a secret?</AlertDialogTitle><AlertDialogDescription>Never redact this exact value for {notSecret?.client_name}. This decision does not expire. Existing references remain available to restore in-flight replies. Other clients keep their own decisions. You can remove this exception in the client’s Secret Scan → Remembered rules tab.</AlertDialogDescription></AlertDialogHeader>
        <AlertDialogFooter><AlertDialogCancel disabled={remembering}>Cancel</AlertDialogCancel><AlertDialogAction disabled={remembering} onClick={event => { event.preventDefault(); void confirmNotSecret() }}>{remembering && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}Not a secret</AlertDialogAction></AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
    <AlertDialog open={purge !== null} onOpenChange={open => { if (!open && !purging) setPurge(null) }}>
      <AlertDialogContent>
        <AlertDialogHeader><AlertDialogTitle>Purge stored secrets?</AlertDialogTitle><AlertDialogDescription>Remove {purge?.label} from local credential storage. Existing references to removed originals will remain literal, with a diagnostic, rather than revealing a value. Purging one secret preserves unrelated in-flight references. New requests can store fresh references.</AlertDialogDescription></AlertDialogHeader>
        <AlertDialogFooter><AlertDialogCancel disabled={busy}>Cancel</AlertDialogCancel><AlertDialogAction className="bg-destructive text-destructive-foreground hover:bg-destructive/90" disabled={busy} onClick={event => { event.preventDefault(); void confirmPurge() }}>{purging && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}Purge</AlertDialogAction></AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </>
}
