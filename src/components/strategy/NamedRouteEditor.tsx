import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/Card'
import { Button } from '@/components/ui/Button'
import { Input } from '@/components/ui/Input'
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
import { RoutingPolicyEditor } from './RoutingPolicyEditor'
import type {
  AutoModelConfig,
  GetNamedRouteParams,
  MutateNamedRouteParams,
  NamedRouteAction,
  NamedRouteStatus,
} from '@/types/tauri-commands'

export function NamedRouteEditor({
  strategyId,
  clientId,
  autoConfig,
  onUpdate,
  readOnly = false,
}: {
  strategyId: string
  clientId?: string
  autoConfig: AutoModelConfig
  onUpdate: () => void
  readOnly?: boolean
}) {
  const [status, setStatus] = useState<NamedRouteStatus | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [label, setLabel] = useState('')
  const [snapshotText, setSnapshotText] = useState('')
  const [confirm, setConfirm] = useState<NamedRouteAction | null>(null)
  const activeKey = JSON.stringify(autoConfig)
  useEffect(() => {
    let live = true
    setStatus(null)
    setError(null)
    void invoke<NamedRouteStatus>('get_named_route', {
      strategyId,
    } satisfies GetNamedRouteParams)
      .then((value) => {
        if (live) setStatus(value)
      })
      .catch((error) => {
        if (live) setError(String(error))
      })
    return () => {
      live = false
    }
  }, [strategyId, activeKey])
  const savedDraftKey = JSON.stringify(status?.versions.draft ?? null)
  useEffect(() => {
    setSnapshotText(
      status?.versions.draft
        ? JSON.stringify(status.versions.draft, null, 2)
        : '',
    )
  }, [savedDraftKey])
  async function mutate(operation: NamedRouteAction) {
    if (!status) return
    setBusy(true)
    setError(null)
    try {
      const value = await invoke<NamedRouteStatus>('mutate_named_route', {
        strategyId,
        expectedFingerprint: status.fingerprint,
        operation,
      } satisfies MutateNamedRouteParams)
      setStatus(value)
      onUpdate()
    } catch (error) {
      setError(String(error))
      throw error
    } finally {
      setBusy(false)
    }
  }
  const action = (operation: NamedRouteAction) => {
    void mutate(operation).catch(() => {})
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle>Named route versions</CardTitle>
        <CardDescription>
          Keep the existing model alias stable while publishing routing changes.
          Saving drafts makes no inference calls and does not change the active
          route or model permissions.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {status && (
          <>
            <p className="text-sm">
              <strong>{status.active.model_name}</strong> ·{' '}
              {status.published_revision !== null
                ? `revision ${status.published_revision}`
                : status.modified_since_publication
                  ? 'modified outside its published revision'
                  : 'not published'}
            </p>
            <p className="text-xs text-muted-foreground">
              This strategy is reusable across its assigned clients. Normal
              permissions, privacy and spending limits apply to every revision.
              Current auto-router enablement is preserved.
            </p>
            <div className="flex flex-wrap gap-2">
              <Button
                variant="outline"
                disabled={readOnly || busy || !!status.versions.draft}
                onClick={() =>
                  action({ action: 'save_draft', snapshot: status.active })
                }
              >
                Draft from active route
              </Button>
              <Button
                variant="outline"
                disabled={readOnly || busy || !status.versions.draft}
                onClick={() => setConfirm({ action: 'discard_draft' })}
              >
                Discard draft
              </Button>
              <Button
                variant="outline"
                disabled={busy}
                onClick={() => {
                  setError(null)
                  void invoke<NamedRouteStatus>('get_named_route', {
                    strategyId,
                  } satisfies GetNamedRouteParams)
                    .then(setStatus)
                    .catch((error) => setError(String(error)))
                }}
              >
                Reload
              </Button>
            </div>
            {status.versions.draft && (
              <div className="space-y-3 rounded-md border p-3">
                <p className="text-sm font-medium">
                  Draft · {status.versions.draft.model_name}
                </p>
                <RoutingPolicyEditor
                  strategyId={strategyId}
                  clientId={clientId}
                  value={status.versions.draft.routing_policy}
                  previewSnapshot={status.versions.draft}
                  draftOnly
                  readOnly={readOnly || busy}
                  onSave={async (policy) => {
                    await mutate({
                      action: 'save_draft',
                      snapshot: {
                        ...status.versions.draft!,
                        routing_policy: policy,
                      },
                    })
                  }}
                />
                <div className="flex gap-2">
                  <Input
                    aria-label="Revision label"
                    placeholder="Revision label (optional)"
                    maxLength={128}
                    value={label}
                    onChange={(event) => setLabel(event.target.value)}
                    disabled={readOnly || busy}
                  />
                  <Button
                    disabled={readOnly || busy}
                    onClick={() => setConfirm({ action: 'publish', label })}
                  >
                    Publish saved draft
                  </Button>
                </div>
                <details className="space-y-2">
                  <summary className="text-xs cursor-pointer">
                    Edit complete draft snapshot
                  </summary>
                  <p className="text-xs text-muted-foreground">
                    Edit priority/available pools, policy, ordering and session
                    settings together. Keep the current alias. Saving validates
                    locally and leaves the active route unchanged.
                  </p>
                  <textarea
                    aria-label="Draft routing snapshot JSON"
                    className="w-full min-h-48 rounded-md border bg-background p-3 font-mono text-xs"
                    value={snapshotText}
                    onChange={(event) => setSnapshotText(event.target.value)}
                    disabled={readOnly || busy}
                  />
                  <Button
                    variant="outline"
                    disabled={readOnly || busy}
                    onClick={() => {
                      try {
                        const snapshot = JSON.parse(snapshotText)
                        action({ action: 'save_draft', snapshot })
                      } catch (error) {
                        setError(String(error))
                      }
                    }}
                  >
                    Save complete draft
                  </Button>
                </details>
                <p className="text-xs text-muted-foreground">
                  Save policy edits before publishing. Publication uses the
                  saved draft shown below, including its pools, ordering and
                  session settings.
                </p>
                <details>
                  <summary className="text-xs cursor-pointer">
                    Review saved snapshot
                  </summary>
                  <pre className="max-h-72 overflow-auto whitespace-pre-wrap text-xs">
                    {JSON.stringify(status.versions.draft, null, 2)}
                  </pre>
                </details>
              </div>
            )}
            <div className="space-y-2">
              {status.versions.revisions.map((revision) => (
                <div
                  key={revision.revision}
                  className="flex flex-wrap items-center gap-2 rounded-md border p-3"
                >
                  <div className="min-w-0 flex-1 text-xs">
                    <p className="font-medium">
                      Revision {revision.revision}
                      {revision.label ? ` · ${revision.label}` : ''}
                    </p>
                    <p className="text-muted-foreground">
                      {new Date(revision.published_at * 1000).toLocaleString()}{' '}
                      · {revision.snapshot.prioritized_models.length} priority
                      models
                    </p>
                  </div>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={
                      readOnly ||
                      busy ||
                      status.published_revision === revision.revision
                    }
                    onClick={() =>
                      setConfirm({
                        action: 'rollback',
                        revision: revision.revision,
                      })
                    }
                  >
                    Activate
                  </Button>
                  <Button
                    size="sm"
                    variant="outline"
                    disabled={
                      readOnly ||
                      busy ||
                      status.versions.active_revision === revision.revision
                    }
                    onClick={() =>
                      setConfirm({
                        action: 'remove_revision',
                        revision: revision.revision,
                      })
                    }
                  >
                    Remove
                  </Button>
                </div>
              ))}
            </div>
            <p className="text-xs text-muted-foreground">
              Up to 32 revisions and 1 MiB per strategy. Removing an inactive
              revision is explicit. Rollback preserves an existing draft.
              Changes made through the ordinary routing controls appear as
              unpublished modifications.
            </p>
          </>
        )}
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
      </CardContent>
      <AlertDialog
        open={confirm !== null}
        onOpenChange={(open) => {
          if (!open) setConfirm(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {confirm?.action === 'publish'
                ? 'Publish saved draft?'
                : confirm?.action === 'rollback'
                  ? 'Activate this revision?'
                  : confirm?.action === 'discard_draft'
                    ? 'Discard saved draft?'
                    : 'Remove inactive revision?'}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {confirm?.action === 'publish' || confirm?.action === 'rollback'
                ? 'New requests will use the reviewed saved routing snapshot. Current permissions and auto-router enablement are preserved. Existing provider-bound continuations may require a fresh session.'
                : 'This removes only the selected local draft or inactive configuration revision.'}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (confirm) action(confirm)
                setConfirm(null)
              }}
            >
              Continue
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  )
}
