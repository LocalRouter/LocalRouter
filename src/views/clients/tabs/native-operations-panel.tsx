import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/Card'
import { Checkbox } from '@/components/ui/checkbox'
import { Button } from '@/components/ui/Button'
import type {
  NativeOperation,
  GetClientNativeOperationsParams,
  SetClientNativeOperationsParams,
} from '@/types/tauri-commands'

export function NativeOperationsPanel({
  clientId,
  onUpdate,
}: {
  clientId: string
  onUpdate: () => void
}) {
  const [operations, setOperations] = useState<NativeOperation[] | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    let live = true
    void invoke<NativeOperation[]>('get_client_native_operations', {
      clientId,
    } satisfies GetClientNativeOperationsParams)
      .then((value) => {
        if (live) setOperations(value)
      })
      .catch((error) => {
        if (live) setError(String(error))
      })
    return () => {
      live = false
    }
  }, [clientId])
  async function save() {
    if (!operations) return
    setBusy(true)
    setError(null)
    try {
      const result = await invoke<NativeOperation[]>(
        'set_client_native_operations',
        {
          clientId,
          allowedOperations: operations,
        } satisfies SetClientNativeOperationsParams,
      )
      setOperations(result)
      onUpdate()
    } catch (error) {
      setError(String(error))
    } finally {
      setBusy(false)
    }
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle>Native provider operations</CardTitle>
        <CardDescription>
          Allow explicit provider operations over supplied content. Existing
          model permissions still apply; provider fees may apply.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {(['ocr', 'rerank', 'video'] as const).map((operation) => (
          <label key={operation} className="flex items-center gap-2 text-sm">
            <Checkbox
              checked={operations?.includes(operation) ?? false}
              disabled={busy || !operations}
              onCheckedChange={(checked) =>
                setOperations(
                  (current) =>
                    current &&
                    (checked
                      ? [
                          ...current.filter((value) => value !== operation),
                          operation,
                        ]
                      : current.filter((value) => value !== operation)),
                )
              }
            />
            {
              {
                ocr: 'Allow OCR',
                rerank: 'Allow rerank',
                video: 'Allow video generation',
              }[operation]
            }
          </label>
        ))}
        <Button disabled={busy || !operations} onClick={() => void save()}>
          {busy ? 'Saving…' : 'Save operation permissions'}
        </Button>
        {error && (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        )}
      </CardContent>
    </Card>
  )
}
