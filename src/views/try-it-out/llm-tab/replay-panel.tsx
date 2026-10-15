import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { sumReplayCosts } from '@/utils/replay'
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
  ReplayEstimate,
  ReplayPreviewParams,
  ReplayArtifact,
  ReplayArtifactList,
  ReplayArtifactSummary,
  ReplayCase,
  ReplayDataset,
  ReplayInput,
  ReplayRun,
  ReplayRunSummary,
  ReplayResult,
  ReplayTarget,
  ReplayStartParams,
  ReplayIdParams,
  ReplayResultParams,
  ReplaySaveDatasetParams,
  ReplayReadParams,
  ReplayDeleteParams,
  ReplayImportParams,
  ReplayImportRunParams,
  ReplayJudgeParams,
  ReplayCurrentPolicyParams,
  RoutingPolicy,
} from '@/types/tauri-commands'
export const initialReplayCases = (input?: ReplayInput): ReplayCase[] => [
  {
    id: crypto.randomUUID(),
    name: 'Case 1',
    input: input ?? {
      endpoint: 'chat_completions',
      body: {
        messages: [{ role: 'user', content: 'Explain this in one sentence.' }],
      },
      source_event_id: null,
    },
    checks: [],
  },
]
const pretty = (v: unknown) => JSON.stringify(v, null, 2)
function download(value: unknown, name: string) {
  const url = URL.createObjectURL(
    new Blob([pretty(value)], { type: 'application/json' }),
  )
  const a = document.createElement('a')
  a.href = url
  a.download = name
  a.click()
  URL.revokeObjectURL(url)
}
export function ReplayPanel({
  clientId,
  models,
  initialInput,
}: {
  clientId: string
  models: {
    id: string
  }[]
  initialInput?: ReplayInput
}) {
  const [casesText, setCasesText] = useState(() =>
    pretty(initialReplayCases(initialInput)),
  )
  const [targetsText, setTargetsText] = useState('[]')
  const [dataset, setDataset] = useState<ReplayDataset>({
    id: crypto.randomUUID(),
    name: 'New dataset',
    revision: 0,
    cases: [],
    saved_at: null,
  })
  const [estimates, setEstimates] = useState<ReplayEstimate[] | null>(null)
  const [maxTokens, setMaxTokens] = useState(512)
  const [run, setRun] = useState<ReplayRunSummary | null>(null)
  const [runs, setRuns] = useState<ReplayRunSummary[]>([])
  const [details, setDetails] = useState<Record<number, ReplayResult>>({})
  const [saved, setSaved] = useState<ReplayArtifactList>({
    items: [],
    issues: [],
  })
  const [toDelete, setToDelete] = useState<ReplayArtifactSummary | null>(null)
  const [judgeModel, setJudgeModel] = useState('')
  const [rubric, setRubric] = useState(
    'Assess accuracy and completeness for the supplied input.',
  )
  const [judgeIndices, setJudgeIndices] = useState<number[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const action = async (fn: () => Promise<void>) => {
    setBusy(true)
    setError('')
    try {
      await fn()
    } catch (e) {
      setError(String(e))
    } finally {
      setBusy(false)
    }
  }
  const refresh = async () => {
    setSaved(await invoke<ReplayArtifactList>('replay_list'))
    setRuns(await invoke<ReplayRunSummary[]>('replay_runs'))
  }
  useEffect(() => {
    void action(refresh)
  }, [])
  useEffect(() => {
    if (run?.status !== 'running') return
    let disposed = false
    const timer = setInterval(() => {
      invoke<ReplayRunSummary>('replay_status', {
        id: run.id,
      } satisfies ReplayIdParams)
        .then((v) => {
          if (!disposed) setRun(v)
        })
        .catch((e) => {
          if (!disposed) setError(String(e))
        })
    }, 750)
    return () => {
      disposed = true
      clearInterval(timer)
    }
  }, [run?.id, run?.status])
  const openRun = (v: ReplayRunSummary) => {
    setRun(v)
    setDetails({})
    setJudgeIndices([])
  }
  const cases = () => JSON.parse(casesText) as ReplayCase[]
  const start = async () => {
    const targets = JSON.parse(targetsText) as ReplayTarget[]
    const id = await invoke<string>('replay_start', {
      request: {
        client_id: clientId,
        dataset_id: dataset.revision ? dataset.id : null,
        cases: cases(),
        targets,
        max_output_tokens: maxTokens,
      },
    } satisfies ReplayStartParams)
    openRun(
      await invoke<ReplayRunSummary>('replay_status', {
        id,
      } satisfies ReplayIdParams),
    )
    await refresh()
  }
  const read = async (item: ReplayArtifactSummary) => {
    const artifact = await invoke<ReplayArtifact>('replay_read', {
      kind: item.kind,
      id: item.id,
    } satisfies ReplayReadParams)
    if (item.kind === 'dataset') {
      const value = artifact.value as ReplayDataset
      setDataset(value)
      setCasesText(pretty(value.cases))
    } else {
      const value = artifact.value as ReplayRun
      openRun({
        ...value,
        results: value.results.map((r, index) => ({ ...r, index })),
      })
      setDetails(Object.fromEntries(value.results.map((r, i) => [i, r])))
    }
  }
  let countLabel = 'Invalid cases or targets JSON'
  try {
    countLabel = `${cases().length} cases × ${(JSON.parse(targetsText) as ReplayTarget[]).length} targets`
  } catch {
    /* editing */
  }
  const costs = sumReplayCosts(run?.results.map((r) => r.cost) ?? [])
  const unknown =
    run?.results.reduce(
      (sum, r) =>
        sum + r.cost.unknown_calls + Number(r.cost.classifier_cost_unknown),
      0,
    ) ?? 0
  return (
    <div className="space-y-4 text-sm">
      <p>
        Compare models and classifier policies using this client’s current
        permissions and limits. Tools are disabled. Opening and importing never
        sends requests. Inputs and results remain in memory until you choose
        Save.
      </p>
      {error && (
        <p role="alert" className="text-destructive">
          {error}
        </p>
      )}
      <label className="block">
        Dataset name
        <input
          className="block w-full border rounded p-2 bg-background"
          value={dataset.name}
          onChange={(e) => setDataset((d) => ({ ...d, name: e.target.value }))}
        />
      </label>
      <label className="block">
        Editable cases (JSON)
        <textarea
          aria-label="Replay cases"
          className="w-full min-h-52 border rounded p-2 bg-background font-mono text-xs"
          value={casesText}
          onChange={(e) => setCasesText(e.target.value)}
        />
      </label>
      <p className="text-xs text-muted-foreground">
        Each case has id, name, input (endpoint, body, source_event_id) and
        checks. Checks: contains, regex, json_valid, json_pointer or
        expected_route. Endpoints: chat_completions, completions, responses,
        decisions, system_one.
      </p>
      <div className="flex flex-wrap gap-2">
        <select
          aria-label="Add comparison model"
          className="border rounded bg-background p-2"
          value=""
          onChange={(e) => {
            try {
              const targets = JSON.parse(targetsText) as ReplayTarget[]
              setTargetsText(
                pretty([...targets, { kind: 'model', model: e.target.value }]),
              )
            } catch (e) {
              setError(String(e))
            }
          }}
        >
          <option value="">Add an allowed model…</option>
          {models.map((m) => (
            <option key={m.id} value={m.id}>
              {m.id}
            </option>
          ))}
        </select>
      </div>
      <Button
        variant="outline"
        disabled={!clientId || busy}
        onClick={() =>
          void action(async () => {
            const policy = await invoke<RoutingPolicy | null>(
              'replay_current_policy',
              { clientId } satisfies ReplayCurrentPolicyParams,
            )
            if (!policy)
              throw Error('This client has no routing policy configured')
            const targets = JSON.parse(targetsText) as ReplayTarget[]
            setTargetsText(
              pretty([
                ...targets,
                { kind: 'policy', name: 'Current policy snapshot', policy },
              ]),
            )
          })
        }
      >
        Add current policy (edit JSON for a draft)
      </Button>
      <label className="block">
        Comparison targets (JSON)
        <textarea
          aria-label="Replay targets"
          className="w-full min-h-24 border rounded p-2 bg-background font-mono text-xs"
          value={targetsText}
          onChange={(e) => setTargetsText(e.target.value)}
        />
      </label>
      <p className="text-xs text-muted-foreground">
        Model target: {'{"kind":"model","model":"provider/model"}'}. Classifier
        target: {'{"kind":"policy","name":"draft","policy":{…}}'} using a
        RoutingPolicy JSON snapshot. Policy targets classify each case without
        generating an answer. Current client restrictions apply to every call.
      </p>
      <div className="flex gap-2 flex-wrap items-center">
        <label>
          Output limit{' '}
          <input
            aria-label="Replay output token limit"
            type="number"
            min={1}
            max={32768}
            value={maxTokens}
            onChange={(e) => setMaxTokens(Number(e.target.value))}
            className="w-24 border bg-background p-1"
          />
        </label>
        <span>
          {countLabel}. Spend is unknown before execution; normal provider
          charges apply.
        </span>
        <Button
          variant="outline"
          disabled={busy || !clientId}
          onClick={() =>
            void action(async () => {
              setEstimates(
                await invoke<ReplayEstimate[]>('replay_preview', {
                  request: {
                    client_id: clientId,
                    dataset_id: null,
                    cases: cases(),
                    targets: JSON.parse(targetsText) as ReplayTarget[],
                    max_output_tokens: maxTokens,
                  },
                } satisfies ReplayPreviewParams),
              )
            })
          }
        >
          Preview costs (no inference)
        </Button>
        <Button disabled={busy || !clientId} onClick={() => void action(start)}>
          Run comparison
        </Button>
        <Button
          variant="outline"
          disabled={busy}
          onClick={() =>
            void action(async () => {
              const a = await invoke<ReplayArtifact>('replay_save_dataset', {
                dataset: { ...dataset, cases: cases() },
              } satisfies ReplaySaveDatasetParams)
              setDataset(a.value as ReplayDataset)
              await refresh()
            })
          }
        >
          Save dataset locally
        </Button>
        <Button
          variant="outline"
          onClick={() =>
            void action(async () =>
              download({ ...dataset, cases: cases() }, 'replay-dataset.json'),
            )
          }
        >
          Export dataset
        </Button>
        <label className="cursor-pointer border rounded p-2">
          Import JSON
          <input
            type="file"
            accept="application/json,.json"
            className="sr-only"
            onChange={(e) => {
              const file = e.target.files?.[0]
              e.target.value = ''
              if (file)
                void action(async () => {
                  if (file.size > 32 * 1024 * 1024)
                    throw Error('Import exceeds 32 MiB')
                  const json = await file.text()
                  const parsed = JSON.parse(json)
                  if (Array.isArray((parsed.value ?? parsed).results)) {
                    openRun(
                      await invoke<ReplayRunSummary>('replay_import_run', {
                        json,
                      } satisfies ReplayImportRunParams),
                    )
                    await refresh()
                    return
                  }
                  const d = await invoke<ReplayDataset>('replay_import', {
                    json,
                  } satisfies ReplayImportParams)
                  setDataset(d)
                  setCasesText(pretty(d.cases))
                })
            }}
          />
        </label>
      </div>
      {estimates && (
        <details>
          <summary>Last requested cost preview (refresh after editing)</summary>
          <pre className="text-xs whitespace-pre-wrap">{pretty(estimates)}</pre>
        </details>
      )}
      <div className="flex gap-2 flex-wrap">
        <Button variant="outline" onClick={() => void action(refresh)}>
          Refresh saved data and runs
        </Button>
        {runs
          .filter((r) => r.client_id === clientId)
          .map((r) => (
            <Button key={r.id} variant="ghost" onClick={() => openRun(r)}>
              {r.name} ({r.status})
            </Button>
          ))}
      </div>
      {run && (
        <div className="space-y-2 border rounded p-3">
          <p>
            {run.name}: {run.status}. Reported spend:{' '}
            {costs.reported_usd === null
              ? 'unknown'
              : `$${costs.reported_usd.toFixed(6)}`}
            ; estimated spend:{' '}
            {costs.estimated_usd === null
              ? 'unknown'
              : `$${costs.estimated_usd.toFixed(6)}`}
            . Classifier reported / estimated:{' '}
            {costs.classifier_reported_usd ?? 'unknown'} /{' '}
            {costs.classifier_estimated_usd ?? 'unknown'} USD. These are partial
            sums when spend is unknown. Unknown calls: {unknown}. Client:{' '}
            {run.client_id}. {run.judge_of && 'Separate judge run.'}
          </p>
          {run.stop_reason && <p>{run.stop_reason}</p>}
          <div className="flex gap-2">
            <Button
              disabled={run.status !== 'running'}
              variant="outline"
              onClick={() =>
                void action(async () => {
                  await invoke('replay_cancel', {
                    id: run.id,
                  } satisfies ReplayIdParams)
                })
              }
            >
              Cancel remaining calls
            </Button>
            <Button
              variant="outline"
              disabled={!runs.some((r) => r.id === run.id)}
              onClick={() =>
                void action(async () => {
                  await invoke<ReplayArtifact>('replay_save_run', {
                    id: run.id,
                  } satisfies ReplayIdParams)
                  await refresh()
                })
              }
            >
              Save run snapshot
            </Button>
            <Button
              variant="outline"
              onClick={() =>
                void action(async () => {
                  const full = await invoke<ReplayRun>('replay_get_run', {
                    id: run.id,
                  } satisfies ReplayIdParams)
                  download(full, 'replay-run.json')
                })
              }
            >
              Export run
            </Button>
          </div>
          <div className="overflow-x-auto">
            <table className="w-full text-xs">
              <thead>
                <tr>
                  {[
                    'Judge',
                    'Case / target',
                    'Status / checks',
                    'Latency / tokens',
                    'Reported / estimated USD',
                    'Output',
                  ].map((h) => (
                    <th key={h} className="p-2 text-left">
                      {h}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {run.results.map((r) => (
                  <tr key={r.index} className="border-t">
                    <td>
                      <input
                        type="checkbox"
                        aria-label={`Judge result ${r.index}`}
                        disabled={r.status !== 'success'}
                        checked={judgeIndices.includes(r.index)}
                        onChange={(e) =>
                          setJudgeIndices((ids) =>
                            e.target.checked
                              ? [...ids, r.index]
                              : ids.filter((i) => i !== r.index),
                          )
                        }
                      />
                    </td>
                    <td className="p-2">
                      {r.case_name}
                      <br />
                      {r.target}
                    </td>
                    <td>
                      {r.status}
                      <br />
                      {r.checks.filter((c) => c.passed).length}/
                      {r.checks.length} checks
                      {r.error && <p className="text-destructive">{r.error}</p>}
                      {r.judge && (
                        <p>
                          Judge score {r.judge.score}: {r.judge.reasoning}
                        </p>
                      )}
                    </td>
                    <td>
                      {r.latency_ms ?? '?'} ms
                      <br />
                      {r.input_tokens ?? '?'} in / {r.output_tokens ?? '?'} out
                    </td>
                    <td>
                      {r.cost.reported_usd ?? '?'} /{' '}
                      {r.cost.estimated_usd ?? '?'}
                      <br />
                      Classifier: {r.cost.classifier_reported_usd ?? '?'} /{' '}
                      {r.cost.classifier_estimated_usd ?? '?'}
                      {(r.cost.unknown_calls > 0 ||
                        r.cost.classifier_cost_unknown) && (
                        <p>Additional spend unknown</p>
                      )}
                    </td>
                    <td>
                      <Button
                        variant="ghost"
                        onClick={() =>
                          void action(async () => {
                            const value = await invoke<ReplayResult>(
                              'replay_result',
                              {
                                id: run.id,
                                index: r.index,
                              } satisfies ReplayResultParams,
                            )
                            setDetails((d) => ({ ...d, [r.index]: value }))
                          })
                        }
                      >
                        Inspect
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="grid md:grid-cols-2 gap-2">
            {Object.entries(details).map(([index, result]) => (
              <details key={index} open className="border rounded p-2">
                <summary>
                  {result.case_name} / {result.target}
                </summary>
                <pre className="text-xs whitespace-pre-wrap break-all max-h-96 overflow-auto">
                  {pretty(result)}
                </pre>
              </details>
            ))}
          </div>
          <label className="block">
            Judge model
            <select
              className="block border bg-background p-2"
              value={judgeModel}
              onChange={(e) => setJudgeModel(e.target.value)}
            >
              <option value="">Select model…</option>
              {models.map((m) => (
                <option key={m.id}>{m.id}</option>
              ))}
            </select>
          </label>
          <label className="block">
            Judge rubric
            <textarea
              className="w-full border rounded p-2 bg-background"
              value={rubric}
              onChange={(e) => setRubric(e.target.value)}
            />
          </label>
          <Button
            disabled={
              busy ||
              run.status === 'running' ||
              run.client_id !== clientId ||
              !judgeModel ||
              !judgeIndices.length
            }
            onClick={() =>
              void action(async () => {
                const id = await invoke<string>('replay_judge', {
                  id: run.id,
                  model: judgeModel,
                  rubric,
                  indices: judgeIndices,
                } satisfies ReplayJudgeParams)
                openRun(
                  await invoke<ReplayRunSummary>('replay_status', {
                    id,
                  } satisfies ReplayIdParams),
                )
                await refresh()
              })
            }
          >
            Run separate paid judge ({judgeIndices.length} results)
          </Button>
          <p className="text-xs">
            Judge scores are model assessments. Judge calls send the selected
            input and output to the chosen provider and have separate spend.
          </p>
        </div>
      )}
      <div className="space-y-2">
        <p>Saved local artifacts ({saved.items.length})</p>
        {saved.issues.map((i) => (
          <p key={i.id} className="text-destructive">
            {i.id}: {i.error}
          </p>
        ))}
        {saved.items.map((item) => (
          <div key={item.id} className="flex gap-2 items-center">
            <span>
              {item.name} · {item.kind} · revision {item.revision}
            </span>
            <Button
              variant="ghost"
              onClick={() => void action(() => read(item))}
            >
              Open
            </Button>
            <Button
              variant="ghost"
              onClick={() =>
                void action(async () => {
                  const a = await invoke<ReplayArtifact>('replay_read', {
                    kind: item.kind,
                    id: item.id,
                  } satisfies ReplayReadParams)
                  download(a, `replay-${item.kind}.json`)
                })
              }
            >
              Export
            </Button>
            <Button variant="ghost" onClick={() => setToDelete(item)}>
              Delete
            </Button>
          </div>
        ))}
      </div>
      <AlertDialog
        open={!!toDelete}
        onOpenChange={(open) => {
          if (!open) setToDelete(null)
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete saved replay data?</AlertDialogTitle>
            <AlertDialogDescription>
              Delete {toDelete?.name} from local storage. Any related live
              comparison will be cancelled. Saved runs are separate artifacts;
              delete them separately to remove their retained inputs and
              results. Export a copy first if needed.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => {
                if (toDelete)
                  void action(async () => {
                    await invoke('replay_delete', {
                      id: toDelete.id,
                      kind: toDelete.kind,
                      revision: toDelete.revision,
                    } satisfies ReplayDeleteParams)
                    if (run?.id === toDelete.id) setRun(null)
                    await refresh()
                  })
              }}
            >
              Delete
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
