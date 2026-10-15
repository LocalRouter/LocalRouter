import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/Card'
import { Input } from '@/components/ui/Input'
import { Label } from '@/components/ui/label'
import { Button } from '@/components/ui/Button'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import type {
  ClientSecurityStatus,
  GetClientSecurityParams,
  SetClientSecurityParams,
  RevokeClientCredentialsParams,
} from '@/types/tauri-commands'

function localInput(iso: string | null) {
  if (!iso) return ''
  const date = new Date(iso)
  return new Date(date.getTime() - date.getTimezoneOffset() * 60000)
    .toISOString()
    .slice(0, 16)
}
export function ClientSecurityPanel({
  clientId,
  enabled,
  onUpdate,
}: {
  clientId: string
  enabled: boolean
  onUpdate: () => void
}) {
  const [status, setStatus] = useState<ClientSecurityStatus | null>(null)
  const [expiry, setExpiry] = useState('')
  const [reviewDays, setReviewDays] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [revoke, setRevoke] = useState(false)
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    if (!status?.credential_expires_at) return
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [status?.credential_expires_at])
  useEffect(() => {
    let live = true
    setStatus(null)
    setError(null)
    invoke<ClientSecurityStatus>('get_client_security', {
      clientId,
    } satisfies GetClientSecurityParams)
      .then((value) => {
        if (!live) return
        setStatus(value)
        setExpiry(localInput(value.credential_expires_at))
        setReviewDays(value.review_idle_after_days?.toString() ?? '')
      })
      .catch((error) => {
        if (live) setError(String(error))
      })
    return () => {
      live = false
    }
  }, [clientId, enabled])
  async function save() {
    if (busy) return
    setBusy(true)
    setError(null)
    try {
      const days = reviewDays.trim() ? Number(reviewDays) : null
      if (days !== null && (!Number.isInteger(days) || days < 1 || days > 3650))
        throw new Error('Idle review must be between 1 and 3650 days.')
      const date = expiry ? new Date(expiry) : null
      if (date && !Number.isFinite(date.getTime()))
        throw new Error('Choose a valid expiry date.')
      const value = await invoke<ClientSecurityStatus>('set_client_security', {
        clientId,
        credentialExpiresAt:
          expiry === localInput(status?.credential_expires_at ?? null)
            ? (status?.credential_expires_at ?? null)
            : (date?.toISOString() ?? null),
        reviewIdleAfterDays: days,
      } satisfies SetClientSecurityParams)
      setStatus(value)
      onUpdate()
    } catch (error) {
      setError(String(error))
    } finally {
      setBusy(false)
    }
  }
  const state = status
    ? !status.has_credential
      ? 'Revoked'
      : !status.enabled
        ? 'Disabled'
        : status.credential_active &&
            (!status.credential_expires_at ||
              now < Date.parse(status.credential_expires_at))
          ? 'Active'
          : 'Expired'
    : 'Loading'
  return (
    <Card>
      <CardHeader>
        <CardTitle>Credential lifetime and review</CardTitle>
        <CardDescription>
          Set when this client's credentials expire and review when they were
          last used. Idle review alone does not revoke access.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm">
          Status: {state}
          {status?.idle_review_due ? ' · idle review due' : ''}
        </p>
        <p className="text-xs text-muted-foreground">
          Last recorded use:{' '}
          {status?.last_used
            ? new Date(status.last_used).toLocaleString()
            : 'No recorded use'}
          . Review uses recorded activity and never treats missing history as
          proof that a credential is safe.
        </p>
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="space-y-2">
            <Label htmlFor="credential-expiry">Expiry (local time)</Label>
            <Input
              id="credential-expiry"
              type="datetime-local"
              value={expiry}
              onChange={(event) => setExpiry(event.target.value)}
              disabled={busy || !status}
            />
            <p className="text-xs text-muted-foreground">
              Leave empty for no expiry. A past date blocks access immediately.
            </p>
          </div>
          <div className="space-y-2">
            <Label htmlFor="credential-idle-review">
              Review after idle days
            </Label>
            <Input
              id="credential-idle-review"
              type="number"
              min={1}
              max={3650}
              value={reviewDays}
              onChange={(event) => setReviewDays(event.target.value)}
              disabled={busy || !status}
              placeholder="Optional"
            />
          </div>
        </div>
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
        <div className="flex gap-2">
          <Button disabled={busy || !status} onClick={() => void save()}>
            Save lifetime policy
          </Button>
          <Button
            variant="outline"
            disabled={busy || !status?.has_credential}
            onClick={() => setRevoke(true)}
          >
            Revoke credentials
          </Button>
        </div>
        <AlertDialog open={revoke} onOpenChange={setRevoke}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>
                Revoke this client’s credentials?
              </AlertDialogTitle>
              <AlertDialogDescription>
                This disables the client, invalidates its OAuth tokens and
                removes its current secret. Settings and usage remain.
                Re-enabling alone cannot restore the old key; rotate credentials
                and enable the client to reconnect.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>Keep credentials</AlertDialogCancel>
              <AlertDialogAction
                onClick={() => {
                  setRevoke(false)
                  setBusy(true)
                  setError(null)
                  void invoke<ClientSecurityStatus>(
                    'revoke_client_credentials',
                    { clientId } satisfies RevokeClientCredentialsParams,
                  )
                    .then((value) => {
                      setStatus(value)
                      onUpdate()
                    })
                    .catch((error) => setError(String(error)))
                    .finally(() => setBusy(false))
                }}
              >
                Revoke credentials
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </CardContent>
    </Card>
  )
}
